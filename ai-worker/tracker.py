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
