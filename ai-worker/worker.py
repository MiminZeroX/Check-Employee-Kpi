#!/usr/bin/env python3
"""Smart Office AI worker.

Reads camera streams (normally the go2rtc RTSP restream, so each camera is only
opened once), classifies each workstation zone, and sends activity segments +
live heartbeats to the API server. No images or video are stored or sent.

Environment:
  SERVER_URL      http://server:8080
  DEVICE_KEY      API key of an "ai_worker" device (Device Integration page)
  MODEL_PATH      object detector .onnx (default models/yolox_nano.onnx; YOLOX or YOLOv8 format)
  LOW_MEMORY=1    ONNX Runtime without memory arena (default on)
  AI_BOOST_FPS    analysis rate while there is motion in a desk zone (default 8)
  FACE_MODEL      YuNet face detector .onnx (optional, enables looking-at-screen)
  SIMULATE=1      no cameras/models: generate plausible states for every desk (demo/testing)
  ORT_THREADS     ONNX Runtime threads (default 2)
"""

from __future__ import annotations

import json
import logging
import os
import random
import signal
import threading
import time
import urllib.error
import urllib.request
from collections import deque
from typing import Optional

from tracker import DeskTracker

log = logging.getLogger("ai-worker")

SEGMENT_FLUSH_SEC = 300      # persist long-running segments every 5 minutes
HEARTBEAT_SEC = 5
CONFIG_REFRESH_SEC = 60


class Client:
    def __init__(self, base: str, key: str):
        self.base = base.rstrip("/")
        self.key = key

    def call(self, method: str, path: str, body=None):
        data = json.dumps(body).encode() if body is not None else None
        req = urllib.request.Request(self.base + path, data=data, method=method, headers={
            "X-Device-Key": self.key, "Content-Type": "application/json", "User-Agent": "smartoffice-ai-worker/0.1"})
        with urllib.request.urlopen(req, timeout=15) as r:
            txt = r.read().decode()
            return json.loads(txt) if txt else None


class Hub:
    """Shared state: trackers, outgoing segment queue, camera status."""

    def __init__(self, client: Client):
        self.client = client
        self.lock = threading.Lock()
        self.trackers: dict[int, DeskTracker] = {}
        self.zones: dict = {}
        self.queue: deque = deque(maxlen=50_000)
        self.camera_status: dict[int, dict] = {}
        self.stop = threading.Event()
        self.detection = {"min_state_sec": 5, "away_grace_sec": 20, "confidence": 0.45}

    def tracker(self, ws_id: int) -> DeskTracker:
        with self.lock:
            t = self.trackers.get(ws_id)
            if t is None:
                t = DeskTracker(ws_id, self.detection["min_state_sec"], self.detection["away_grace_sec"])
                self.trackers[ws_id] = t
            t.min_state_sec = float(self.detection["min_state_sec"])
            t.away_grace_sec = float(self.detection["away_grace_sec"])
            return t

    def observe(self, ws_id: int, state: str, conf: float, now: float, yaw: Optional[float] = None):
        seg = self.tracker(ws_id).update(state, conf, now, yaw)
        if seg:
            self.queue.append(seg)

    def zone_observe(self, zone_id: int, count: int):
        from tracker import ZoneCounter
        with self.lock:
            self.zones.setdefault(zone_id, ZoneCounter(zone_id)).add(count)

    def flush_camera(self, ws_ids, now: float):
        """Camera went offline: close running segments instead of inventing states."""
        with self.lock:
            for w in ws_ids:
                t = self.trackers.get(w)
                if t:
                    seg = t.flush(now)
                    if seg:
                        self.queue.append(seg)
                    t.reset()

    # ---------------------------------------------------------- background loops

    def sender_loop(self):
        last_flush = time.time()
        while not self.stop.wait(5):
            now = time.time()
            if now - last_flush >= SEGMENT_FLUSH_SEC:
                with self.lock:
                    for t in self.trackers.values():
                        seg = t.flush(now)
                        if seg:
                            self.queue.append(seg)
                last_flush = now
            self.send_pending()

    def send_pending(self):
        while self.queue:
            batch = [self.queue.popleft() for _ in range(min(1000, len(self.queue)))]
            try:
                self.client.call("POST", "/api/ingest/events", {"events": batch})
            except Exception as e:  # keep data and retry later
                self.queue.extendleft(reversed(batch))
                log.warning("send events failed (%s), %d segments buffered", e, len(self.queue))
                return

    def heartbeat_loop(self):
        while not self.stop.wait(HEARTBEAT_SEC):
            with self.lock:
                desks = [d for d in (t.live() for t in self.trackers.values()) if d]
                cams = [{"camera_id": k, **v} for k, v in self.camera_status.items()]
                zones = [z for z in (c.take() for c in self.zones.values()) if z]
            try:
                self.client.call("POST", "/api/ingest/heartbeat", {"cameras": cams, "desks": desks, "zones": zones})
            except Exception as e:
                log.warning("heartbeat failed: %s", e)


