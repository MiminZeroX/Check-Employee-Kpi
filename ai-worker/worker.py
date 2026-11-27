#!/usr/bin/env python3
"""Smart Office AI worker.

Reads camera streams (normally the go2rtc RTSP restream, so each camera is only
opened once), classifies each workstation zone, and sends activity segments +
live heartbeats to the API server. No images or video are stored or sent.

Environment:
  SERVER_URL      http://server:8080
  DEVICE_KEY      API key of an "ai_worker" device (Device Integration page)
  MODEL_PATH      object detector .onnx (YOLOX or YOLOv8 format, COCO classes)
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
            try:
                self.client.call("POST", "/api/ingest/heartbeat", {"cameras": cams, "desks": desks})
            except Exception as e:
                log.warning("heartbeat failed: %s", e)


class CameraWorker(threading.Thread):
    """Reads one stream at the configured AI frame rate and classifies its workstations."""

    def __init__(self, hub: Hub, detector, cam: dict):
        super().__init__(daemon=True, name=f"cam-{cam['id']}")
        self.hub, self.detector = hub, detector
        self.update_config(cam)
        self.stop = threading.Event()

    def update_config(self, cam: dict):
        self.cam = cam

    def set_status(self, online: bool, fps: float = 0.0, error: Optional[str] = None):
        with self.hub.lock:
            self.hub.camera_status[self.cam["id"]] = {"online": online, "fps": round(fps, 2), "error": error}

    def run(self):
        import cv2
        from detector import classify

        backoff = 2
        while not self.stop.is_set() and not self.hub.stop.is_set():
            cap = cv2.VideoCapture(self.cam["source"], cv2.CAP_FFMPEG)
            if not cap.isOpened():
                self.set_status(False, error="cannot open stream")
                log.warning("camera %s: cannot open stream, retry in %ss", self.cam["name"], backoff)
                self.stop.wait(backoff)
                backoff = min(backoff * 2, 60)
                continue
            backoff = 2
            log.info("camera %s: connected", self.cam["name"])
            last_proc, frames, t_fps = 0.0, 0, time.time()
            # a local video file (for testing / tuning with recordings) is read at the AI frame rate,
            # a live stream is drained continuously so the newest frame is always processed
            is_file = "://" not in self.cam["source"]
            while not self.stop.is_set() and not self.hub.stop.is_set():
                interval = 1.0 / max(0.2, float(self.cam.get("fps") or 2))
                if is_file:
                    self.stop.wait(max(0.0, interval - (time.time() - last_proc)))
                ok = cap.grab()
                if not ok:
                    break
                now = time.time()
                if not is_file and now - last_proc < interval:
                    continue
                ok, frame = cap.retrieve()
                if not ok:
                    break
                last_proc = now
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
                    roi_px = [[x * w, y * h] for x, y in ws["roi"]]
                    state, conf, yaw = classify(self.detector, frame, dets, roi_px,
                                                float(ws["screen_yaw_deg"]), float(ws["yaw_tolerance_deg"]), conf_thr)
                    self.hub.observe(ws["id"], state, conf, now, yaw)
                frames += 1
                if now - t_fps >= 10:
                    self.set_status(True, frames / (now - t_fps))
                    frames, t_fps = 0, now
                elif frames == 1:
                    self.set_status(True, 0)
            cap.release()
            self.set_status(False, error="stream ended")
            self.hub.flush_camera([w["id"] for w in self.cam["workstations"]], time.time())
            log.warning("camera %s: stream lost, reconnecting", self.cam["name"])
            self.stop.wait(2)


class SimulatedCamera(threading.Thread):
    """Generates believable desk activity without cameras or models (demo / load tests)."""

    TRANSITIONS = {
        "focused": [("focused", 0.985), ("present", 0.004), ("looking_away", 0.004), ("phone", 0.004), ("away", 0.003)],
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
        model = os.environ.get("MODEL_PATH", "models/object_detection_yolox_2022nov.onnx")
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
