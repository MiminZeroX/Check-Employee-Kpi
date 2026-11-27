#!/usr/bin/env python3
"""End-to-end API checks against a running server that was filled with scripts/seed_demo.py.

    python3 tests/api_test.py --url http://localhost:8080 --password admin12345

Creates its own test records (prefixed "TEST-") and removes what it can afterwards.
Standard library only.
"""
import argparse, base64, datetime as dt, hashlib, hmac, http.server, json, socket, struct, sys, threading, time, urllib.error, urllib.request

PASS, FAIL = [], []


class Api:
    def __init__(self, base, token=None):
        self.base, self.token = base.rstrip('/'), token

    def req(self, method, path, body=None, expect=None):
        data = json.dumps(body).encode() if body is not None else None
        h = {'Content-Type': 'application/json'}
        if self.token:
            h['Authorization'] = f'Bearer {self.token}'
        r = urllib.request.Request(self.base + path, data=data, method=method, headers=h)
        try:
            with urllib.request.urlopen(r, timeout=30) as resp:
                code, txt = resp.status, resp.read().decode()
        except urllib.error.HTTPError as e:
            code, txt = e.code, e.read().decode()
        if expect is not None and code != expect:
            raise AssertionError(f'{method} {path} -> {code} (expected {expect}): {txt[:300]}')
        try:
            return code, json.loads(txt) if txt else None
        except json.JSONDecodeError:
            return code, txt

    def get(self, p, **k): return self.req('GET', p, **k)[1]
    def post(self, p, b=None, **k): return self.req('POST', p, b if b is not None else {}, **k)[1]
    def put(self, p, b, **k): return self.req('PUT', p, b, **k)[1]
    def delete(self, p, **k): return self.req('DELETE', p, **k)[1]


def check(name, cond, detail=''):
    (PASS if cond else FAIL).append(name)
    print(('  ok   ' if cond else '  FAIL ') + name + (f'  [{detail}]' if detail and not cond else ''))


def login(base, user, pw):
    a = Api(base)
    return Api(base, a.post('/api/auth/login', {'username': user, 'password': pw}, expect=200)['token'])


def section(t): print(f'\n== {t}')


def test_hr(base, admin, hr, emp):
    section('HR: shifts, holidays, requests, approvals, work sessions')
    today = dt.date.today()
    shifts = admin.get('/api/shifts')
    check('default shift exists', any(s['is_default'] for s in shifts))
    sid = admin.post('/api/shifts', {'name': 'TEST-กะบ่าย', 'start_time': '13:00', 'end_time': '22:00', 'break_start': '17:00',
                                     'break_end': '18:00', 'work_days': '1,2,3,4,5,6', 'grace_min': 10}, expect=200)['id']
    code, _ = admin.req('POST', '/api/shifts', {'name': 'x', 'start_time': '25:00', 'end_time': '1:00'})
    check('invalid shift time rejected', code == 400)
    import random
    yr = random.randint(2100, 8999)  # a fresh year each run keeps the test repeatable
    n = admin.post('/api/holidays/import-fixed', {'year': yr}, expect=200)['added']
    check('fixed Thai holidays imported', n >= 10 or len(admin.get(f'/api/holidays?year={yr}')) >= 15, n)
    again = admin.post('/api/holidays/import-fixed', {'year': yr}, expect=200)['added']
    check('holiday import is idempotent', again == 0, again)
    # leave: employee asks, HR approves
    types = emp.get('/api/leave-types')
    sick = next(t for t in types if t['code'] == 'sick')
    first_monday = dt.date(yr, 1, 5)
    future = first_monday + dt.timedelta(days=(7 - first_monday.weekday()) % 7 + 14)  # a Monday in that year
    bal0 = emp.get(f'/api/leave-balances?year={yr}')['rows']
    r = emp.post('/api/requests', {'kind': 'leave', 'leave_type_id': sick['id'], 'start_date': str(future),
                                    'end_date': str(future + dt.timedelta(days=2)), 'reason': 'TEST-ไปพบแพทย์'}, expect=200)
    check('leave request counts working days', r['days'] == 3.0, r)
    code, body = emp.req('POST', '/api/requests', {'kind': 'leave', 'leave_type_id': sick['id'], 'start_date': str(future),
                                                    'reason': 'TEST-ซ้ำ'})
    check('overlapping leave rejected', code == 400, body)
    code, _ = emp.req('POST', f"/api/requests/{r['id']}/decide", {'approve': True})
    check('employee cannot approve', code == 403)
    inbox = hr.get('/api/approvals')
    check('HR inbox shows the request', any(x['id'] == r['id'] for x in inbox['requests']))
    code, _ = hr.req('POST', f"/api/requests/{r['id']}/decide", {'approve': False})
    check('reject needs a reason', code == 400)
    hr.post(f"/api/requests/{r['id']}/decide", {'approve': True, 'note': 'TEST-หายไว ๆ'}, expect=200)
    bal = {b['code']: b for b in emp.get(f'/api/leave-balances?year={yr}')['rows']}
    before = {b['code']: b for b in bal0}
    check('leave balance reduced', bal['sick']['used'] == before['sick']['used'] + 3, (before['sick'], bal['sick']))
    code, body = emp.req('POST', '/api/requests', {'kind': 'leave', 'leave_type_id': next(t for t in types if t['code'] == 'personal')['id'],
                                                    'start_date': str(future + dt.timedelta(days=7)), 'end_date': str(future + dt.timedelta(days=11)),
                                                    'reason': 'TEST-เกินโควตา'})
    check('leave over quota rejected', code == 400 and 'คงเหลือ' in str(body), body)
    ot = emp.post('/api/requests', {'kind': 'overtime', 'start_date': str(future + dt.timedelta(days=21)), 'start_time': '18:00', 'hours': 2, 'reason': 'TEST-ปิดงบ'}, expect=200)
    off = emp.post('/api/requests', {'kind': 'offsite', 'start_date': str(future + dt.timedelta(days=28)), 'location': 'TEST-ลูกค้า สีลม',
                                      'reason': 'TEST-ประชุมลูกค้า'}, expect=200)
    emp.post(f"/api/requests/{off['id']}/cancel", expect=200)
    mine = emp.get('/api/requests?mine=true')
    check('own request list', {ot['id'], off['id'], r['id']} <= {x['id'] for x in mine})
    check('cancelled request', next(x for x in mine if x['id'] == off['id'])['status'] == 'cancelled')
    # work sessions for the seeded period
    frm = today - dt.timedelta(days=13)
    admin.post('/api/activity/recompute', {'from': str(frm), 'to': str(today)}, expect=200)
    ws = admin.get(f'/api/work-sessions?from={frm}&to={today}')
    statuses = {w['status'] for w in ws}
    check('work sessions computed', len(ws) > 50, len(ws))
    check('weekends are days off', 'dayoff' in statuses, statuses)
    check('present days have check-in', all(w['check_in'] for w in ws if w['status'] == 'present'))
    daily = admin.get(f'/api/attendance/daily?date={today - dt.timedelta(days=1)}')
    check('daily attendance has status counts', 'present' in daily and 'rows' in daily)
    mws = emp.get(f'/api/me/work-sessions?from={frm}&to={today}')
    check('employee sees only own sessions', mws and all(w['employee_id'] == mws[0]['employee_id'] for w in mws))
    # shift assignment: day off overrides
    emps = admin.get('/api/employees')
    e1 = emps[0]['id']
    admin.post('/api/shift-assignments', {'employee_ids': [e1], 'from': str(today + dt.timedelta(days=30)),
                                          'to': str(today + dt.timedelta(days=31)), 'shift_id': None, 'note': 'TEST'}, expect=200)
    asg = admin.get(f'/api/shift-assignments?from={today + dt.timedelta(days=30)}&to={today + dt.timedelta(days=31)}')
    check('shift assignment stored', len([a for a in asg if a['note'] == 'TEST']) == 2)
    # org chart & manager scoping
    depts = admin.get('/api/departments')
    oc = admin.get('/api/org-chart')
    check('org chart lists departments', len(oc['departments']) == len(depts))
    code, body = admin.req('PUT', f"/api/departments/{depts[0]['id']}", {'name': depts[0]['name'], 'parent_id': depts[0]['id']})
    check('department cannot be its own parent', code == 400, body)
    # cleanup
    for a in asg:
        if a['note'] == 'TEST':
            admin.delete(f"/api/shift-assignments/{a['id']}")
    admin.delete(f'/api/shifts/{sid}')
    return [r['id'], ot['id'], off['id']]