STALE_SEC = float(os.environ.get("STALE_SEC", "5"))            # no new frame for this long = camera offline
RECONNECT_GRACE_SEC = float(os.environ.get("RECONNECT_GRACE_SEC", "30"))  # short drop-outs keep the running segment
BOOST_FPS = float(os.environ.get("AI_BOOST_FPS", "8"))           # analysis rate while people move
MOTION_THRESHOLD = float(os.environ.get("MOTION_THRESHOLD", "0.015"))   # share of zone pixels that changed
# strong movement inside a desk zone while the detector sees nobody = a person moving fast (blurred),
# because an empty desk does not move
MOVING_PERSON_THRESHOLD = float(os.environ.get("MOVING_PERSON_THRESHOLD", "0.04"))
RECENTLY_SEEN_SEC = float(os.environ.get("RECENTLY_SEEN_SEC", "15"))


class CameraWorker(threading.Thread):
    """Analyses one camera.

    A FrameGrabber thread always holds the newest frame, so a slow inference never
    builds up delay. Analysis runs at the camera's configured fps while the scene is
    still and is boosted (AI_BOOST_FPS) while there is motion in a desk zone or a
    desk state is still being confirmed, so fast movements are caught without
    paying for a high frame rate all day.
    """

    def __init__(self, hub: Hub, detector, cam: dict):
        super().__init__(daemon=True, name=f"cam-{cam['id']}")
        self.hub, self.detector = hub, detector
        self.stop = threading.Event()
        self.prev_box: dict[int, tuple] = {}
        self.smoothers: dict[int, object] = {}
        self.yaw_filters: dict[int, object] = {}
        self.last_seen: dict[int, float] = {}
        self.update_config(cam)

    def update_config(self, cam: dict):
        self.cam = cam
        self._masks = None  # recomputed for the new zones

    def set_status(self, online: bool, **extra):
        with self.hub.lock:
            self.hub.camera_status[self.cam["id"]] = {"online": online, **extra}

    def _zone_masks(self, shape):
        """Per-desk zone masks and their union, at motion-check resolution (160x90)."""
        import cv2
        import numpy as np
        h, w = shape
        if self._masks is None or self._masks[1].shape != (h, w):
            per = {}
            for ws in self.cam["workstations"]:
                m = np.zeros((h, w), dtype=np.uint8)
                pts = np.array([[x * w, y * h] for x, y in ws["roi"]], dtype=np.int32)
                cv2.fillPoly(m, [pts], 1)
                per[ws["id"]] = m
            union = np.zeros((h, w), dtype=np.uint8)
            for m in per.values():
                union |= m
            self._masks = (per, union if union.any() else np.ones((h, w), dtype=np.uint8))
        return self._masks

    def run(self):
        import cv2
        from detector import classify, count_in_zone
        from stream import FrameGrabber
        from tracker import AdaptiveRate, StateSmoother, YawFilter

        grabber = FrameGrabber(self.cam["source"], self.cam["name"], self.stop)
        grabber.start()
        base_fps = max(0.2, float(self.cam.get("fps") or 2))
        rate = AdaptiveRate(base_fps, max(base_fps, BOOST_FPS))
        last_seq, last_proc, prev_small = -1, 0.0, None
        processed, t_stat, latency_ms, offline_since, flushed = 0, time.time(), 0.0, None, False
        while not self.stop.is_set() and not self.hub.stop.is_set():
            self.stop.wait(min(0.05, 0.5 / rate.boost_fps))
            frame, seq, ft = grabber.latest()
            now = time.time()

            # ---- health: stale / disconnected camera
            if frame is None or now - ft > STALE_SEC:
                if offline_since is None:
                    offline_since = now
                    self.set_status(False, error=grabber.error or "no frames", reconnects=grabber.reconnects)
                # a short drop-out keeps the running segment; a long one closes it at the last real frame
                if frame is not None and not flushed and now - offline_since > RECONNECT_GRACE_SEC:
                    self.hub.flush_camera([w["id"] for w in self.cam["workstations"]], ft)
                    self.prev_box, self.smoothers, self.yaw_filters = {}, {}, {}
                    flushed = True
                continue
            offline_since, flushed = None, False
            if seq == last_seq:
                continue

            # ---- adaptive rate: cheap motion check inside the desk zones
            rate.base_fps = max(0.2, float(self.cam.get("fps") or 2))
            small = cv2.GaussianBlur(cv2.resize(cv2.cvtColor(frame, cv2.COLOR_BGR2GRAY), (160, 90)), (5, 5), 0)
            per_mask, mask = self._zone_masks(small.shape)
            motion, changed = False, None
            if prev_small is not None:
                changed = cv2.absdiff(small, prev_small) > 25
                motion = (changed & (mask > 0)).sum() / max(1, mask.sum()) >= MOTION_THRESHOLD
            with self.hub.lock:
                unsettled = any(self.hub.trackers[w["id"]].unsettled for w in self.cam["workstations"]
                                if w["id"] in self.hub.trackers)
            unsettled = unsettled or any(not sm.unanimous for sm in self.smoothers.values())
            if not rate.due(now, last_proc, motion, unsettled):
                continue
            last_seq, last_proc, prev_small = seq, now, small

            # ---- inference on the newest frame
            h, w = frame.shape[:2]
            target_w = int(self.cam.get("width") or 960)
            if w > target_w:
                frame = cv2.resize(frame, (target_w, int(h * target_w / w)))
                h, w = frame.shape[:2]
            conf_thr = float(self.hub.detection["confidence"])
            try:
                dets = self.detector.detect(frame, conf_thr)
            except Exception as e:
                log.exception("inference failed: %s", e)
                continue
            for ws in self.cam["workstations"]:
                wid = ws["id"]
                roi_px = [[x * w, y * h] for x, y in ws["roi"]]
                raw, conf, yaw, box = classify(self.detector, frame, dets, roi_px, float(ws["screen_yaw_deg"]),
                                               float(ws["yaw_tolerance_deg"]), conf_thr, self.prev_box.get(wid))
                if box is not None:
                    self.prev_box[wid] = box
                    self.last_seen[wid] = ft
                else:
                    self.prev_box.pop(wid, None)
                    # nobody detected, but the desk zone is moving a lot: a person moving fast / blurred
                    m = per_mask.get(wid)
                    if raw == "away" and changed is not None and m is not None and \
                            (changed & (m > 0)).sum() / max(1, m.sum()) >= MOVING_PERSON_THRESHOLD:
                        if ft - self.last_seen.get(wid, -1e9) <= RECENTLY_SEEN_SEC:
                            raw, conf = "present", 0.5
                            self.last_seen[wid] = ft
                        else:
                            raw, conf = "unknown", 0.4
                sm = self.smoothers.setdefault(wid, StateSmoother())
                yf = self.yaw_filters.setdefault(wid, YawFilter())
                state, sconf = sm.add(raw, conf, ft)
                # time stamps come from frame capture, not from when the AI finished
                self.hub.observe(wid, state, sconf, ft, yf.add(yaw, ft))
            for z in self.cam.get("zones") or []:
                roi_px = [[x * w, y * h] for x, y in z["roi"]]
                self.hub.zone_observe(z["id"], count_in_zone(dets, roi_px, conf_thr))
            latency_ms = latency_ms * 0.8 + (time.time() - ft) * 1000 * 0.2
            processed += 1
            with self.hub.lock:
                was_online = self.hub.camera_status.get(self.cam["id"], {}).get("online", False)
            if now - t_stat >= 5 or not was_online:
                self.set_status(True, fps=round(processed / max(now - t_stat, 1e-3), 2),
                                input_fps=round(grabber.input_fps, 1), latency_ms=round(latency_ms),
                                reconnects=grabber.reconnects, boosted=rate.current_fps(now) > rate.base_fps, error=None)
                if now - t_stat >= 5:
                    processed, t_stat = 0, now
        self.stop.set()


