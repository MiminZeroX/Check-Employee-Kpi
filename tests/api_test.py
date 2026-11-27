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
    yr = random.randint(2040, 2099)  # a fresh year each run keeps the test repeatable
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
             'security': lambda: test_security(a.url, admin, hr, emp)}
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