def multipart(fields, file_field, file_name, data):
    boundary = 'TESTBOUNDARY7MA4YWxkTrZu0gW'
    out = b''
    for k, v in fields.items():
        out += f'--{boundary}\r\nContent-Disposition: form-data; name="{k}"\r\n\r\n{v}\r\n'.encode()
    out += (f'--{boundary}\r\nContent-Disposition: form-data; name="{file_field}"; filename="{file_name}"\r\n'
            f'Content-Type: application/octet-stream\r\n\r\n').encode() + data + b'\r\n' + f'--{boundary}--\r\n'.encode()
    return out, f'multipart/form-data; boundary={boundary}'


def raw(api, method, path, body, ctype):
    r = urllib.request.Request(api.base + path, data=body, method=method, headers={'Authorization': f'Bearer {api.token}', 'Content-Type': ctype})
    try:
        with urllib.request.urlopen(r, timeout=30) as resp:
            return resp.status, resp.headers, resp.read()
    except urllib.error.HTTPError as e:
        return e.code, e.headers, e.read()


def test_crm(base, admin, hr, emp):
    section('CRM: projects, contacts, pipeline, tickets, documents, webhooks')
    # webhook receiver
    got = []
    class H(http.server.BaseHTTPRequestHandler):
        def do_POST(self):
            body = self.rfile.read(int(self.headers['Content-Length']))
            got.append((self.headers['X-SmartOffice-Event'], self.headers['X-SmartOffice-Signature'], body))
            self.send_response(200); self.end_headers()
        def log_message(self, *a): pass
    srv = http.server.HTTPServer(('127.0.0.1', 8097), H)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    wh = admin.post('/api/webhooks', {'name': 'TEST-hook', 'url': 'http://127.0.0.1:8097/hook', 'events': 'deal.stage_changed,ticket.created'}, expect=200)
    code, _ = admin.req('POST', '/api/webhooks', {'name': 'x', 'url': 'ftp://x', 'events': '*'})
    check('webhook URL validated', code == 400)
    t = admin.post(f"/api/webhooks/{wh['id']}/test", expect=200)
    check('webhook test delivered', t['status'] == 200, t)
    ev, sig, body = got[-1]
    check('webhook signature is HMAC-SHA256 of body', sig == 'sha256=' + hmac.new(wh['secret'].encode(), body, hashlib.sha256).hexdigest())
    # projects
    emps = admin.get('/api/employees')
    custs = admin.get('/api/customers')
    pid = admin.post('/api/projects', {'name': 'TEST-โครงการ ERP', 'customer_id': custs[0]['id'], 'owner_employee_id': emps[0]['id'],
                                       'status': 'active', 'start_date': str(dt.date.today()), 'due_date': str(dt.date.today() + dt.timedelta(days=30))}, expect=200)['id']
    tid = admin.post('/api/tasks', {'title': 'TEST-งานโครงการ', 'project_id': pid, 'assignee_employee_id': emps[0]['id'], 'status': 'done',
                                    'due_date': str(dt.date.today()), 'quality_score': 4}, expect=200)['id']
    pr = next(p for p in admin.get('/api/projects') if p['id'] == pid)
    check('project progress counted', pr['tasks_total'] == 1 and pr['tasks_done'] == 1 and pr['tasks_on_time'] == 1, pr)
    code, _ = admin.req('POST', '/api/projects', {'name': 'x', 'start_date': '2026-05-01', 'due_date': '2026-04-01'})
    check('project dates validated', code == 400)
    # contacts
    cid = admin.post('/api/contacts', {'customer_id': custs[0]['id'], 'name': 'TEST-คุณติดต่อ', 'is_primary': True}, expect=200)['id']
    check('contact listed for customer', any(c['id'] == cid and c['is_primary'] for c in admin.get(f"/api/contacts?customer_id={custs[0]['id']}")))
    # deals
    did = admin.post('/api/deals', {'title': 'TEST-ดีลระบบ', 'customer_id': custs[0]['id'], 'value': 100000}, expect=200)['id']
    mv = admin.post(f'/api/deals/{did}/stage', {'stage': 'proposal'}, expect=200)
    check('stage move sets probability', mv['probability'] == 50)
    summ = admin.get('/api/deals/summary')
    check('pipeline weighted value', summ['weighted_pipeline'] >= 50000, summ)
    time.sleep(1.5)
    check('deal webhook fired', any(e == 'deal.stage_changed' for e, _, _ in got))
    code, _ = hr.req('POST', '/api/deals', {'title': 'x'})
    check('HR (crm.view only) cannot create deals', code == 403)
    # tickets
    tk = admin.post('/api/tickets', {'subject': 'TEST-เครื่องพิมพ์เสีย', 'customer_id': custs[0]['id'], 'contact_id': cid, 'priority': 'urgent',
                                     'assignee_employee_id': emps[0]['id']}, expect=200)
    due = dt.datetime.fromisoformat(tk['due_at'].replace('Z', '+00:00'))
    check('urgent ticket SLA ~4h', 3.9 < (due - dt.datetime.now(dt.timezone.utc)).total_seconds() / 3600 < 4.1)
    mine = emp.get('/api/tickets?mine=true')
    check('assignee sees own ticket', any(x['id'] == tk['id'] for x in mine))
    emp.post(f"/api/tickets/{tk['id']}/status", {'status': 'resolved'}, expect=200)
    check('assignee resolved ticket', next(x for x in admin.get('/api/tickets') if x['id'] == tk['id'])['resolved_at'] is not None)
    # documents
    body, ctype = multipart({'title': 'TEST-สัญญาจ้าง', 'entity': 'employee', 'entity_id': str(emps[0]['id'])}, 'file', 'contract.pdf', b'%PDF-1.4 test')
    code, _, resp = raw(admin, 'POST', '/api/documents', body, ctype)
    doc = json.loads(resp)
    check('document uploaded', code == 200 and len(doc.get('sha256', '')) == 64, resp[:200])
    body2, ctype2 = multipart({'title': 'x'}, 'file', 'evil.html', b'<script>alert(1)</script>')
    code, _, _ = raw(admin, 'POST', '/api/documents', body2, ctype2)
    check('HTML upload rejected', code == 400)
    big, ctype3 = multipart({}, 'file', 'big.pdf', b'0' * (21 * 1024 * 1024))
    code, _, _ = raw(admin, 'POST', '/api/documents', big, ctype3)
    check('file over 20MB rejected', code in (400, 413), code)
    code, hdrs, data = raw(admin, 'GET', f"/api/documents/{doc['id']}/download", None, 'application/json')
    check('download is an attachment', code == 200 and hdrs['Content-Disposition'].startswith('attachment') and data == b'%PDF-1.4 test')
    own = emp.get('/api/documents')  # emp001 is employee 1 -> sees only own documents
    check('employee sees only own documents', all(d['entity'] == 'employee' for d in own))
    # cleanup
    admin.delete(f"/api/documents/{doc['id']}"); admin.delete(f"/api/tickets/{tk['id']}"); admin.delete(f'/api/deals/{did}')
    admin.delete(f'/api/contacts/{cid}'); admin.delete(f'/api/tasks/{tid}'); admin.delete(f'/api/projects/{pid}')
    admin.delete(f"/api/webhooks/{wh['id']}")
    srv.shutdown()