class SimulatedCamera(threading.Thread):
    """Generates believable desk activity without cameras or models (demo / load tests)."""

    TRANSITIONS = {
        "focused": [("focused", 0.983), ("present", 0.004), ("looking_away", 0.004), ("phone", 0.004), ("meeting", 0.002), ("away", 0.003)],
        "meeting": [("meeting", 0.97), ("focused", 0.03)],
        "present": [("present", 0.95), ("focused", 0.04), ("away", 0.01)],
        "looking_away": [("looking_away", 0.9), ("focused", 0.1)],
        "phone": [("phone", 0.96), ("focused", 0.04)],
        "away": [("away", 0.99), ("focused", 0.01)],
        "unknown": [("focused", 1.0)],
    }

    def __init__(self, hub: Hub, cam: dict, seed: int):
        super().__init__(daemon=True, name=f"sim-{cam['id']}")
        self.hub = hub
        self.update_config(cam)
        self.stop = threading.Event()
        self.rng = random.Random(seed)
        self.raw: dict[int, str] = {}

    def update_config(self, cam: dict):
        self.cam = cam

    def run(self):
        with self.hub.lock:
            self.hub.camera_status[self.cam["id"]] = {"online": True, "fps": 1.0, "error": None}
        while not self.stop.wait(1.0) and not self.hub.stop.is_set():
            now = time.time()
            for ws in self.cam["workstations"]:
                cur = self.raw.get(ws["id"]) or self.rng.choice(["focused", "focused", "present", "away"])
                r, acc = self.rng.random(), 0.0
                for nxt, p in self.TRANSITIONS[cur]:
                    acc += p
                    if r <= acc:
                        cur = nxt
                        break
                self.raw[ws["id"]] = cur
                yaw = None
                if cur == "focused":
                    yaw = float(ws["screen_yaw_deg"]) + self.rng.uniform(-10, 10)
                elif cur == "looking_away":
                    yaw = float(ws["screen_yaw_deg"]) + self.rng.choice([-1, 1]) * self.rng.uniform(40, 70)
                self.hub.observe(ws["id"], cur, self.rng.uniform(0.7, 0.95), now, yaw)
            for z in self.cam.get("zones") or []:
                base = {"meeting_room": 3, "entrance": 1, "area": 6}.get(z["kind"], 2)
                self.hub.zone_observe(z["id"], max(0, int(self.rng.gauss(base, 1.5))))


