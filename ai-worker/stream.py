"""Robust camera reader.

One thread per camera reads frames as fast as the camera sends them and keeps
only the newest one, so AI processing never works on a backlog of old frames
(no growing delay), however long one inference takes.

Protection against the usual ways a CCTV stream fails:
  * RTSP over TCP with open/read timeouts - a dead camera cannot block forever
  * reconnect with back-off when the stream errors or ends
  * frozen-picture detection (decoder keeps returning the same image)
"""

from __future__ import annotations

import logging
import os
import threading
import time
from typing import Optional

# Must be set before OpenCV opens a stream. TCP avoids smeared frames from lost UDP
# packets; timeout (microseconds) makes a dead camera return instead of hanging.
os.environ.setdefault("OPENCV_FFMPEG_CAPTURE_OPTIONS", "rtsp_transport;tcp|timeout;5000000|stimeout;5000000")

import cv2  # noqa: E402

from tracker import FreezeDetector  # noqa: E402

log = logging.getLogger("ai-worker.stream")

OPEN_TIMEOUT_MS = int(os.environ.get("STREAM_OPEN_TIMEOUT_MS", "8000"))
READ_TIMEOUT_MS = int(os.environ.get("STREAM_READ_TIMEOUT_MS", "8000"))
FREEZE_SEC = float(os.environ.get("STREAM_FREEZE_SEC", "15"))


def thumbnail(frame) -> bytes:
    """Tiny grey thumbnail used for freeze and motion checks."""
    g = cv2.cvtColor(frame, cv2.COLOR_BGR2GRAY)
    return cv2.resize(g, (32, 18), interpolation=cv2.INTER_AREA).tobytes()


class FrameGrabber(threading.Thread):
    def __init__(self, source: str, name: str, stop: threading.Event, pace_fps: Optional[float] = None):
        super().__init__(daemon=True, name=f"grab-{name}")
        self.source, self.cam_name, self.stop_event = source, name, stop
        # local files are replayed at their own speed so they behave like a live camera
        self.is_file = "://" not in source
        self.pace_fps = pace_fps
        self._lock = threading.Lock()
        self._frame = None
        self._seq = 0
        self._t = 0.0
        self.connected = False
        self.error: Optional[str] = "connecting"
        self.reconnects = 0
        self.input_fps = 0.0

    def latest(self):
        """(frame, sequence number, capture time) of the newest frame."""
        with self._lock:
            return self._frame, self._seq, self._t

    def _open(self):
        params = [cv2.CAP_PROP_OPEN_TIMEOUT_MSEC, OPEN_TIMEOUT_MS, cv2.CAP_PROP_READ_TIMEOUT_MSEC, READ_TIMEOUT_MS]
        try:
            cap = cv2.VideoCapture(self.source, cv2.CAP_FFMPEG, params)
        except Exception:  # older OpenCV without the params overload
            cap = cv2.VideoCapture(self.source, cv2.CAP_FFMPEG)
        try:
            cap.set(cv2.CAP_PROP_BUFFERSIZE, 1)
        except Exception:
            pass
        return cap

    def run(self):
        backoff = 1.0
        while not self.stop_event.is_set():
            cap = self._open()
            if not cap.isOpened():
                self.connected, self.error = False, "cannot open stream"
                log.warning("camera %s: cannot open stream, retry in %.0fs", self.cam_name, backoff)
                self.stop_event.wait(backoff)
                backoff = min(backoff * 2, 30)
                continue
            src_fps = cap.get(cv2.CAP_PROP_FPS) or 0
            pace = 1.0 / (self.pace_fps or src_fps or 15) if self.is_file else 0
            freeze = FreezeDetector(FREEZE_SEC)
            frames, t_rate, last_sig = 0, time.time(), 0.0
            reason = "stream ended"
            while not self.stop_event.is_set():
                if pace:
                    self.stop_event.wait(pace)
                ok, frame = cap.read()
                now = time.time()
                if not ok or frame is None:
                    break
                with self._lock:
                    self._frame, self._seq, self._t = frame, self._seq + 1, now
                if not self.connected:
                    log.info("camera %s: connected", self.cam_name)
                self.connected, self.error = True, None
                backoff = 1.0
                frames += 1
                if now - t_rate >= 5:
                    self.input_fps = frames / (now - t_rate)
                    frames, t_rate = 0, now
                if now - last_sig >= 0.5:
                    last_sig = now
                    if freeze.feed(thumbnail(frame), now):
                        reason = "frozen picture"
                        break
            cap.release()
            if self.stop_event.is_set():
                break
            self.connected, self.error = False, reason
            self.reconnects += 1
            log.warning("camera %s: %s, reconnecting", self.cam_name, reason)
            self.stop_event.wait(min(backoff, 2))