def start_mocks():
    """go2rtc stand-in on :1984 (snapshot + stream registry) and an ONVIF camera on :8098."""
    import cv2, numpy as np
    img = np.full((180, 320, 3), 255, np.uint8)
    jpg = cv2.imencode('.jpg', img)[1].tobytes()
    streams, soap = {}, []

    class G(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            if self.path.startswith('/api/frame.jpeg'):
                self.send_response(200); self.send_header('Content-Type', 'image/jpeg'); self.end_headers(); self.wfile.write(jpg)
            elif self.path.startswith('/api/streams'):
                b = json.dumps({k: {} for k in streams}).encode(); self.send_response(200); self.end_headers(); self.wfile.write(b)
            else:
                self.send_response(200); self.end_headers(); self.wfile.write(b'{}')
        def do_PUT(self):
            from urllib.parse import urlparse, parse_qs
            q = parse_qs(urlparse(self.path).query); streams[q['name'][0]] = q['src'][0]
            self.send_response(200); self.end_headers()
        def do_DELETE(self): self.send_response(200); self.end_headers()
        def log_message(self, *a): pass

    class O(http.server.BaseHTTPRequestHandler):
        def do_POST(self):
            body = self.rfile.read(int(self.headers['Content-Length'])).decode()
            soap.append(body)
            if 'PasswordDigest' not in body:
                self.send_response(401); self.end_headers(); return
            if 'GetCapabilities' in body:
                resp = '<tt:Media><tt:XAddr>http://127.0.0.1:8098/onvif/Media</tt:XAddr></tt:Media><tt:PTZ><tt:XAddr>http://127.0.0.1:8098/onvif/PTZ</tt:XAddr></tt:PTZ>'
            elif 'GetProfiles' in body:
                resp = '<trt:Profiles token="Profile_1" fixed="true"><tt:Name>main</tt:Name></trt:Profiles>'
            elif 'GetPresets' in body:
                resp = '<tptz:Preset token="1"><tt:Name>ประตู</tt:Name></tptz:Preset><tptz:Preset token="2"><tt:Name>โต๊ะ A</tt:Name></tptz:Preset>'
            else:
                resp = '<ok/>'
            out = f'<?xml version="1.0"?><s:Envelope><s:Body>{resp}</s:Body></s:Envelope>'.encode()
            self.send_response(200); self.send_header('Content-Type', 'application/soap+xml'); self.end_headers(); self.wfile.write(out)
        def log_message(self, *a): pass

    servers = []
    for port, h in ((1984, G), (8098, O)):
        try:
            srv = http.server.ThreadingHTTPServer(('127.0.0.1', port), h)
            threading.Thread(target=srv.serve_forever, daemon=True).start(); servers.append(srv)
        except OSError:
            pass
    # WS-Discovery responder (answers multicast probes like a camera would)
    def responder():
        s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM, socket.IPPROTO_UDP)
        s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        s.bind(('', 3702))
        s.setsockopt(socket.IPPROTO_IP, socket.IP_ADD_MEMBERSHIP, struct.pack('4sl', socket.inet_aton('239.255.255.250'), socket.INADDR_ANY))
        while True:
            data, addr = s.recvfrom(65535)
            if b'Probe' in data:
                s.sendto(b'<e:Envelope><e:Body><d:ProbeMatches><d:ProbeMatch><d:Scopes>onvif://www.onvif.org/name/TEST_CAM onvif://www.onvif.org/hardware/DS-2CD2143</d:Scopes>'
                         b'<d:XAddrs>http://127.0.0.1:8098/onvif/device_service</d:XAddrs></d:ProbeMatch></d:ProbeMatches></e:Body></e:Envelope>', addr)
    threading.Thread(target=responder, daemon=True).start()
    return streams, soap


def test_vms(base, admin, hr, emp):
    section('VMS / AI: zones, occupancy, heatmap, PTZ, discovery, playback, privacy masks')
    streams, soap = start_mocks()
    time.sleep(0.5)
    cam = admin.post('/api/cameras', {'name': 'TEST-PTZ', 'rtsp_url': 'rtsp://u:secret@10.0.0.9/live', 'ptz_supported': True,
                                      'onvif_host': '127.0.0.1:8098', 'onvif_user': 'admin', 'onvif_password': 'cam-pass',
                                      'playback_url': 'rtsp://u:secret@nvr/Streaming/tracks/101?starttime={start}&endtime={end}',
                                      'privacy_masks': [[[0, 0], [0.5, 0], [0.5, 0.5], [0, 0.5]]]}, expect=200)
    listed = next(c for c in admin.get('/api/cameras') if c['id'] == cam['id'])
    check('camera secrets masked in API', '****' in listed['rtsp_url'] and '****' in listed['playback_url'] and listed['has_onvif_password'] == 1, listed)
    # PTZ through ONVIF
    admin.post(f"/api/cameras/{cam['id']}/ptz", {'action': 'move', 'pan': 0.5, 'tilt': 0, 'zoom': 0}, expect=200)
    admin.post(f"/api/cameras/{cam['id']}/ptz", {'action': 'stop'}, expect=200)
    check('ONVIF ContinuousMove + Stop with digest auth', any('ContinuousMove' in b and 'Profile_1' in b for b in soap) and any('<Stop' in b for b in soap))
    check('ONVIF password not sent in clear', not any('cam-pass' in b for b in soap))
    presets = admin.get(f"/api/cameras/{cam['id']}/ptz/presets")
    check('PTZ presets listed', [p['token'] for p in presets] == ['1', '2'], presets)
    code, _ = hr.req('POST', f"/api/cameras/{cam['id']}/ptz", {'action': 'stop'})
    check('PTZ needs cameras.manage', code == 403)
    code, _ = admin.req('POST', f"/api/cameras/{cam['id']}/ptz", {'action': 'preset', 'preset': '1"><x'})
    check('PTZ preset token validated', code == 400)
    # discovery
    found = admin.post('/api/cameras/discover', expect=200)
    check('ONVIF WS-Discovery finds camera', any(f.get('name') == 'TEST CAM' for f in found), found)
    # playback
    pb = admin.post(f"/api/cameras/{cam['id']}/playback", {'start': '2026-01-31 08:00:00', 'minutes': 15}, expect=200)
    src = streams.get(pb['stream'], '')
    check('playback stream registered with UTC times', 'starttime=20260131T010000Z' in src and 'endtime=20260131T011500Z' in src, src)
    code, _ = emp.req('POST', f"/api/cameras/{cam['id']}/playback", {'start': '2026-01-31 08:00:00'})
    check('playback needs playback.view', code == 403)
    # privacy mask applied to snapshots on the server
    import cv2, numpy as np
    sess = admin.post('/api/video/session', expect=200)
    req = urllib.request.Request(base + '/api/video/session', data=b'{}', method='POST', headers={'Authorization': f'Bearer {admin.token}', 'Content-Type': 'application/json'})
    cookie = urllib.request.urlopen(req).headers['Set-Cookie'].split(';')[0]
    stream_name = next(c for c in admin.get('/api/cameras') if c['id'] == cam['id'])['stream_name']
    r = urllib.request.urlopen(urllib.request.Request(f'{base}/video/api/frame.jpeg?src={stream_name}', headers={'Cookie': cookie}))
    im = cv2.imdecode(np.frombuffer(r.read(), np.uint8), cv2.IMREAD_GRAYSCALE)
    check('snapshot masked area is black, rest untouched', im[40, 40] < 30 and im[140, 260] > 220, (int(im[40, 40]), int(im[140, 260])))
    # zones + occupancy via the AI-worker heartbeat
    z = admin.post('/api/zones', {'camera_id': cam['id'], 'name': 'TEST-ห้องประชุม', 'kind': 'meeting_room', 'capacity': 6,
                                  'roi': [[0, 0], [1, 0], [1, 1], [0, 1]]}, expect=200)
    dev = admin.post('/api/devices', {'name': 'TEST-worker', 'kind': 'ai_worker'}, expect=200)
    w = Api(base); hb = lambda body: urllib.request.urlopen(urllib.request.Request(base + '/api/ingest/heartbeat', data=json.dumps(body).encode(),
                                                                                       method='POST', headers={'X-Device-Key': dev['api_key'], 'Content-Type': 'application/json'}))
    cfg = json.loads(urllib.request.urlopen(urllib.request.Request(base + '/api/ingest/config', headers={'X-Device-Key': dev['api_key']})).read())
    check('worker config includes zones', any(zz['id'] == z['id'] for c in cfg['cameras'] for zz in c.get('zones', [])))
    hb({'zones': [{'zone_id': z['id'], 'samples': 10, 'people_sum': 40, 'max_people': 6, 'current': 5, 'entries': 3}]})
    occ = next(o for o in admin.get('/api/zones/occupancy')['zones'] if o['zone_id'] == z['id'])
    check('zone occupancy stored (avg 4, peak 6, now 5)', occ['peak'] == 6 and occ['people_now'] == 5 and 4.0 in occ['avg_by_hour'], occ)
    hm = admin.get(f'/api/analytics/heatmap?from={dt.date.today() - dt.timedelta(days=13)}&to={dt.date.today()}')
    check('heatmap 7x24 with data', len(hm['cells']) == 7 and len(hm['cells'][0]) == 24 and any(c['people'] > 0 for row in hm['cells'] for c in row))
    # cleanup
    admin.delete(f"/api/zones/{z['id']}"); admin.delete(f"/api/devices/{dev['id']}"); admin.delete(f"/api/cameras/{cam['id']}")