def main():
    logging.basicConfig(level=os.environ.get("LOG_LEVEL", "INFO"), format="%(asctime)s %(levelname)s %(name)s: %(message)s")
    server = os.environ.get("SERVER_URL", "http://localhost:8080")
    key = os.environ.get("DEVICE_KEY", "")
    if not key:
        raise SystemExit("DEVICE_KEY is required (create an 'AI Worker' device in Device Integration)")
    simulate = os.environ.get("SIMULATE", "0") == "1"
    hub = Hub(Client(server, key))

    detector = None
    if not simulate:
        from detector import Detector
        model = os.environ.get("MODEL_PATH", "models/yolox_nano.onnx")
        face = os.environ.get("FACE_MODEL", "models/face_detection_yunet_2023mar.onnx")
        detector = Detector(model, face if os.path.exists(face) else None, int(os.environ.get("ORT_THREADS", "2")))
        log.info("model loaded: %s (%s format, %dpx) face=%s", model, detector.fmt, detector.size, bool(detector.face))

    workers: dict[int, threading.Thread] = {}

    def shutdown(*_):
        log.info("shutting down, flushing segments")
        hub.stop.set()

    signal.signal(signal.SIGTERM, shutdown)
    signal.signal(signal.SIGINT, shutdown)

    threading.Thread(target=hub.sender_loop, daemon=True).start()
    threading.Thread(target=hub.heartbeat_loop, daemon=True).start()

    while not hub.stop.is_set():
        try:
            cfg = hub.client.call("GET", "/api/ingest/config")
            hub.detection.update(cfg.get("detection") or {})
            wanted = {c["id"]: c for c in cfg["cameras"]}
            for cid in list(workers):
                if cid not in wanted:
                    workers.pop(cid).stop.set()
                    with hub.lock:
                        hub.camera_status.pop(cid, None)
            for cid, cam in wanted.items():
                w = workers.get(cid)
                if w and w.cam.get("source") != cam.get("source"):
                    w.stop.set()
                    w = None
                if w:
                    w.update_config(cam)
                    continue
                w = SimulatedCamera(hub, cam, seed=cid) if simulate else CameraWorker(hub, detector, cam)
                workers[cid] = w
                w.start()
            log.info("config: %d cameras, %d workstations", len(wanted), sum(len(c["workstations"]) for c in wanted.values()))
        except urllib.error.HTTPError as e:
            log.error("config request failed: HTTP %s (check DEVICE_KEY)", e.code)
        except Exception as e:
            log.error("config request failed: %s", e)
        hub.stop.wait(CONFIG_REFRESH_SEC)

    now = time.time()
    with hub.lock:
        for t in hub.trackers.values():
            seg = t.flush(now)
            if seg:
                hub.queue.append(seg)
    hub.send_pending()


if __name__ == "__main__":
    main()
