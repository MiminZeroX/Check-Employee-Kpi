#!/usr/bin/env python3
"""Generates docs/ER_DIAGRAM.md (Mermaid erDiagram) from the live database schema.

    python3 scripts/gen_er_diagram.py                       # mysql/mariadb client, database "smartoffice"
    python3 scripts/gen_er_diagram.py --db smartoffice --client "mariadb -uroot"

Reads information_schema only (no data). Tables are grouped by module; one diagram per
module keeps each picture readable. Foreign keys that point to another module are listed
below the diagram instead of drawn.
"""
import argparse, collections, os, shlex, subprocess, sys

GROUPS = [
    ('องค์กรและผู้ใช้', ['organizations', 'branches', 'departments', 'employees', 'users', 'roles', 'role_permissions', 'system_settings', 'audit_logs']),
    ('กล้อง AI และอุปกรณ์', ['cameras', 'workstations', 'devices', 'detection_zones', 'zone_occupancy', 'activity_events', 'data_corrections', 'notifications']),
    ('เวลาทำงานและ HR', ['attendance_logs', 'daily_summaries', 'work_sessions', 'shifts', 'shift_assignments', 'holidays', 'leave_types', 'requests']),
    ('งาน KPI และรายงาน', ['tasks', 'projects', 'kpi_metrics', 'kpi_records', 'reports', 'documents']),
    ('CRM และบริการ', ['customers', 'contacts', 'deals', 'tickets', 'webhooks']),
    ('ERP: เงินเดือนและเบิกจ่าย', ['employee_pay', 'payroll_runs', 'payslips', 'payslip_items', 'expense_categories', 'expense_claims', 'expense_items']),
    ('ERP: ขายและเอกสาร', ['products', 'sales_docs', 'sales_doc_lines', 'sales_payments', 'doc_sequences']),
    ('ERP: จัดซื้อและคลัง', ['suppliers', 'warehouses', 'purchase_orders', 'purchase_order_lines', 'purchase_bills', 'purchase_bill_lines', 'purchase_payments', 'stock_moves', 'stock_balances']),
    ('ERP: บัญชีและทรัพย์สิน', ['accounts', 'journal_entries', 'journal_lines', 'fixed_assets', 'asset_depreciation']),
]


def query(client, db, sql):
    out = subprocess.run(shlex.split(client) + ['--default-character-set=utf8mb4', '-N', '-B', db, '-e', sql],
                         capture_output=True, text=True, check=True).stdout
    return [line.split('\t') for line in out.splitlines() if line]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--db', default='smartoffice')
    ap.add_argument('--client', default=os.environ.get('MYSQL_CLIENT', 'mysql -uroot'))
    ap.add_argument('--out', default=os.path.join(os.path.dirname(__file__), '..', 'docs', 'ER_DIAGRAM.md'))
    a = ap.parse_args()
    cols = collections.defaultdict(list)
    for t, c, typ, key, null in query(a.client, a.db,
            "SELECT TABLE_NAME, COLUMN_NAME, DATA_TYPE, COLUMN_KEY, IS_NULLABLE FROM information_schema.COLUMNS "
            "WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME NOT LIKE '\\_sqlx%' ORDER BY TABLE_NAME, ORDINAL_POSITION"):
        cols[t].append((c, typ, key, null))
    fks = query(a.client, a.db,
                "SELECT TABLE_NAME, COLUMN_NAME, REFERENCED_TABLE_NAME FROM information_schema.KEY_COLUMN_USAGE "
                "WHERE TABLE_SCHEMA = DATABASE() AND REFERENCED_TABLE_NAME IS NOT NULL ORDER BY TABLE_NAME, COLUMN_NAME")
    fk_cols = {(t, c) for t, c, _ in fks}
    known = {t for _, ts in GROUPS for t in ts}
    groups = [(g, [t for t in ts if t in cols]) for g, ts in GROUPS]
    other = sorted(t for t in cols if t not in known)
    if other:
        groups.append(('อื่น ๆ', other))
    where = {t: g for g, ts in groups for t in ts}

    lines = ['# ER Diagram', '',
             f'สร้างอัตโนมัติจากฐานข้อมูลจริงด้วย `scripts/gen_er_diagram.py` · {len(cols)} ตาราง · {len(fks)} foreign key', '',
             'PK = primary key · FK = foreign key · UK = unique · แสดงเฉพาะคีย์และคอลัมน์หลัก (ดูคอลัมน์ทั้งหมดใน `server/migrations/`)', '']
    for g, ts in groups:
        if not ts:
            continue
        lines += [f'## {g}', '', '```mermaid', 'erDiagram']
        for t in ts:
            lines.append(f'  {t} {{')
            shown = hidden = 0
            for c, typ, key, null in cols[t]:
                tag = 'PK' if key == 'PRI' else 'FK' if (t, c) in fk_cols else 'UK' if key == 'UNI' else ''
                # keys always; other columns up to 8 so the boxes stay small
                if not tag and shown >= 8:
                    hidden += 1
                    continue
                shown += 0 if tag else 1
                lines.append(f'    {typ} {c}{" " + tag if tag else ""}')

            lines.append('  }')
        cross = []
        for t, c, ref in fks:
            if where.get(t) != g:
                continue
            if where.get(ref) == g:
                lines.append(f'  {ref} ||--o{{ {t} : "{c}"')
            else:
                cross.append(f'- `{t}.{c}` → `{ref}` ({where.get(ref, "?")})')
        lines += ['```', '']
        if cross:
            lines += ['เชื่อมไปโมดูลอื่น:', ''] + cross + ['']
    with open(a.out, 'w', encoding='utf-8') as f:
        f.write('\n'.join(lines))
    print(f'wrote {a.out}: {len(cols)} tables, {len(fks)} foreign keys')


if __name__ == '__main__':
    sys.exit(main())