# ---------------------------------------------------------------- security (MFA / SSO / rate limit)

def totp(secret, step):
    key = base64.b32decode(secret + '=' * (-len(secret) % 8))
    h = hmac.new(key, struct.pack('>q', step), hashlib.sha1).digest()
    o = h[19] & 15
    return '%06d' % ((struct.unpack('>I', h[o:o + 4])[0] & 0x7fffffff) % 1000000)


OIDC_PORT = 8099


def start_oidc_mock():
    """Minimal OpenID provider: discovery, authorize (auto-approves), token with PKCE check, JWKS."""
    from cryptography.hazmat.primitives.asymmetric import rsa, padding
    from cryptography.hazmat.primitives import hashes
    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    pub = key.public_key().public_numbers()
    b64 = lambda b: base64.urlsafe_b64encode(b).rstrip(b'=').decode()
    n2b = lambda i: i.to_bytes((i.bit_length() + 7) // 8, 'big')
    iss = f'http://127.0.0.1:{OIDC_PORT}'
    codes, users = {}, {'sub': 'u-123', 'email': 'TEST-sso@example.com'}

    def sign(claims):
        head = b64(json.dumps({'alg': 'RS256', 'kid': 'k1', 'typ': 'JWT'}).encode())
        body = b64(json.dumps(claims).encode())
        sig = key.sign(f'{head}.{body}'.encode(), padding.PKCS1v15(), hashes.SHA256())
        return f'{head}.{body}.{b64(sig)}'

    class H(http.server.BaseHTTPRequestHandler):
        def log_message(self, *a): pass

        def send(self, code, obj=None, loc=None):
            self.send_response(code)
            if loc: self.send_header('Location', loc)
            data = json.dumps(obj).encode() if obj is not None else b''
            self.send_header('Content-Type', 'application/json'); self.send_header('Content-Length', str(len(data)))
            self.end_headers(); self.wfile.write(data)

        def do_GET(self):
            from urllib.parse import urlparse, parse_qs, urlencode
            u = urlparse(self.path); q = {k: v[0] for k, v in parse_qs(u.query).items()}
            if u.path == '/.well-known/openid-configuration':
                return self.send(200, {'issuer': iss, 'authorization_endpoint': iss + '/authorize', 'token_endpoint': iss + '/token', 'jwks_uri': iss + '/jwks'})
            if u.path == '/jwks':
                return self.send(200, {'keys': [{'kty': 'RSA', 'kid': 'k1', 'use': 'sig', 'alg': 'RS256', 'n': b64(n2b(pub.n)), 'e': b64(n2b(pub.e))}]})
            if u.path == '/authorize':
                code = b64(hashlib.sha256(q['state'].encode()).digest())[:20]
                codes[code] = q
                return self.send(302, loc=q['redirect_uri'] + '?' + urlencode({'code': code, 'state': q['state']}))
            self.send(404, {})

        def do_POST(self):
            from urllib.parse import parse_qs
            f = {k: v[0] for k, v in parse_qs(self.rfile.read(int(self.headers['Content-Length'])).decode()).items()}
            a = codes.pop(f.get('code'), None)
            if not a or b64(hashlib.sha256(f['code_verifier'].encode()).digest()) != a['code_challenge'] or f['client_id'] != 'smartoffice-test':
                return self.send(400, {'error': 'invalid_grant'})
            now = int(time.time())
            tok = sign({'iss': iss, 'aud': 'smartoffice-test', 'sub': users['sub'], 'email': users['email'], 'email_verified': True,
                        'nonce': a['nonce'], 'iat': now, 'exp': now + 300})
            self.send(200, {'access_token': 'x', 'token_type': 'Bearer', 'id_token': tok})

    srv = http.server.ThreadingHTTPServer(('127.0.0.1', OIDC_PORT), H)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *a, **k): return None


def raw_get(url, headers=None):
    op = urllib.request.build_opener(NoRedirect)
    try:
        r = op.open(urllib.request.Request(url, headers=headers or {}), timeout=10)
    except urllib.error.HTTPError as e:
        r = e
    return r.status if hasattr(r, 'status') else r.code, r.headers


def test_security(base, admin, hr, emp):
    section('MFA (TOTP)')
    anon = Api(base)
    uname = f'TEST-mfa-{int(time.time())}'
    u = admin.post('/api/users', {'username': uname, 'display_name': 'MFA test', 'password': 'mfapass123', 'role': 'viewer'}, expect=200)
    me = login(base, uname, 'mfapass123')
    check('MFA off by default', me.get('/api/auth/mfa')['enabled'] is False)
    code, _ = me.req('POST', '/api/auth/mfa/setup', {'password': 'wrong-pass'})
    check('setup needs the password', code == 400)
    s = me.post('/api/auth/mfa/setup', {'password': 'mfapass123'}, expect=200)
    check('setup returns secret, otpauth URI and QR', len(s['secret']) == 32 and s['uri'].startswith('otpauth://totp/') and s['qr'].startswith('data:image/svg+xml;base64,'))
    step = int(time.time()) // 30
    code, _ = me.req('POST', '/api/auth/mfa/enable', {'code': '000000' if totp(s['secret'], step) != '000000' else '111111'})
    check('wrong code does not enable', code == 400)
    me.post('/api/auth/mfa/enable', {'code': totp(s['secret'], step)}, expect=200)
    check('MFA now on', me.get('/api/auth/mfa')['enabled'] is True)
    r = anon.post('/api/auth/login', {'username': uname, 'password': 'mfapass123'}, expect=200)
    check('password login asks for the code, no session yet', r.get('mfa_required') is True and 'token' not in r)
    code, _ = Api(base, r['mfa_token']).req('GET', '/api/auth/me')
    check('pending-MFA token is not a session', code == 401)
    code, _ = anon.req('POST', '/api/auth/mfa/verify', {'mfa_token': r['mfa_token'], 'code': totp(s['secret'], step)})
    check('code already used cannot be replayed', code == 400)
    v = anon.post('/api/auth/mfa/verify', {'mfa_token': r['mfa_token'], 'code': totp(s['secret'], step + 1)}, expect=200)
    check('correct code gives a session', 'token' in v and v['user']['name'] == 'MFA test')
    code, _ = anon.req('POST', '/api/auth/mfa/verify', {'mfa_token': r['mfa_token'] + 'x', 'code': totp(s['secret'], step + 1)})
    check('tampered MFA token rejected', code == 400)
    admin.put(f"/api/users/{u['id']}", {'display_name': 'MFA test', 'role': 'viewer', 'is_active': True, 'mfa_reset': True}, expect=200)
    r = anon.post('/api/auth/login', {'username': uname, 'password': 'mfapass123'}, expect=200)
    check('admin MFA reset turns it off', 'token' in r)
    admin.delete(f"/api/users/{u['id']}")

    section('Single sign-on (OpenID Connect)')
    if not anon.get('/api/auth/oidc').get('enabled'):
        check('OIDC configured on the test server (OIDC_ISSUER=http://127.0.0.1:8099)', False, 'start the server with the OIDC_* test variables')
    else:
        srv = start_oidc_mock()
        try:
            u = admin.post('/api/users', {'username': f'TEST-sso-{int(time.time())}', 'display_name': 'SSO test', 'email': 'TEST-sso@example.com',
                                          'password': 'ssopass123', 'role': 'viewer'}, expect=200)
            code, h = raw_get(base + '/api/auth/oidc/start')
            cookie = h['Set-Cookie'].split(';')[0]
            check('start redirects to the provider with PKCE + state cookie', code in (302, 303, 307) and 'code_challenge=' in h['Location'] and cookie.startswith('so_oidc='))
            code, h2 = raw_get(h['Location'])
            cb = h2['Location']
            code, h3 = raw_get(cb)
            check('callback without the browser cookie is refused', '#sso_error=' in h3['Location'])
            # a fresh round with the cookie
            code, h = raw_get(base + '/api/auth/oidc/start'); cookie = h['Set-Cookie'].split(';')[0]
            _, h2 = raw_get(h['Location'])
            _, h3 = raw_get(h2['Location'], {'Cookie': cookie})
            loc = h3['Location']
            check('callback hands over a one-time code', loc.startswith('/#sso='), loc)
            one = loc.split('=', 1)[1]
            r = anon.post('/api/auth/oidc/exchange', {'code': one}, expect=200)
            check('exchange gives a session for the e-mail-matched user', r['user']['name'] == 'SSO test')
            code, _ = anon.req('POST', '/api/auth/oidc/exchange', {'code': one})
            check('one-time code cannot be reused', code == 401)
            rows = admin.get('/api/users')
            check('account now linked to the SSO identity', any(x['id'] == u['id'] and x['sso_linked'] for x in rows))
            _, h3 = raw_get(h2['Location'], {'Cookie': cookie})
            check('provider code / state cannot be replayed', '#sso_error=' in h3['Location'])
            admin.delete(f"/api/users/{u['id']}")
        finally:
            srv.shutdown()

    section('API rate limit')
    import concurrent.futures as cf
    def hit(_):
        try:
            with urllib.request.urlopen(base + '/api/public/config', timeout=10) as r: return r.status
        except urllib.error.HTTPError as e: return e.code
    with cf.ThreadPoolExecutor(16) as ex:
        codes = list(ex.map(hit, range(1500)))
    check('burst of 1500 requests is throttled with 429', codes.count(429) > 0 and codes.count(200) > 0, f'{codes.count(200)} ok / {codes.count(429)} limited')
    time.sleep(3)
    check('allowed again after a pause', hit(0) == 200)


