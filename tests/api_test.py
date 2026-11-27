#!/usr/bin/env python3
"""End-to-end API checks against a running server that was filled with scripts/seed_demo.py.

    python3 tests/api_test.py --url http://localhost:8080 --password admin12345

Creates its own test records (prefixed "TEST-") and removes what it can afterwards.
Standard library only.
"""
import argparse, datetime as dt, hashlib, hmac, http.server, json, sys, threading, time, urllib.error, urllib.request

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
    n = admin.post('/api/holidays/import-fixed', {'year': today.year + 1}, expect=200)['added']
    check('fixed Thai holidays imported', n >= 10, n)
    hol = admin.get(f'/api/holidays?year={today.year + 1}')
    check('holiday list for next year', len(hol) >= 10)
    # leave: employee asks, HR approves
    types = emp.get('/api/leave-types')
    sick = next(t for t in types if t['code'] == 'sick')
    bal0 = emp.get('/api/leave-balances')['rows']
    future = today + dt.timedelta(days=(7 - today.weekday()) % 7 + 7)  # a Monday in 1-2 weeks
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
    bal = {b['code']: b for b in emp.get('/api/leave-balances')['rows']}
    before = {b['code']: b for b in bal0}
    check('leave balance reduced', bal['sick']['used'] == before['sick']['used'] + 3, (before['sick'], bal['sick']))
    code, body = emp.req('POST', '/api/requests', {'kind': 'leave', 'leave_type_id': next(t for t in types if t['code'] == 'personal')['id'],
                                                    'start_date': str(future + dt.timedelta(days=7)), 'end_date': str(future + dt.timedelta(days=11)),
                                                    'reason': 'TEST-เกินโควตา'})
    check('leave over quota rejected', code == 400 and 'คงเหลือ' in str(body), body)
    ot = emp.post('/api/requests', {'kind': 'overtime', 'start_date': str(today), 'start_time': '18:00', 'hours': 2, 'reason': 'TEST-ปิดงบ'}, expect=200)
    off = emp.post('/api/requests', {'kind': 'offsite', 'start_date': str(future + dt.timedelta(days=3)), 'location': 'TEST-ลูกค้า สีลม',
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


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--url', default='http://localhost:8080')
    ap.add_argument('--password', default='admin12345')
    ap.add_argument('--only', default='')
    a = ap.parse_args()
    admin = login(a.url, 'admin', a.password)
    hr = login(a.url, 'hr', 'hrpass123')
    emp = login(a.url, 'emp001', 'emp001pass')
    tests = {'hr': lambda: test_hr(a.url, admin, hr, emp), 'crm': lambda: test_crm(a.url, admin, hr, emp)}
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
