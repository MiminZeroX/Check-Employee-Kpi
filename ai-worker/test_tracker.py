import unittest

from tracker import DeskTracker, point_in_polygon


class GeometryTest(unittest.TestCase):
    def test_point_in_polygon(self):
        sq = [[0, 0], [10, 0], [10, 10], [0, 10]]
        self.assertTrue(point_in_polygon(5, 5, sq))
        self.assertFalse(point_in_polygon(15, 5, sq))
        tri = [[0, 0], [10, 0], [0, 10]]
        self.assertTrue(point_in_polygon(2, 2, tri))
        self.assertFalse(point_in_polygon(8, 8, tri))


class TrackerTest(unittest.TestCase):
    def test_debounce_ignores_flicker(self):
        t = DeskTracker(1, min_state_sec=5, away_grace_sec=20)
        segs = [t.update("focused", 0.9, s) for s in range(0, 30)]
        # 3 seconds of looking away is shorter than min_state_sec: no new segment
        segs += [t.update("looking_away", 0.8, s) for s in range(30, 33)]
        segs += [t.update("focused", 0.9, s) for s in range(33, 40)]
        self.assertEqual([s for s in segs if s], [])
        self.assertEqual(t.state, "focused")

    def test_state_change_emits_segment(self):
        t = DeskTracker(7, min_state_sec=5, away_grace_sec=20)
        for s in range(0, 60):
            t.update("focused", 0.9, 1000 + s)
        out = [t.update("phone", 0.7, 1060 + s) for s in range(0, 10)]
        segs = [s for s in out if s]
        self.assertEqual(len(segs), 1)
        self.assertEqual(segs[0]["state"], "focused")
        self.assertEqual(segs[0]["workstation_id"], 7)
        self.assertEqual(t.state, "phone")
        # the new segment starts when the phone was first seen, not when it was confirmed
        self.assertAlmostEqual(t.seg_start, 1060)

    def test_away_needs_grace_period(self):
        t = DeskTracker(1, min_state_sec=5, away_grace_sec=20)
        t.update("focused", 0.9, 0)
        self.assertIsNone(t.update("away", 0.9, 1))
        self.assertIsNone(t.update("away", 0.9, 15))
        seg = t.update("away", 0.9, 21)
        self.assertIsNotNone(seg)
        self.assertEqual(t.state, "away")

    def test_flush_splits_long_segment(self):
        t = DeskTracker(1)
        t.update("focused", 0.9, 0)
        seg = t.flush(300)
        self.assertEqual(seg["state"], "focused")
        self.assertEqual(t.seg_start, 300)
        self.assertEqual(t.state, "focused")


if __name__ == "__main__":
    unittest.main()