# ---------------------------------------------------------------- reports (xlsx, settings, branches)

def raw_bytes(api, path):
    r = urllib.request.Request(api.base + path, headers={'Authorization': f'Bearer {api.token}'})
    try:
        with urllib.request.urlopen(r, timeout=30) as resp:
            return resp.status, resp.headers.get('Content-Type', ''), resp.read()
    except urllib.error.HTTPError as e:
        return e.code, '', e.read()


def test_reports(base, admin, hr, emp):
    import io, zipfile
    section('Excel export')
    frm, to = (dt.date.today() - dt.timedelta(days=13)).isoformat(), dt.date.today().isoformat()
    code, ctype, data = raw_bytes(admin, f'/api/reports/summary.xlsx?from={frm}&to={to}')
    check('summary.xlsx served as a spreadsheet', code == 200 and 'spreadsheetml' in ctype and data[:2] == b'PK', f'{code} {ctype}')
    z = zipfile.ZipFile(io.BytesIO(data))
    check('zip is intact (CRC ok)', z.testzip() is None)
    try:
        import openpyxl
        wb = openpyxl.load_workbook(io.BytesIO(data))
        check('two sheets: KPI summary + attendance', wb.sheetnames == ['สรุป KPI', 'เวลาทำงาน'], wb.sheetnames)
        ws = wb['สรุป KPI']
        hdr = [c.value for c in ws[1]]
        _, _, csv = raw_bytes(admin, f'/api/reports/summary.csv?from={frm}&to={to}')
        csv_rows = csv.decode('utf-8-sig').strip().split('\n')
        check('same rows as the CSV export', ws.max_row == len(csv_rows) and hdr == csv_rows[0].split(','), f'{ws.max_row} vs {len(csv_rows)}')
        nums = [r[hdr.index('at_desk_hours')] for r in ws.iter_rows(min_row=2, values_only=True)]
        check('numbers are real numbers, Thai text intact', all(isinstance(v, (int, float)) for v in nums if v is not None)
              and any(any(ch >= '\u0e00' and ch <= '\u0e7f' for ch in str(c or '')) for r in ws.iter_rows(min_row=2, values_only=True) for c in r))
        check('header frozen with filter', ws.freeze_panes == 'A2' and ws.auto_filter.ref is not None)
        att = wb['เวลาทำงาน']
        check('attendance sheet has work sessions', att.max_row > 1 and [c.value for c in att[1]][:3] == ['date', 'employee_code', 'name'])
    except ImportError:
        check('openpyxl not installed: structural check only', '[Content_Types].xml' in z.namelist())
    eid = admin.get('/api/employees')[0]['id']
    code, ctype, data = raw_bytes(admin, f'/api/reports/employee/{eid}/xlsx?from={frm}&to={to}')
    check('employee daily xlsx', code == 200 and data[:2] == b'PK')
    code, _, _ = raw_bytes(emp, f'/api/reports/summary.xlsx?from={frm}&to={to}')
    check('employee cannot export the summary', code == 403)
    code, _, _ = raw_bytes(emp, f'/api/reports/employee/{eid}/xlsx')
    other = eid != emp.get('/api/auth/me')['employee_id']
    check('employee cannot export someone else', code == 403 or not other)

    section('Report settings')
    old = admin.get('/api/settings')
    code, _ = admin.req('PUT', '/api/settings', {'report_doc_prefix': 'bad prefix!'})
    check('invalid document prefix rejected', code == 400)
    code, _ = admin.req('PUT', '/api/settings', {'ticket_sla_hours': {'low': 1, 'normal': 2}})
    check('incomplete SLA rejected', code == 400)
    admin.put('/api/settings', {'report_doc_prefix': 'TST-KPI', 'report_show_activity': False,
                                'ticket_sla_hours': {'low': 100, 'normal': 50, 'high': 10, 'urgent': 2}}, expect=200)
    _, _, html = raw_bytes(admin, f'/api/reports/employee/{eid}?from={frm}&to={to}')
    html = html.decode()
    check('A4 report uses the document prefix', 'TST-KPI-25' in html)
    check('activity section can be hidden', 'สรุปกิจกรรมที่ระบบตรวจพบ' not in html and '3. แนวโน้มการทำงานรายวัน' in html)
    t = admin.post('/api/tickets', {'subject': 'TEST-SLA', 'priority': 'urgent'}, expect=200)
    due = dt.datetime.fromisoformat(t['due_at'].replace('Z', '+00:00'))
    check('ticket SLA follows the setting (urgent = 2 h)', abs((due - dt.datetime.now(dt.timezone.utc)).total_seconds() - 7200) < 300, t['due_at'])
    admin.delete(f"/api/tickets/{t['id']}")
    admin.put('/api/settings', {'report_doc_prefix': old.get('report_doc_prefix', 'RPT'), 'report_show_activity': old.get('report_show_activity', True),
                                'ticket_sla_hours': old.get('ticket_sla_hours') or {'low': 72, 'normal': 48, 'high': 24, 'urgent': 4}}, expect=200)
    _, _, html = raw_bytes(admin, f'/api/reports/employee/{eid}?from={frm}&to={to}')
    check('activity section back after restoring', 'สรุปกิจกรรมที่ระบบตรวจพบ' in html.decode())

    section('Branch filters')
    stamp = int(time.time())
    b = admin.post('/api/branches', {'name': f'TEST-สาขา-{stamp}'}, expect=200)
    d = admin.post('/api/departments', {'name': f'TEST-แผนก-{stamp}', 'branch_id': b['id']}, expect=200)
    e = admin.post('/api/employees', {'employee_code': f'TB{stamp % 100000}', 'first_name': 'ทดสอบ', 'last_name': 'สาขา', 'department_id': d['id']}, expect=200)
    k = admin.get(f"/api/kpi/scores?from={frm}&to={to}&branch_id={b['id']}")
    check('KPI scores filtered by branch', [r['employee_id'] for r in k['rows']] == [e['id']], [r['employee_id'] for r in k['rows']])
    allk = admin.get(f'/api/kpi/scores?from={frm}&to={to}')
    check('no branch = everyone', len(allk['rows']) > 1)
    sm = admin.get(f"/api/activity/summary?from={frm}&to={to}&branch_id={b['id']}")
    check('activity summary filtered by branch', all(r['employee_id'] == e['id'] for r in sm['rows']))
    tl = admin.get(f"/api/activity/timeline?branch_id={b['id']}")
    check('timeline filtered by branch', all(r['employee_id'] == e['id'] for r in tl['employees']))
    _, _, data = raw_bytes(admin, f"/api/reports/summary.csv?from={frm}&to={to}&branch_id={b['id']}")
    check('CSV export filtered by branch', len(data.decode('utf-8-sig').strip().split('\n')) <= 2)
    admin.delete(f"/api/employees/{e['id']}"); admin.delete(f"/api/departments/{d['id']}"); admin.delete(f"/api/branches/{b['id']}")


# ---------------------------------------------------------------- ERP: payroll, expense claims, journal

