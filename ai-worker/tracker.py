"""Geometry helpers and the per-desk state machine (debounce -> activity segments).

No third-party imports so it can be unit-tested and used by the simulator.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Optional

STATES = ("focused", "present", "phone", "looking_away", "away", "unknown")


def point_in_polygon(x: float, y: float, poly: list[list[float]]) -> bool:
    """Ray casting; `poly` is a list of [x, y] points in the same units as x, y."""
    inside = False
    n = len(poly)
    j = n - 1
    for i in range(n):
        xi, yi = poly[i]
        xj, yj = poly[j]
        if (yi > y) != (yj > y) and x < (xj - xi) * (y - yi) / ((yj - yi) or 1e-12) + xi:
            inside = not inside
        j = i
    return inside


def iso(ts: float) -> str:
    return datetime.fromtimestamp(ts, tz=timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")


@dataclass
class DeskTracker:
    """Turns noisy per-frame classifications into stable activity segments.

    A new state must persist for `min_state_sec` (or `away_grace_sec` for "away")
    before the current segment is closed, which removes flicker such as a person
    briefly turning their head or being occluded for a frame.
    """

    workstation_id: int
    min_state_sec: float = 5.0
    away_grace_sec: float = 20.0
    state: Optional[str] = None
    seg_start: float = 0.0
    confs: list[float] = field(default_factory=list)
    cand: Optional[str] = None
    cand_since: float = 0.0
    cand_confs: list[float] = field(default_factory=list)
    yaw: Optional[float] = None

    def _segment(self, end: float) -> Optional[dict]:
        if self.state is None or end - self.seg_start < 1.0:
            return None
        conf = sum(self.confs) / len(self.confs) if self.confs else 0.0
        return {
            "workstation_id": self.workstation_id,
            "state": self.state,
            "started_at": iso(self.seg_start),
            "ended_at": iso(end),
            "confidence": round(conf, 3),
        }

    def update(self, raw: str, conf: float, now: float, yaw: Optional[float] = None) -> Optional[dict]:
        """Feeds one observation; returns a finished segment when the state changes."""
        if raw not in STATES:
            raw = "unknown"
        if yaw is not None:
            self.yaw = yaw
        if self.state is None:
            self.state, self.seg_start, self.confs = raw, now, [conf]
            return None
        if raw == self.state:
            self.cand = None
            self.confs.append(conf)
            if len(self.confs) > 2000:
                self.confs = self.confs[-1000:]
            return None
        if raw != self.cand:
            self.cand, self.cand_since, self.cand_confs = raw, now, [conf]
        else:
            self.cand_confs.append(conf)
        hold = self.away_grace_sec if raw == "away" else self.min_state_sec
        if now - self.cand_since >= hold:
            seg = self._segment(self.cand_since)
            self.state, self.seg_start, self.confs = raw, self.cand_since, self.cand_confs
            self.cand = None
            return seg
        return None

    def flush(self, now: float) -> Optional[dict]:
        """Closes the running segment at `now` and keeps the same state (used to
        persist long segments periodically and on shutdown)."""
        seg = self._segment(now)
        if seg:
            self.seg_start = now
            self.confs = self.confs[-1:]
        return seg

    @property
    def unsettled(self) -> bool:
        """True while a new state has been seen but not yet confirmed."""
        return self.cand is not None

    def reset(self) -> None:
        self.state = None
        self.cand = None
        self.confs = []

    def live(self) -> Optional[dict]:
        if self.state is None:
            return None
        conf = sum(self.confs[-20:]) / len(self.confs[-20:]) if self.confs else 0.0
        d = {"workstation_id": self.workstation_id, "state": self.state, "confidence": round(conf, 3), "since": iso(self.seg_start)}
        if self.yaw is not None:
            d["yaw"] = round(self.yaw, 1)
        return d


# ---------------------------------------------------------------------------
# Stability helpers (pure Python so they can be unit-tested)
# ---------------------------------------------------------------------------

def box_iou(a, b) -> float:
    """IoU of two (x1, y1, x2, y2) boxes."""
    ix1, iy1, ix2, iy2 = max(a[0], b[0]), max(a[1], b[1]), min(a[2], b[2]), min(a[3], b[3])
    inter = max(0.0, ix2 - ix1) * max(0.0, iy2 - iy1)
    union = (a[2] - a[0]) * (a[3] - a[1]) + (b[2] - b[0]) * (b[3] - b[1]) - inter
    return inter / union if union > 0 else 0.0


class StateSmoother:
    """Confidence-weighted vote over the last `window_sec` seconds.

    One blurred frame during a fast movement (missed person, missed phone, a head
    turn caught mid-way) is out-voted by the frames around it instead of
    flipping the desk state.
    """

    def __init__(self, window_sec: float = 1.5):
        self.window_sec = window_sec
        self.obs: list[tuple[float, str, float]] = []

    def add(self, state: str, conf: float, t: float) -> tuple[str, float]:
        self.obs.append((t, state, max(conf, 0.05)))
        cutoff = t - self.window_sec
        self.obs = [o for o in self.obs if o[0] >= cutoff][-30:]
        scores: dict[str, float] = {}
        counts: dict[str, int] = {}
        for _, s, c in self.obs:
            scores[s] = scores.get(s, 0.0) + c
            counts[s] = counts.get(s, 0) + 1
        best = max(scores, key=lambda s: (scores[s], s == state))
        share = scores[best] / sum(scores.values())
        return best, round(scores[best] / counts[best] * share, 3)

    @property
    def unanimous(self) -> bool:
        return len({s for _, s, _ in self.obs}) <= 1


class YawFilter:
    """Exponential moving average of head yaw; resets after a gap."""

    def __init__(self, alpha: float = 0.4, reset_after_sec: float = 3.0):
        self.alpha, self.reset_after = alpha, reset_after_sec
        self.value: Optional[float] = None
        self.t = 0.0

    def add(self, yaw: Optional[float], t: float) -> Optional[float]:
        if yaw is None:
            return None
        if self.value is None or t - self.t > self.reset_after:
            self.value = yaw
        else:
            self.value = self.alpha * yaw + (1 - self.alpha) * self.value
        self.t = t
        return self.value


class AdaptiveRate:
    """Processes `base_fps` when nothing happens and `boost_fps` while there is
    motion or a desk state is still being decided, for `hold_sec` afterwards."""

    def __init__(self, base_fps: float, boost_fps: float, hold_sec: float = 3.0):
        self.base_fps, self.boost_fps, self.hold_sec = base_fps, max(boost_fps, base_fps), hold_sec
        self.boost_until = 0.0

    def current_fps(self, now: float) -> float:
        return self.boost_fps if now < self.boost_until else self.base_fps

    def due(self, now: float, last_proc: float, motion: bool, unsettled: bool) -> bool:
        if motion or unsettled:
            self.boost_until = now + self.hold_sec
        return now - last_proc >= 1.0 / self.current_fps(now)


class FreezeDetector:
    """A camera/decoder that hangs often keeps sending the very same picture.
    Live images always differ slightly (sensor noise, on-screen clock), so an
    identical thumbnail for `freeze_sec` means the stream is frozen."""

    def __init__(self, freeze_sec: float = 15.0):
        self.freeze_sec = freeze_sec
        self.sig: Optional[bytes] = None
        self.since = 0.0

    def feed(self, signature: bytes, t: float) -> bool:
        if signature != self.sig:
            self.sig, self.since = signature, t
            return False
        return t - self.since >= self.freeze_sec
