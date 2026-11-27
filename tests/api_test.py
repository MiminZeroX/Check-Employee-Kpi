#!/usr/bin/env python3
"""End-to-end API checks against a running server that was filled with scripts/seed_demo.py.

    python3 tests/api_test.py --url http://localhost:8080 --password admin12345

Creates its own test records (prefixed "TEST-") and removes what it can afterwards.
Standard library only.
"""
import argparse, datetime as dt, json, sys, urllib.error, urllib.request

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


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--url', default='http://localhost:8080')
    ap.add_argument('--password', default='admin12345')
    ap.add_argument('--only', default='')
    a = ap.parse_args()
    admin = login(a.url, 'admin', a.password)
    hr = login(a.url, 'hr', 'hrpass123')
    emp = login(a.url, 'emp001', 'emp001pass')
    tests = {'hr': lambda: test_hr(a.url, admin, hr, emp)}
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