def test_payroll(base, admin, hr, emp):
    import io, zipfile
    section('ERP: accounting core')
    accs = admin.get('/api/accounting/accounts')
    role = {a['system_role']: a['id'] for a in accs if a['system_role']}
    check('chart of accounts seeded with system accounts', {'cash', 'bank', 'salary_expense', 'sso_payable', 'wht_payable', 'vat_input'} <= set(role))
    code, _ = emp.req('GET', '/api/accounting/accounts')
    check('employee cannot read the chart of accounts', code == 403)
    today = dt.date.today()
    code, body = admin.req('POST', '/api/accounting/journal', {'entry_date': str(today), 'memo': 'TEST unbalanced',
        'lines': [{'account_id': role['cash'], 'debit': 100}, {'account_id': role['capital'], 'credit': 90}]})
    check('unbalanced manual entry rejected', code == 400, body)
    je = admin.post('/api/accounting/journal', {'entry_date': str(today), 'memo': 'TEST ทุนเริ่มต้น',
        'lines': [{'account_id': role['bank'], 'debit': 1000000}, {'account_id': role['capital'], 'credit': 1000000}]}, expect=200)
    d = admin.get(f"/api/accounting/journal/{je['id']}")
    check('manual entry posted with running number', d['entry_no'].startswith('JV25') and len(d['lines']) == 2)
    # closed period
    admin.put('/api/accounting/settings', {'accounting_lock_date': str(today)}, expect=200)
    code, body = admin.req('POST', '/api/accounting/journal', {'entry_date': str(today), 'memo': 'TEST locked',
        'lines': [{'account_id': role['cash'], 'debit': 1}, {'account_id': role['bank'], 'credit': 1}]})
    check('posting into a closed period refused', code == 400 and 'ปิดแล้ว' in str(body), body)
    code, _ = admin.req('POST', f"/api/accounting/journal/{je['id']}/void")
    check('voiding inside a closed period refused', code == 400)
    admin.put('/api/accounting/settings', {'accounting_lock_date': ''}, expect=200)
    admin.post(f"/api/accounting/journal/{je['id']}/void", expect=200)

    section('ERP: payroll setup')
    emps = admin.get('/api/payroll/employees')
    e1 = next(e for e in emps if e['employee_code'] == 'EMP001')
    e2 = next(e for e in emps if e['employee_code'] != 'EMP001')
    code, _ = emp.req('GET', '/api/payroll/employees')
    check('employee cannot see salaries', code == 403)
    code, _ = admin.req('PUT', f"/api/payroll/employees/{e1['employee_id']}", {'pay_type': 'monthly', 'base_salary': 50000, 'daily_rate': 0,
        'fixed_allowance': 0, 'tax_id': '123', 'sso_enabled': True, 'pvd_rate': 0, 'pvd_employer_rate': 0, 'tax_allowance_annual': 0})
    check('invalid tax id rejected', code == 400)
    hr.put(f"/api/payroll/employees/{e1['employee_id']}", {'pay_type': 'monthly', 'base_salary': 50000, 'daily_rate': 0, 'fixed_allowance': 2000,
        'tax_id': '1101700012345', 'bank_name': 'กสิกรไทย', 'bank_account': '0123456789', 'sso_enabled': True, 'pvd_rate': 3,
        'pvd_employer_rate': 3, 'tax_allowance_annual': 0}, expect=200)
    hr.put(f"/api/payroll/employees/{e2['employee_id']}", {'pay_type': 'daily', 'base_salary': 0, 'daily_rate': 500, 'fixed_allowance': 0,
        'tax_id': '1101700012346', 'sso_enabled': True, 'pvd_rate': 0, 'pvd_employer_rate': 0, 'tax_allowance_annual': 0}, expect=200)

    section('ERP: expense claim')
    cats = emp.get('/api/expense-categories')
    claim = emp.post('/api/expenses', {'title': 'TEST เดินทางพบลูกค้า', 'pay_method': 'payroll', 'items': [
        {'expense_date': str(today), 'category_id': cats[0]['id'], 'description': 'ค่าน้ำมัน', 'amount': 1070, 'vat_amount': 70},
        {'expense_date': str(today), 'category_id': cats[1]['id'], 'description': 'ค่าอาหารลูกค้า', 'amount': 430.5}]}, expect=200)
    check('claim created with total', claim['claim_no'].startswith('EX') and abs(claim['total'] - 1500.5) < 0.001, claim)
    boundary = 'x' + str(int(time.time()))
    body = (f'--{boundary}\r\nContent-Disposition: form-data; name="entity"\r\n\r\nexpense\r\n'
            f'--{boundary}\r\nContent-Disposition: form-data; name="entity_id"\r\n\r\n{claim["id"]}\r\n'
            f'--{boundary}\r\nContent-Disposition: form-data; name="file"; filename="receipt.pdf"\r\nContent-Type: application/pdf\r\n\r\n%PDF-1.4 test\r\n'
            f'--{boundary}--\r\n').encode()
    code, _, up = raw(emp, 'POST', '/api/documents', body, f'multipart/form-data; boundary={boundary}')
    check('employee attaches a receipt to own claim', code == 200, up)
    code, _ = hr.req('GET', f"/api/expenses/{claim['id']}")
    check('approver cannot open a colleague\'s draft', code == 404)
    code, _ = hr.req('POST', f"/api/expenses/{claim['id']}/decide", {'approve': True})
    check('draft cannot be approved', code in (400, 404))
    emp.post(f"/api/expenses/{claim['id']}/submit", expect=200)
    code, _ = emp.req('POST', f"/api/expenses/{claim['id']}/decide", {'approve': True})
    check('employee cannot approve own claim', code == 403)
    code, _ = hr.req('POST', f"/api/expenses/{claim['id']}/decide", {'approve': False})
    check('rejection needs a reason', code == 400)
    det = hr.get(f"/api/expenses/{claim['id']}")
    check('approver sees items and receipt', len(det['items']) == 2 and len(det['receipts']) == 1 and det['can_decide'])
    code, _, _ = raw_bytes(hr, f"/api/documents/{det['receipts'][0]['id']}/download")
    check('approver can download the receipt', code == 200)
    r = hr.post(f"/api/expenses/{claim['id']}/decide", {'approve': True, 'note': 'ok'}, expect=200)
    lines = admin.get(f"/api/accounting/journal/{r['journal_entry_id']}")['lines']
    by = {l['account_code']: l for l in lines}
    check('approval posts expense net of VAT, input VAT and employee payable',
          abs(by['1400']['debit'] - 70) < .001 and abs(by['2340']['credit'] - 1500.5) < .001 and abs(sum(l['debit'] for l in lines) - 1500.5) < .001, lines)

    section('ERP: payroll run')
    y, m = today.year, today.month
    for old in admin.get('/api/payroll/runs'):
        if old['period_year'] == y and old['period_month'] == m and old['status'] != 'void':
            admin.post(f"/api/payroll/runs/{old['id']}/void", expect=200)
    run = hr.post('/api/payroll/runs', {'year': y, 'month': m}, expect=200)
    code, _ = hr.req('POST', '/api/payroll/runs', {'year': y, 'month': m})
    check('one active run per month', code == 409)
    rd = hr.get(f"/api/payroll/runs/{run['id']}")
    s1 = next(x for x in rd['payslips'] if x['employee_id'] == e1['employee_id'])
    s2 = next(x for x in rd['payslips'] if x['employee_id'] == e2['employee_id'])
    check('monthly slip: SSO 875 (ceiling), PVD 3%', s1['sso_employee'] == 875 and s1['pvd_employee'] > 0, s1)
    check('net = earnings - deductions', abs(s1['net_pay'] - (s1['total_earnings'] - s1['total_deductions'])) < .001)
    slip = hr.get(f"/api/payroll/payslips/{s1['id']}")
    codes = {i['code']: i for i in slip['items']}
    check('reimbursement on the payslip, not taxable', 'reimbursement' in codes and not codes['reimbursement']['taxable'] and abs(codes['reimbursement']['amount'] - 1500.5) < .001)
    check('daily-paid slip from days worked', s2['pay_type'] == 'daily' and s2['total_earnings'] >= 0)
    hr.post(f"/api/payroll/payslips/{s1['id']}/items", {'kind': 'earning', 'code': 'bonus', 'label': 'TEST โบนัส', 'amount': 100000}, expect=200)
    s1b = hr.get(f"/api/payroll/payslips/{s1['id']}")['payslip']
    check('bonus raises withholding tax (taxed on the difference)', s1b['wht'] > s1['wht'] + 5000, (s1['wht'], s1b['wht']))
    check('bonus excluded from social security', s1b['sso_employee'] == 875)
    code, _ = emp.req('GET', f"/api/payroll/payslips/{s1['id']}")
    check('employee cannot see the payslip before approval', code == 404)
    check('my payslips empty before approval', all(p['id'] != s1['id'] for p in emp.get('/api/me/payslips')))
    hr.post(f"/api/payroll/runs/{run['id']}/approve", expect=200)
    code, _ = hr.req('POST', f"/api/payroll/payslips/{s1['id']}/items", {'kind': 'earning', 'code': 'other', 'label': 'x', 'amount': 1})
    check('approved run is locked', code == 400)
    mine = emp.get('/api/me/payslips')
    check('employee sees own payslip after approval', any(p['id'] == s1['id'] for p in mine))
    code, _ = emp.req('GET', f"/api/payroll/payslips/{s2['id']}")
    check('employee cannot open a colleague\'s payslip', code == 404)
    _, _, html = raw_bytes(emp, f"/api/payroll/payslips/{s1['id']}/print")
    html = html.decode()
    check('printable payslip with amount in Thai words', 'ใบแจ้งเงินเดือน' in html and 'บาท' in html and 'xxx-6789' in html)
    code, ctype, data = raw_bytes(hr, f"/api/payroll/runs/{run['id']}/pnd1.xlsx")
    check('ภ.ง.ด.1 export', code == 200 and data[:2] == b'PK')
    code, _, data = raw_bytes(hr, f"/api/payroll/runs/{run['id']}/sso.xlsx")
    check('สปส.1-10 export', code == 200 and zipfile.ZipFile(io.BytesIO(data)).testzip() is None)
    code, _, data = raw_bytes(hr, f"/api/payroll/runs/{run['id']}/bank.csv")
    check('bank transfer file', code == 200 and '0123456789' in data.decode('utf-8-sig'))
    paid = hr.post(f"/api/payroll/runs/{run['id']}/pay", {}, expect=200)
    jl = admin.get(f"/api/accounting/journal/{paid['journal_entry_id']}")['lines']
    dr, cr = sum(l['debit'] for l in jl), sum(l['credit'] for l in jl)
    rd = hr.get(f"/api/payroll/runs/{run['id']}")
    net = sum(x['net_pay'] for x in rd['payslips'])
    by = {}
    for l in jl:
        by.setdefault(l['account_code'], 0)
        by[l['account_code']] += l['credit'] - l['debit']
    check('payroll entry balanced', abs(dr - cr) < .001 and dr > 0, (dr, cr))
    check('bank credited with total net pay', abs(by.get('1110', 0) - net) < .01, (by.get('1110'), net))
    check('SSO payable = employee + employer share', abs(by.get('2310', 0) - 2 * rd['totals']['sso_employee']) < .01)
    reimb = sum(i['amount'] for x in rd['payslips'] for i in hr.get(f"/api/payroll/payslips/{x['id']}")['items'] if i['code'] == 'reimbursement')
    check('employee payable cleared by the payroll (all reimbursed claims)', reimb >= 1500.5 and abs(by.get('2340', 0) + reimb) < .01, (by.get('2340'), reimb))
    c = emp.get(f"/api/expenses/{claim['id']}")['claim']
    check('claim marked paid with the payroll', c['status'] == 'paid' and c['payroll_run_no'] == run['run_no'])
    tb = admin.get(f'/api/accounting/trial-balance?from={today.replace(day=1)}&to={today}')
    check('trial balance balances', abs(tb['totals']['debit'] - tb['totals']['credit']) < .01 and abs(tb['totals']['closing_debit'] - tb['totals']['closing_credit']) < .01, tb['totals'])
    # undo: void returns the claim to approved and removes the entry
    hr.post(f"/api/payroll/runs/{run['id']}/void", expect=200)
    check('voided payroll voids its journal entry', admin.get(f"/api/accounting/journal/{paid['journal_entry_id']}")['status'] == 'void')
    check('claim back to approved after void', emp.get(f"/api/expenses/{claim['id']}")['claim']['status'] == 'approved')
    p = admin.post(f"/api/expenses/{claim['id']}/pay", {}, expect=200)
    check('finance pays the claim directly', emp.get(f"/api/expenses/{claim['id']}")['claim']['status'] == 'paid' and p['journal_entry_id'])


