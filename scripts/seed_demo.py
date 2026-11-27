#!/usr/bin/env python3
"""Fill a fresh installation with realistic demo data through the public API.

Everything here is fictitious. The script uses only the Python standard library.

    python3 scripts/seed_demo.py --url http://localhost:8080 --user admin --password admin12345 --days 10

It creates departments, employees, cameras, workstations, an AI-worker device and a
ZKTeco terminal, then pushes simulated attendance scans (via the real ZKTeco ADMS
protocol) and AI activity segments (via the AI-worker ingest API) for the last N days.
"""

import argparse
import datetime as dt
import json
import random
import urllib.error
import urllib.request

TZ = dt.timezone(dt.timedelta(hours=7))


class Api:
    def __init__(self, base):
        self.base = base.rstrip("/")
        self.token = None
        self.device_key = None

    def call(self, method, path, body=None, raw=None, headers=None, expect_json=True):
        h = {"Content-Type": "application/json"}
        if self.token:
            h["Authorization"] = f"Bearer {self.token}"
        h.update(headers or {})
        data = raw.encode() if raw is not None else (json.dumps(body).encode() if body is not None else None)
        req = urllib.request.Request(self.base + path, data=data, method=method, headers=h)
        try:
            with urllib.request.urlopen(req, timeout=30) as r:
                txt = r.read().decode()
        except urllib.error.HTTPError as e:
            raise SystemExit(f"{method} {path} -> {e.code}: {e.read().decode()}")
        return json.loads(txt) if expect_json and txt else txt

    def worker(self, method, path, body):
        return self.call(method, path, body, headers={"X-Device-Key": self.device_key})


FIRST = ["สมชาย", "สุดา", "วิชัย", "อรทัย", "ปกรณ์", "กมลวรรณ", "ธนพล", "ศิริพร", "ณัฐวุฒิ", "พิมพ์ชนก", "อนุชา", "จิราพร"]
LAST = ["ใจดี", "รักงาน", "มั่นคง", "สุขสบาย", "ทองดี", "ศรีสุข", "เจริญผล", "บุญมา", "แสงทอง", "พัฒนา", "วงศ์ใหญ่", "ดีเลิศ"]
POSITIONS = ["เจ้าหน้าที่ขาย", "นักพัฒนาซอฟต์แวร์", "เจ้าหน้าที่บัญชี", "เจ้าหน้าที่บริการลูกค้า"]


def iso(t):
    return t.astimezone(dt.timezone.utc).isoformat().replace("+00:00", "Z")


