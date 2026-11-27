import unittest

from tracker import AdaptiveRate, FreezeDetector, StateSmoother, YawFilter, box_iou


class SmootherTest(unittest.TestCase):
    def test_single_blurred_frame_is_outvoted(self):
        sm = StateSmoother(window_sec=1.5)
        out = [sm.add("focused", 0.9, t * 0.25) for t in range(4)]
        out.append(sm.add("away", 0.9, 1.0))       # one missed detection during a fast move
        out.append(sm.add("focused", 0.9, 1.25))
        self.assertTrue(all(s == "focused" for s, _ in out))

    def test_real_change_wins_after_window(self):
        sm = StateSmoother(window_sec=1.0)
        for t in range(4):
            sm.add("focused", 0.9, t * 0.25)
        states = [sm.add("phone", 0.8, 1.0 + t * 0.25)[0] for t in range(6)]
        self.assertEqual(states[-1], "phone")


class YawTest(unittest.TestCase):
    def test_ema_reduces_jitter_and_resets(self):
        f = YawFilter(alpha=0.4)
        vals = [f.add(y, i * 0.2) for i, y in enumerate([0, 20, -20, 20, -20])]
        self.assertLess(max(abs(v) for v in vals[1:]), 20)
        self.assertEqual(f.add(50, 10.0), 50)       # long gap -> no stale smoothing


class RateTest(unittest.TestCase):
    def test_boost_on_motion_then_back_to_base(self):
        r = AdaptiveRate(base_fps=2, boost_fps=8, hold_sec=3)
        self.assertFalse(r.due(now=10.2, last_proc=10.0, motion=False, unsettled=False))  # 2 fps: wait 0.5 s
        self.assertTrue(r.due(now=10.2, last_proc=10.0, motion=True, unsettled=False))    # 8 fps while moving
        self.assertEqual(r.current_fps(12.0), 8)
        self.assertEqual(r.current_fps(14.0), 2)


class FreezeTest(unittest.TestCase):
    def test_identical_picture_means_frozen(self):
        f = FreezeDetector(freeze_sec=10)
        self.assertFalse(f.feed(b"a", 0))
        self.assertFalse(f.feed(b"a", 9))
        self.assertTrue(f.feed(b"a", 10.5))
        self.assertFalse(f.feed(b"b", 11))          # picture changes again -> live

    def test_iou(self):
        self.assertAlmostEqual(box_iou((0, 0, 10, 10), (0, 0, 10, 10)), 1.0)
        self.assertEqual(box_iou((0, 0, 1, 1), (5, 5, 6, 6)), 0.0)


class ClassifyTest(unittest.TestCase):
    def setUp(self):
        from detector import CELL_PHONE, PERSON, Det, classify
        self.Det, self.classify, self.P, self.PH = Det, classify, PERSON, CELL_PHONE
        self.roi = [[100, 100], [300, 100], [300, 300], [100, 300]]

    def test_blurred_person_is_still_followed(self):
        prev = (150, 120, 250, 290)
        blurred = [self.Det(self.P, 0.3, 160, 120, 260, 290)]  # below threshold 0.45 because of motion blur
        state, _, _, box = self.classify(None, None, blurred, self.roi, 0, 30, 0.45, prev)
        self.assertEqual(state, "present")
        self.assertIsNotNone(box)
        # without tracking history the same frame would be "unknown"
        state2, *_ = self.classify(None, None, blurred, self.roi, 0, 30, 0.45, None)
        self.assertEqual(state2, "unknown")

    def test_leaning_out_of_zone_is_not_away(self):
        prev = (200, 120, 300, 290)
        leaning = [self.Det(self.P, 0.8, 250, 120, 360, 290)]   # centre now outside the zone
        state, *_ = self.classify(None, None, leaning, self.roi, 0, 30, 0.45, prev)
        self.assertEqual(state, "present")

    def test_phone_in_hand(self):
        dets = [self.Det(self.P, 0.9, 150, 120, 250, 290), self.Det(self.PH, 0.5, 190, 200, 210, 230)]
        state, *_ = self.classify(None, None, dets, self.roi, 0, 30, 0.45, None)
        self.assertEqual(state, "phone")

    def test_visitor_at_desk_is_meeting(self):
        dets = [self.Det(self.P, 0.9, 150, 120, 250, 290), self.Det(self.P, 0.8, 255, 100, 320, 300)]
        state, *_ = self.classify(None, None, dets, self.roi, 0, 30, 0.45, None)
        self.assertEqual(state, "meeting")

    def test_zone_counter(self):
        from detector import count_in_zone
        from tracker import ZoneCounter
        dets = [self.Det(self.P, 0.9, 150, 120, 250, 290), self.Det(self.P, 0.2, 160, 130, 200, 200)]
        self.assertEqual(count_in_zone(dets, self.roi, 0.45), 1)
        z = ZoneCounter(5)
        for c in [0, 2, 2, 1, 3]:
            z.add(c)
        out = z.take()
        self.assertEqual((out["max_people"], out["entries"], out["current"], out["samples"]), (3, 4, 3, 5))
        self.assertIsNone(z.take())

    def test_empty_desk(self):
        state, *_ = self.classify(None, None, [], self.roi, 0, 30, 0.45, None)
        self.assertEqual(state, "away")


if __name__ == "__main__":
    unittest.main()