# ---------------------------------------------------------------- ERP: products, stock, sales documents

def test_sales(base, admin, hr, emp):
    stamp = int(time.time())
    today = dt.date.today()
    section('ERP: products and stock')
    code, _ = emp.req('GET', '/api/products')
    check('employee cannot see products', code == 403)
    p = admin.post('/api/products', {'sku': f'TS-{stamp}', 'name': 'TEST เครื่องพิมพ์', 'kind': 'product', 'unit': 'เครื่อง', 'sale_price': 5000,
                                    'purchase_price': 3000, 'opening_qty': 10, 'opening_cost': 3000}, expect=200)
    svc = admin.post('/api/products', {'sku': f'TV-{stamp}', 'name': 'TEST ค่าติดตั้ง', 'kind': 'service', 'sale_price': 1000}, expect=200)
    card = admin.get(f"/api/products/{p['id']}/stock")
    check('opening stock 10 at cost 3,000', card['product']['on_hand'] == 10 and card['product']['avg_cost'] == 3000 and len(card['moves']) == 1)
    code, _ = admin.req('POST', '/api/products', {'sku': f'TS-{stamp}', 'name': 'dup', 'kind': 'product'})
    check('duplicate SKU rejected', code == 409)

    section('ERP: customer tax details')
    code, _ = admin.req('POST', '/api/customers', {'name': 'x', 'tax_id': '12345'})
    check('invalid customer tax id rejected', code == 400)
    c = admin.post('/api/customers', {'name': f'TEST ลูกค้า {stamp}', 'company': f'บริษัท ทดสอบ {stamp} จำกัด', 'tax_id': '0105555012345',
                                      'branch_code': '00000', 'address': '1 ถนนสุขุมวิท กรุงเทพฯ 10110', 'credit_days': 15, 'status': 'active'}, expect=200)

    section('ERP: quotation -> invoice -> receipt')
    q = admin.post('/api/sales/docs', {'doc_type': 'quotation', 'customer_id': c['id'], 'vat_mode': 'exclusive', 'wht_rate': 3, 'lines': [
        {'product_id': p['id'], 'description': 'เครื่องพิมพ์', 'qty': 2, 'unit_price': 5000},
        {'product_id': svc['id'], 'description': 'ค่าติดตั้ง', 'qty': 1, 'unit_price': 1000, 'discount_pct': 10}]}, expect=200)
    check('quotation numbered QT with totals (10,900 + VAT 7%)', q['doc_no'].startswith('QT') and abs(q['total'] - 11663) < .001, q)
    code, _ = emp.req('GET', f"/api/sales/docs/{q['id']}")
    check('employee cannot read sales documents', code == 403)
    inv = admin.post(f"/api/sales/docs/{q['id']}/convert", expect=200)
    d = admin.get(f"/api/sales/docs/{inv['id']}")
    check('converted to a draft invoice without a number yet', d['doc']['status'] == 'draft' and d['doc']['doc_no'] is None and len(d['lines']) == 2)
    check('due date from customer credit term (15 days)', d['doc']['due_date'] == str(today + dt.timedelta(days=15)))
    check('buyer tax details on the document', d['doc']['customer_tax_id'] == '0105555012345' and d['doc']['customer_branch'] == 'สำนักงานใหญ่')
    code, _, _ = raw_bytes(admin, f"/api/sales/docs/{inv['id']}/print")
    check('draft cannot be printed as a tax invoice', code == 400)
    iss = admin.post(f"/api/sales/docs/{inv['id']}/issue", expect=200)
    check('issued with IV number', iss['doc_no'].startswith('IV'))
    check('quotation marked converted', admin.get(f"/api/sales/docs/{q['id']}")['doc']['status'] == 'converted')
    lines = admin.get(f"/api/accounting/journal/{iss['journal_entry_id']}")['lines']
    by = {}
    for l in lines:
        by[l['account_code']] = by.get(l['account_code'], 0) + l['debit'] - l['credit']
    check('posting: AR 11,663 / sales 10,000 / service 900 / output VAT 763', abs(by['1200'] - 11663) < .001 and abs(by['4100'] + 10000) < .001
          and abs(by['4200'] + 900) < .001 and abs(by['2200'] + 763) < .001, by)
    check('cost of goods sold at average cost (2 x 3,000)', abs(by['5100'] - 6000) < .001 and abs(by['1300'] + 6000) < .001)
    check('stock issued (10 -> 8)', admin.get(f"/api/products/{p['id']}/stock")['product']['on_hand'] == 8)
    _, _, html = raw_bytes(admin, f"/api/sales/docs/{inv['id']}/print")
    html = html.decode()
    check('tax invoice print: title, original/copy, amount in words', 'ใบกำกับภาษี' in html and 'ต้นฉบับ' in html and 'สำเนา' in html and 'หนึ่งหมื่นหนึ่งพันหกร้อยหกสิบสามบาทถ้วน' in html)
    code, _ = admin.req('POST', f"/api/sales/docs/{inv['id']}/payments", {'amount': 20000})
    check('over-payment refused', code == 400)
    # customer pays 5,000 + later the rest, withholding 3% (327) on the second payment
    p1 = admin.post(f"/api/sales/docs/{inv['id']}/payments", {'amount': 5000, 'method': 'transfer', 'reference': 'TEST-TRF-1'}, expect=200)
    check('partial payment -> receipt RC, status partial', p1['receipt_no'].startswith('RC') and admin.get(f"/api/sales/docs/{inv['id']}")['doc']['status'] == 'partial')
    code, _ = admin.req('POST', f"/api/sales/docs/{inv['id']}/void")
    check('invoice with payments cannot be voided', code == 400)
    p2 = admin.post(f"/api/sales/docs/{inv['id']}/payments", {'amount': 6336, 'wht_amount': 327, 'method': 'transfer'}, expect=200)
    d = admin.get(f"/api/sales/docs/{inv['id']}")
    check('fully paid incl. tax withheld', d['doc']['status'] == 'paid' and abs(d['doc']['paid_amount'] - 11663) < .001 and len(d['payments']) == 2)
    rl = admin.get(f"/api/sales/docs/{p2['receipt_id']}")
    jl = admin.get(f"/api/accounting/journal/{rl['doc']['journal_entry_id']}")['lines']
    by = {l['account_code']: l['debit'] - l['credit'] for l in jl}
    check('receipt posting: bank 6,336 + WHT receivable 327 / AR 6,663', abs(by['1110'] - 6336) < .001 and abs(by['1210'] - 327) < .001 and abs(by['1200'] + 6663) < .001, by)
    _, _, rh = raw_bytes(admin, f"/api/sales/docs/{p2['receipt_id']}/print")
    check('receipt print shows the invoice it settles', 'ใบเสร็จรับเงิน' in rh.decode() and iss['doc_no'] in rh.decode())
    # undo the second payment: back to partial
    pay_id = next(x['id'] for x in d['payments'] if x['receipt_id'] == p2['receipt_id'])
    admin.post(f"/api/sales/payments/{pay_id}/void", expect=200)
    d = admin.get(f"/api/sales/docs/{inv['id']}")
    check('voiding a payment re-opens the invoice', d['doc']['status'] == 'partial' and abs(d['doc']['paid_amount'] - 5000) < .001)
    check('voided receipt journal is void', admin.get(f"/api/accounting/journal/{rl['doc']['journal_entry_id']}")['status'] == 'void')

    section('ERP: inclusive VAT, stock limits, void')
    code, body = admin.req('POST', '/api/sales/docs', {'doc_type': 'invoice', 'customer_id': c['id'], 'lines': [
        {'product_id': p['id'], 'description': 'เกินสต็อก', 'qty': 50, 'unit_price': 5000}]})
    big = body['id']
    code, body = admin.req('POST', f'/api/sales/docs/{big}/issue')
    check('cannot sell more than in stock', code == 400 and 'ไม่พอ' in str(body), body)
    check('failed issue leaves the draft unnumbered', admin.get(f'/api/sales/docs/{big}')['doc']['doc_no'] is None)
    admin.delete(f'/api/sales/docs/{big}', expect=200)
    i2 = admin.post('/api/sales/docs', {'doc_type': 'invoice', 'customer_id': c['id'], 'vat_mode': 'inclusive', 'lines': [
        {'product_id': p['id'], 'description': 'ราคารวม VAT', 'qty': 1, 'unit_price': 5350}]}, expect=200)
    check('inclusive VAT total', abs(i2['total'] - 5350) < .001)
    r2 = admin.post(f"/api/sales/docs/{i2['id']}/issue", expect=200)
    by = {l['account_code']: l['debit'] - l['credit'] for l in admin.get(f"/api/accounting/journal/{r2['journal_entry_id']}")['lines'] if l['account_code'] in ('4100', '2200')}
    check('inclusive: revenue 5,000 + VAT 350', abs(by['4100'] + 5000) < .001 and abs(by['2200'] + 350) < .001, by)
    check('stock 8 -> 7', admin.get(f"/api/products/{p['id']}/stock")['product']['on_hand'] == 7)
    admin.post(f"/api/sales/docs/{i2['id']}/void", expect=200)
    check('void returns stock (7 -> 8) and voids the posting', admin.get(f"/api/products/{p['id']}/stock")['product']['on_hand'] == 8
          and admin.get(f"/api/accounting/journal/{r2['journal_entry_id']}")['status'] == 'void')
    v = admin.get(f"/api/sales/docs/{i2['id']}")['doc']
    check('voided invoice keeps its number', v['status'] == 'void' and v['doc_no'] == r2['doc_no'])

    section('ERP: stock count and transfer')
    wh2 = admin.post('/api/warehouses', {'code': f'W{stamp % 100000}', 'name': 'TEST คลังสาขา'}, expect=200)
    main = next(w for w in admin.get('/api/warehouses') if w['is_default'])
    admin.post('/api/inventory/transfer', {'product_id': p['id'], 'from_warehouse_id': main['id'], 'to_warehouse_id': wh2['id'], 'qty': 3}, expect=200)
    bal = {b['warehouse_id']: b['qty'] for b in admin.get(f"/api/products/{p['id']}/stock")['balances']}
    check('transfer moves stock between warehouses', bal[main['id']] == 5 and bal[wh2['id']] == 3)
    adj = admin.post('/api/inventory/adjust', {'product_id': p['id'], 'warehouse_id': wh2['id'], 'counted_qty': 2, 'note': 'TEST นับได้ 2'}, expect=200)
    check('stock count books the loss at average cost', adj['difference'] == -1 and abs(adj['value'] + 3000) < .001, adj)
    low = admin.get('/api/inventory/stock')
    check('stock report with value', any(i['id'] == p['id'] and i['qty'] == 7 and abs(i['value'] - 21000) < .01 for i in low['items']))
    s = admin.get('/api/sales/summary')
    check('sales summary: receivable includes the open invoice', s['receivable'] >= 6663 - .001)
    tb = admin.get(f'/api/accounting/trial-balance?from={today.replace(day=1)}&to={today}')
    check('trial balance still balances', abs(tb['totals']['closing_debit'] - tb['totals']['closing_credit']) < .01, tb['totals'])


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--url', default='http://localhost:8080')
    ap.add_argument('--password', default='admin12345')
    ap.add_argument('--only', default='')
    a = ap.parse_args()
    admin = login(a.url, 'admin', a.password)
    hr = login(a.url, 'hr', 'hrpass123')
    emp = login(a.url, 'emp001', 'emp001pass')
    tests = {'hr': lambda: test_hr(a.url, admin, hr, emp), 'crm': lambda: test_crm(a.url, admin, hr, emp), 'vms': lambda: test_vms(a.url, admin, hr, emp),
             'security': lambda: test_security(a.url, admin, hr, emp),
             'reports': lambda: test_reports(a.url, admin, hr, emp),
             'payroll': lambda: test_payroll(a.url, admin, hr, emp),
             'sales': lambda: test_sales(a.url, admin, hr, emp)}
    for name, fn in tests.items():
        if a.only and name not in a.only.split(','):
            continue
        try:
            fn()
        except AssertionError as e:
            check(f'{name}: unexpected response', False, str(e))
    print(f'\n{len(PASS)} passed, {len(FAIL)} failed')
    sys.exit(1 if FAIL else 0)


if __name__ == '__main__':
    main()
