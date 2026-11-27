"""Person / phone detection (ONNX Runtime) and head-direction estimation (OpenCV YuNet).

Supported object-detection models (COCO classes):
  * YOLOX (Apache-2.0)  - yolox_nano.onnx (416px, ~100 MB RAM, default) or yolox_tiny.onnx
                          (more accurate for small objects such as phones, ~125 MB RAM)
  * YOLOv8 / YOLO11 export format (1, 84, N). Note: Ultralytics weights are AGPL-3.0,
    check the licence before using them in a commercial deployment.
Face detector: YuNet (MIT) from opencv_zoo - gives 5 landmarks used to estimate yaw.

Only metadata leaves this module: no frames are stored or transmitted.
"""

from __future__ import annotations

import math
import threading
from dataclasses import dataclass
from typing import Optional

import os

import cv2
import numpy as np

# one OpenCV thread is enough at 1-2 fps and saves memory per thread
cv2.setNumThreads(int(os.environ.get("OPENCV_THREADS", "1")))

from tracker import box_iou, point_in_polygon

PERSON, CELL_PHONE = 0, 67


@dataclass
class Det:
    cls: int
    conf: float
    x1: float
    y1: float
    x2: float
    y2: float

    @property
    def cx(self) -> float:
        return (self.x1 + self.x2) / 2

    @property
    def cy(self) -> float:
        return (self.y1 + self.y2) / 2

    @property
    def area(self) -> float:
        return max(0.0, self.x2 - self.x1) * max(0.0, self.y2 - self.y1)


def letterbox(img: np.ndarray, size: int) -> tuple[np.ndarray, float]:
    h, w = img.shape[:2]
    r = min(size / h, size / w)
    nh, nw = int(round(h * r)), int(round(w * r))
    out = np.full((size, size, 3), 114, dtype=np.uint8)
    out[:nh, :nw] = cv2.resize(img, (nw, nh), interpolation=cv2.INTER_LINEAR)
    return out, r


class Detector:
    PREFERRED = ["OpenVINOExecutionProvider", "CUDAExecutionProvider", "CPUExecutionProvider"]

    def __init__(self, model_path: str, face_model_path: Optional[str] = None, threads: int = 2):
        import onnxruntime as ort

        so = ort.SessionOptions()
        so.intra_op_num_threads = threads
        so.inter_op_num_threads = 1
        if os.environ.get("LOW_MEMORY", "1") == "1":
            # no pre-allocated memory arena / patterns: ~50 MB less RAM, a few ms slower per frame
            so.enable_cpu_mem_arena = False
            so.enable_mem_pattern = False
        avail = ort.get_available_providers()
        providers = [p for p in self.PREFERRED if p in avail] or avail
        self.sess = ort.InferenceSession(model_path, so, providers=providers)
        inp = self.sess.get_inputs()[0]
        self.input_name = inp.name
        self.size = int(inp.shape[2]) if isinstance(inp.shape[2], int) else 640
        out_shape = self.sess.get_outputs()[0].shape
        # (1, 84, N) => YOLOv8 style; (1, N, 85) => YOLOX style
        self.fmt = "yolov8" if len(out_shape) == 3 and out_shape[1] == 84 else "yolox"
        self._grids = None
        self._lock = threading.Lock()
        self.face = None
        if face_model_path:
            self.face = cv2.FaceDetectorYN.create(face_model_path, "", (320, 320), 0.6, 0.3, 50)
            self._face_lock = threading.Lock()

    # ------------------------------------------------------------ objects

    def _yolox_grids(self, n: int):
        if self._grids is None or self._grids[0].shape[1] != n:
            grids, strides = [], []
            for s in (8, 16, 32):
                hs = ws = self.size // s
                xv, yv = np.meshgrid(np.arange(ws), np.arange(hs))
                g = np.stack((xv, yv), 2).reshape(1, -1, 2)
                grids.append(g)
                strides.append(np.full((1, g.shape[1], 1), s))
            self._grids = (np.concatenate(grids, 1), np.concatenate(strides, 1))
        return self._grids

    def detect(self, frame: np.ndarray, conf_thr: float = 0.3) -> list[Det]:
        img, r = letterbox(frame, self.size)
        if self.fmt == "yolov8":
            blob = cv2.cvtColor(img, cv2.COLOR_BGR2RGB).transpose(2, 0, 1)[None].astype(np.float32) / 255.0
        else:
            blob = img.transpose(2, 0, 1)[None].astype(np.float32)
        with self._lock:
            out = self.sess.run(None, {self.input_name: blob})[0]
        return self.postprocess(out, r, conf_thr)

    def postprocess(self, out: np.ndarray, r: float, conf_thr: float) -> list[Det]:
        if self.fmt == "yolov8":
            pred = out[0].T  # (N, 84)
            boxes, scores = pred[:, :4], pred[:, 4:]
        else:
            pred = out[0].copy()  # (N, 85)
            grids, strides = self._yolox_grids(pred.shape[0])
            pred[:, :2] = (pred[:, :2] + grids[0]) * strides[0]
            pred[:, 2:4] = np.exp(pred[:, 2:4]) * strides[0]
            boxes, scores = pred[:, :4], pred[:, 4:5] * pred[:, 5:]
        keep_cls = scores[:, [PERSON, CELL_PHONE]]
        cls_idx = np.where(keep_cls[:, 1] > keep_cls[:, 0], CELL_PHONE, PERSON)
        conf = keep_cls.max(axis=1)
        m = conf >= conf_thr * 0.5
        boxes, conf, cls_idx = boxes[m], conf[m], cls_idx[m]
        if not len(conf):
            return []
        xywh = np.stack([boxes[:, 0] - boxes[:, 2] / 2, boxes[:, 1] - boxes[:, 3] / 2, boxes[:, 2], boxes[:, 3]], 1) / r
        dets: list[Det] = []
        for c in (PERSON, CELL_PHONE):
            sel = np.where(cls_idx == c)[0]
            if not len(sel):
                continue
            idx = cv2.dnn.NMSBoxes(xywh[sel].tolist(), conf[sel].tolist(), conf_thr * 0.5, 0.45)
            for i in np.array(idx).flatten():
                x, y, w, h = xywh[sel][i]
                dets.append(Det(c, float(conf[sel][i]), float(x), float(y), float(x + w), float(y + h)))
        return dets

    # ------------------------------------------------------------ head direction

    def head_yaw(self, frame: np.ndarray, d: Det) -> Optional[tuple[float, float]]:
        """Returns (yaw_degrees, score) for the largest face in the upper part of a person box."""
        if self.face is None:
            return None
        h, w = frame.shape[:2]
        x1, y1 = max(0, int(d.x1)), max(0, int(d.y1))
        x2, y2 = min(w, int(d.x2)), min(h, int(d.y1 + (d.y2 - d.y1) * 0.6))
        if x2 - x1 < 24 or y2 - y1 < 24:
            return None
        crop = frame[y1:y2, x1:x2]
        with self._face_lock:
            self.face.setInputSize((crop.shape[1], crop.shape[0]))
            _, faces = self.face.detect(crop)
        if faces is None or not len(faces):
            return None
        f = max(faces, key=lambda r: r[2] * r[3])
        re_x, le_x, nose_x = f[4], f[6], f[8]
        eye_mid = (re_x + le_x) / 2
        eye_dist = max(abs(le_x - re_x), 1.0)
        ratio = max(-1.0, min(1.0, (nose_x - eye_mid) / eye_dist * 2))
        return math.degrees(math.asin(ratio)), float(f[14])