def simulate_day(rng, day, arrive, leave):
    """Returns a list of (state, start, end) for one person's working day."""
    segs = []
    t = arrive
    lunch_start = dt.datetime.combine(day, dt.time(12, 0), TZ) + dt.timedelta(minutes=rng.randint(-10, 15))
    lunch_done = False
    while t < leave:
        if not lunch_done and t >= lunch_start:
            d = dt.timedelta(minutes=rng.randint(45, 65))
            segs.append(("away", t, min(t + d, leave)))
            t += d
            lunch_done = True
            continue
        r = rng.random()
        if r < 0.62:
            st, d = "focused", rng.randint(8, 40)
        elif r < 0.75:
            st, d = "present", rng.randint(3, 20)
        elif r < 0.83:
            st, d = "looking_away", rng.randint(1, 6)
        elif r < 0.90:
            st, d = "phone", rng.randint(1, 9)
        elif r < 0.98:
            st, d = "away", rng.randint(3, 15)
        else:
            st, d = "unknown", rng.randint(1, 3)
        end = min(t + dt.timedelta(minutes=d), leave)
        segs.append((st, t, end))
        t = end
    return segs


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--url", default="http://localhost:8080")
    ap.add_argument("--user", default="admin")
    ap.add_argument("--password", required=True)
    ap.add_argument("--days", type=int, default=10)
    ap.add_argument("--employees", type=int, default=8)
    ap.add_argument("--seed", type=int, default=7)
    a = ap.parse_args()
    rng = random.Random(a.seed)
    api = Api(a.url)
    api.token = api.call("POST", "/api/auth/login", {"username": a.user, "password": a.password})["token"]

    br = api.call("POST", "/api/branches", {"name": "สำนักงานใหญ่", "address": "กรุงเทพมหานคร"})["id"]
    depts = [api.call("POST", "/api/departments", {"name": n, "branch_id": br})["id"]
             for n in ["ฝ่ายขาย", "ฝ่ายไอที", "ฝ่ายบัญชี", "ฝ่ายบริการลูกค้า"]]

    emps = []
    for i in range(a.employees):
        d = i % len(depts)
        eid = api.call("POST", "/api/employees", {
            "employee_code": f"EMP{i + 1:03d}", "first_name": FIRST[i % len(FIRST)], "last_name": LAST[i % len(LAST)],
            "position": POSITIONS[d], "department_id": depts[d], "attendance_ref": str(1001 + i),
            "work_start": "08:30", "work_end": "17:30", "monitoring_consent": True,
            "email": f"emp{i + 1}@example.com",
        })["id"]
        emps.append(eid)

    cams, desks = [], []
    for c in range(max(1, (a.employees + 3) // 4)):
        cid = api.call("POST", "/api/cameras", {
            "name": f"CAM-{c + 1:02d} โซนสำนักงาน {c + 1}", "branch_id": br, "floor": "3", "brand": "Hikvision",
            "location": f"โซน {chr(65 + c)}", "rtsp_url": f"rtsp://viewer:secret@192.168.1.{101 + c}:554/Streaming/Channels/102",
            "ai_enabled": True, "ai_fps": 2, "ai_width": 960,
        })["id"]
        cams.append(cid)
        for k in range(4):
            idx = c * 4 + k
            if idx >= a.employees:
                break
            x, y = (k % 2) * 0.5, (k // 2) * 0.5
            roi = [[x + 0.04, y + 0.06], [x + 0.46, y + 0.06], [x + 0.46, y + 0.46], [x + 0.04, y + 0.46]]
            wid = api.call("POST", "/api/workstations", {
                "camera_id": cid, "name": f"โต๊ะ {chr(65 + c)}{k + 1}", "employee_id": emps[idx], "roi": roi,
            })["id"]
            desks.append((wid, emps[idx], idx))

    api.device_key = api.call("POST", "/api/devices", {"name": "AI Worker #1", "kind": "ai_worker"})["api_key"]
    api.call("POST", "/api/devices", {"name": "เครื่องสแกนหน้า ประตูหลัก", "kind": "zkteco", "serial_no": "DEMO-ZK-0001"})

    today = dt.datetime.now(TZ).date()
    now = dt.datetime.now(TZ)
    total_events = 0
    for back in range(a.days - 1, -1, -1):
        day = today - dt.timedelta(days=back)
        if day.weekday() >= 5:
            continue
        att_lines, events = [], []
        for wid, eid, idx in desks:
            if rng.random() < 0.05:
                continue  # leave / absent
            late_bias = 18 if idx % 5 == 3 else 0
            scan = dt.datetime.combine(day, dt.time(8, 0), TZ) + dt.timedelta(minutes=rng.randint(0, 35) + late_bias)
            if scan > now:
                continue
            arrive = scan + dt.timedelta(minutes=rng.randint(2, 9))
            leave = dt.datetime.combine(day, dt.time(17, 30), TZ) + dt.timedelta(minutes=rng.randint(-5, 80))
            leave = min(leave, now)
            att_lines.append(f"{1001 + idx}\t{scan:%Y-%m-%d %H:%M:%S}\t0\t15\t0\t0")
            if leave < now:
                att_lines.append(f"{1001 + idx}\t{leave + dt.timedelta(minutes=3):%Y-%m-%d %H:%M:%S}\t1\t15\t0\t0")
            for st, s, e in simulate_day(rng, day, arrive, leave):
                if e > s:
                    events.append({"workstation_id": wid, "state": st, "started_at": iso(s), "ended_at": iso(e),
                                   "confidence": round(rng.uniform(0.6, 0.97), 2)})
        if att_lines:
            # ZKTeco ADMS push; first approve the terminal (it registers itself disabled if unknown)
            api.call("POST", "/iclock/cdata?SN=DEMO-ZK-0001&table=ATTLOG&Stamp=1", raw="\n".join(att_lines),
                     headers={"Content-Type": "text/plain"}, expect_json=False)
        for i in range(0, len(events), 1000):
            api.worker("POST", "/api/ingest/events", {"events": events[i:i + 1000]})
        total_events += len(events)

    # tasks, customers and supervisor reviews (work results drive the KPI)
    for i, eid in enumerate(emps):
        cust = api.call("POST", "/api/customers", {
            "name": f"ลูกค้าตัวอย่าง {i + 1}", "company": f"บริษัท ตัวอย่าง {i + 1} จำกัด", "status": "active",
            "owner_employee_id": eid, "satisfaction": rng.randint(3, 5),
        })["id"]
        for j in range(rng.randint(4, 8)):
            due = today - dt.timedelta(days=rng.randint(-5, a.days))
            status = rng.choice(["done", "done", "done", "in_progress", "todo"])
            # most work is finished on or before the due date, some is late
            finished = dt.datetime.combine(due, dt.time(16, 0), TZ) + dt.timedelta(days=rng.choice([-2, -1, 0, 0, 0, 1, 3]))
            finished = min(finished, now)
            api.call("POST", "/api/tasks", {
                "title": f"งาน {j + 1} ของ EMP{i + 1:03d}", "project": rng.choice(["CRM Rollout", "ปิดงบ Q4", "Website"]),
                "assignee_employee_id": eid, "customer_id": cust, "priority": rng.choice(["low", "normal", "high"]),
                "status": status, "due_date": due.isoformat(),
                "quality_score": rng.randint(3, 5) if status == "done" else None,
                "completed_at": iso(finished) if status == "done" else None,
            })
        api.call("POST", "/api/kpi/records", {
            "employee_id": eid, "metric_id": 5, "period_start": (today - dt.timedelta(days=a.days)).isoformat(),
            "period_end": today.isoformat(), "score": rng.randint(65, 95), "note": "ประเมินโดยหัวหน้างาน (ข้อมูลสมมติ)",
        })

    print(api.call("POST", "/api/activity/recompute", {
        "from": (today - dt.timedelta(days=a.days)).isoformat(), "to": today.isoformat()}))
    print(f"demo data created: {len(emps)} employees, {len(cams)} cameras, {total_events} activity segments")


if __name__ == "__main__":
    main()