def count_in_zone(dets: list[Det], roi_px: list[list[float]], conf_thr: float) -> int:
    """Anonymous people count for an area zone (no identities, no images kept)."""
    return sum(1 for d in dets if d.cls == PERSON and d.conf >= conf_thr and point_in_polygon(d.cx, d.cy, roi_px))


def classify(detector: Optional[Detector], frame, dets: list[Det], roi_px: list[list[float]],
             screen_yaw: float, yaw_tol: float, conf_thr: float,
             prev_box: Optional[tuple] = None) -> tuple[str, float, Optional[float], Optional[tuple]]:
    """Classifies one workstation. Returns (state, confidence, yaw, person_box).

    `prev_box` is the person box chosen for this desk on the previous frame. A
    detection that overlaps it keeps being followed even when fast movement blurs
    it (lower confidence, down to half the threshold) or the person leans out of
    the zone for a moment, so a quick movement is not mistaken for "away".
    """
    def follows(d: Det) -> bool:
        return prev_box is not None and box_iou((d.x1, d.y1, d.x2, d.y2), prev_box) >= 0.3

    people = [d for d in dets if d.cls == PERSON]
    in_roi = [d for d in people if point_in_polygon(d.cx, d.cy, roi_px)]
    persons = [d for d in people
               if (d.conf >= conf_thr and point_in_polygon(d.cx, d.cy, roi_px))
               or (d.conf >= conf_thr * 0.5 and follows(d))]
    if not persons:
        if in_roi:  # someone there but too uncertain to say
            return "unknown", max(d.conf for d in in_roi), None, None
        return "away", 0.9, None, None
    # prefer the person we were already following, then the biggest confident one
    p = max(persons, key=lambda d: (follows(d), d.area * d.conf))
    box = (p.x1, p.y1, p.x2, p.y2)
    pad_x, pad_y = (p.x2 - p.x1) * 0.15, (p.y2 - p.y1) * 0.1
    phones = [d for d in dets if d.cls == CELL_PHONE and d.conf >= conf_thr * 0.8
              and p.x1 - pad_x <= d.cx <= p.x2 + pad_x and p.y1 - pad_y <= d.cy <= p.y2 + pad_y]
    if phones:
        return "phone", max(d.conf for d in phones), None, box
    # someone else standing / sitting at the same desk: a conversation or small meeting
    xs = [q[0] for q in roi_px]
    ys = [q[1] for q in roi_px]
    mx, my = (max(xs) - min(xs)) * 0.3, (max(ys) - min(ys)) * 0.3
    visitors = [d for d in people if d is not p and d.conf >= conf_thr
                and min(xs) - mx <= d.cx <= max(xs) + mx and min(ys) - my <= d.cy <= max(ys) + my
                and box_iou((d.x1, d.y1, d.x2, d.y2), box) < 0.5]
    if visitors:
        return "meeting", min(p.conf, max(d.conf for d in visitors)), None, box
    head = detector.head_yaw(frame, p) if detector is not None else None
    if head is None:
        return "present", p.conf, None, box
    yaw, score = head
    state = "focused" if abs(yaw - screen_yaw) <= yaw_tol else "looking_away"
    return state, min(p.conf, score), yaw, box
