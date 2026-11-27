// ERP pages: payroll, expense claims, accounting (journal, trial balance, chart of accounts, settings)
// and the employee's own payslips / expense claims.
import { get, post, put, del, qs, download, openHtml, session } from '../api.js';
import {
  esc, icon, table, toast, showError, modal, formModal, confirmBox, crudPage, fmtDate, today, monthStart, rangePicker, renderFields, readFields,
} from '../ui.js';

export const money = (n) => Number(n || 0).toLocaleString('th-TH', { minimumFractionDigits: 2, maximumFractionDigits: 2 });
const TH_MONTHS = ['ม.ค.', 'ก.พ.', 'มี.ค.', 'เม.ย.', 'พ.ค.', 'มิ.ย.', 'ก.ค.', 'ส.ค.', 'ก.ย.', 'ต.ค.', 'พ.ย.', 'ธ.ค.'];
const period = (y, m) => `${TH_MONTHS[m - 1]} ${y + 543}`;
const RUN_STATUS = {
  draft: '<span class="badge">ฉบับร่าง</span>', approved: '<span class="badge info">อนุมัติแล้ว</span>',
  paid: '<span class="badge ok">จ่ายแล้ว</span>', void: '<span class="badge warn">ยกเลิก</span>',
};
export const CLAIM_STATUS = {
  draft: '<span class="badge">ฉบับร่าง</span>', submitted: '<span class="badge warn">รออนุมัติ</span>', approved: '<span class="badge info">อนุมัติแล้ว</span>',
  rejected: '<span class="badge danger">ไม่อนุมัติ</span>', paid: '<span class="badge ok">จ่ายแล้ว</span>',
};
const tabs = (items, active) => `<div class="tabs">${items.map(([k, l]) => `<button data-tab="${k}" class="${k === active ? 'active' : ''}">${l}</button>`).join('')}</div>`;

function tabbed(el, items, render) {
  el.querySelector('.tabs').addEventListener('click', (e) => {
    const b = e.target.closest('[data-tab]'); if (!b) return;
    el.querySelectorAll('.tabs button').forEach((x) => x.classList.toggle('active', x === b));
    render(b.dataset.tab);
  });
  render(items[0][0]);
}

/** <select> of cash / bank accounts for payments. */
async function payAccountField() {
  const accs = (await get('/accounting/accounts')).filter((a) => a.type === 'asset' && a.is_active && /^11/.test(a.code));
  return { key: 'pay_account_id', label: 'จ่ายจากบัญชี', type: 'select', required: true, numeric: true,
    options: accs.map((a) => ({ value: a.id, label: `${a.code} ${a.name}` })), default: accs.find((a) => a.system_role === 'bank')?.id };
}

// ---------------------------------------------------------------- payroll

export async function payroll(el) {
  const canManage = session.can('payroll.manage');
  el.innerHTML = `<div class="page-head"><div><h1>เงินเดือน (Payroll)</h1><p>คำนวณจากเวลาทำงานจริง: ขาด ลาไม่รับค่าจ้าง OT วันหยุด · ประกันสังคม ภาษีหัก ณ ที่จ่าย กองทุนสำรองเลี้ยงชีพ · ลงบัญชีอัตโนมัติ</p></div></div>
    ${tabs([['runs', 'รอบเงินเดือน'], ['setup', 'ข้อมูลเงินเดือนพนักงาน'], ['settings', 'ตั้งค่าการคำนวณ']], 'runs')}<div data-body></div>`;
  const body = el.querySelector('[data-body]');
  tabbed(el, [['runs']], (t) => (t === 'runs' ? runs(body) : t === 'setup' ? setup(body) : paySettings(body)).catch(showError));

  async function runs(b) {
    const rows = await get('/payroll/runs');
    b.innerHTML = `<div class="toolbar" style="padding:0 0 12px">${canManage ? `<button class="btn primary" data-new>${icon('plus')} สร้างรอบเงินเดือน</button>` : ''}</div>
      <div class="panel">${table([
        { key: 'run_no', label: 'เลขที่' }, { label: 'งวด', render: (r) => period(r.period_year, r.period_month) },
        { label: 'วันจ่าย', render: (r) => fmtDate(r.pay_date) }, { key: 'employees', label: 'พนักงาน', align: 'r' },
        { label: 'รายได้รวม', align: 'r', render: (r) => money(r.total_earnings) }, { label: 'รายการหัก', align: 'r', hideSm: true, render: (r) => money(r.total_deductions) },
        { label: 'จ่ายสุทธิ', align: 'r', render: (r) => `<b>${money(r.total_net)}</b>` }, { label: 'สถานะ', render: (r) => RUN_STATUS[r.status] },
      ], rows, { empty: 'ยังไม่มีรอบเงินเดือน', actions: (r) => `<button class="btn sm" data-open="${r.id}">เปิด</button>` })}</div>`;
    b.querySelector('[data-new]')?.addEventListener('click', () => {
      const d = new Date();
      formModal({
        title: 'สร้างรอบเงินเดือน',
        intro: '<p class="small muted" style="margin-top:0">ระบบจะคำนวณทุกคนที่ตั้งค่าเงินเดือนแล้ว จากเวลาทำงาน คำขอลา/OT ที่อนุมัติ และใบเบิกที่รอจ่ายพร้อมเงินเดือน</p>',
        fields: [
          { key: 'year', label: 'ปี (ค.ศ.)', type: 'number', required: true, default: d.getFullYear(), min: 2000, max: 2200 },
          { key: 'month', label: 'เดือน', type: 'select', required: true, numeric: true, default: d.getMonth() + 1, options: TH_MONTHS.map((m, i) => ({ value: i + 1, label: m })) },
          { key: 'pay_date', label: 'วันที่จ่าย (ว่าง = สิ้นเดือน)', type: 'date' },
        ],
        submitLabel: 'สร้างและคำนวณ',
        onSubmit: async (v) => { const r = await post('/payroll/runs', v); toast(`สร้าง ${r.run_no} แล้ว (${r.employees} คน)`); runs(b); openRun(r.id, () => runs(b)); },
      });
    });
    b.querySelectorAll('[data-open]').forEach((x) => x.addEventListener('click', () => openRun(x.dataset.open, () => runs(b))));
  }

  async function openRun(id, onChange) {
    const m = modal({ title: 'รอบเงินเดือน', body: '<div class="empty">กำลังโหลด...</div>', wide: true });
    const draw = async () => {
      const d = await get(`/payroll/runs/${id}`);
      const r = d.run;
      m.el.querySelector('.modal-head h2').textContent = `${r.run_no} · งวด ${period(r.period_year, r.period_month)}`;
      const act = (k, label, cls = '') => `<button class="btn ${cls}" data-do="${k}">${label}</button>`;
      m.body.innerHTML = `<div class="grid g-4" style="margin-bottom:14px">
          <div class="panel stat"><div><b>${money(r.total_earnings)}</b><span>รายได้รวม</span></div></div>
          <div class="panel stat"><div><b>${money(r.total_deductions)}</b><span>รายการหักรวม</span></div></div>
          <div class="panel stat ok"><div><b>${money(r.total_net)}</b><span>จ่ายสุทธิ ${r.employees} คน</span></div></div>
          <div class="panel stat"><div><b>${money(r.total_employer)}</b><span>นายจ้างสมทบ (ปกส. + กองทุน)</span></div></div></div>
        <div class="row wrap" style="margin-bottom:12px">${RUN_STATUS[r.status]} <span class="small muted">จ่ายวันที่ ${fmtDate(r.pay_date)} · ภาษีหัก ณ ที่จ่าย ${money(d.totals.wht)} · ประกันสังคม ${money(d.totals.sso_employee)} + ${money(d.totals.sso_employer)}</span>
          <span class="spacer"></span>
          ${canManage && r.status === 'draft' ? act('compute', `${icon('refresh')} คำนวณใหม่`) + act('approve', 'อนุมัติ', 'primary') : ''}
          ${canManage && r.status === 'approved' ? act('pay', 'บันทึกจ่ายเงิน + ลงบัญชี', 'primary') : ''}
          ${r.status !== 'void' ? act('bank', `${icon('download')} ไฟล์โอนธนาคาร`) + act('pnd1', `${icon('download')} ภ.ง.ด.1`) + act('sso', `${icon('download')} สปส.1-10`) : ''}
          ${canManage && r.status !== 'void' ? act('void', 'ยกเลิกรอบ', 'danger') : ''}</div>
        ${d.unconfigured.length ? `<p class="notice">ยังไม่ได้ตั้งค่าเงินเดือน ${d.unconfigured.length} คน: ${d.unconfigured.slice(0, 8).map((e) => esc(e.name)).join(', ')}${d.unconfigured.length > 8 ? ' ...' : ''} (แท็บ "ข้อมูลเงินเดือนพนักงาน")</p>` : ''}
        ${table([
          { key: 'employee_code', label: 'รหัส' }, { key: 'name', label: 'ชื่อ' },
          { label: 'วันทำงาน / ขาด / ลาไม่รับ', hideSm: true, render: (s) => `${s.work_days} / ${s.absent_days} / ${s.unpaid_leave_days}` },
          { label: 'OT ชม.', align: 'r', hideSm: true, render: (s) => ((s.ot_min + s.holiday_min) / 60).toFixed(1) },
          { label: 'รายได้', align: 'r', render: (s) => money(s.total_earnings) },
          { label: 'ปกส.', align: 'r', hideSm: true, render: (s) => money(s.sso_employee) },
          { label: 'ภาษี', align: 'r', hideSm: true, render: (s) => money(s.wht) },
          { label: 'หักรวม', align: 'r', render: (s) => money(s.total_deductions) },
          { label: 'สุทธิ', align: 'r', render: (s) => `<b>${money(s.net_pay)}</b>` },
        ], d.payslips, { empty: 'ไม่มีพนักงานในรอบนี้', actions: (s) => `<button class="btn sm" data-slip="${s.id}">รายละเอียด</button><button class="btn sm ghost" data-print="${s.id}" title="พิมพ์สลิป">${icon('print')}</button>` })}`;
      m.body.querySelectorAll('[data-print]').forEach((x) => x.addEventListener('click', () => openHtml(`/payroll/payslips/${x.dataset.print}/print`).catch(showError)));
      m.body.querySelectorAll('[data-slip]').forEach((x) => x.addEventListener('click', () => slipDetail(x.dataset.slip, r.status === 'draft' && canManage, draw)));
      m.body.querySelectorAll('[data-do]').forEach((x) => x.addEventListener('click', async () => {
        const k = x.dataset.do;
        try {
          if (k === 'compute') { await post(`/payroll/runs/${id}/compute`); toast('คำนวณใหม่แล้ว'); }
          if (k === 'approve') {
            if (!(await confirmBox('อนุมัติรอบเงินเดือนนี้? พนักงานจะเห็นสลิปของตนเอง และแก้ไขรายการไม่ได้อีก', 'อนุมัติ', false))) return;
            await post(`/payroll/runs/${id}/approve`); toast('อนุมัติแล้ว แจ้งพนักงานแล้ว');
          }
          if (k === 'pay') {
            const f = await payAccountField();
            await new Promise((res) => formModal({ title: 'บันทึกจ่ายเงินเดือน', fields: [f], submitLabel: 'บันทึกจ่ายและลงบัญชี',
              intro: `<p class="small muted" style="margin-top:0">ลงบัญชีเงินเดือน ภาษีหัก ณ ที่จ่ายค้างนำส่ง ประกันสังคมค้างนำส่ง ณ วันที่ ${fmtDate(r.pay_date)}</p>`,
              onSubmit: async (v) => { const p = await post(`/payroll/runs/${id}/pay`, v); toast('บันทึกจ่ายและลงบัญชีแล้ว'); res(p); } }));
          }
          if (k === 'void') {
            if (!(await confirmBox('ยกเลิกรอบเงินเดือนนี้? ถ้าจ่ายแล้ว รายการบัญชีจะถูกยกเลิกด้วย และใบเบิกจะกลับไปรอจ่าย', 'ยกเลิกรอบ'))) return;
            await post(`/payroll/runs/${id}/void`); toast('ยกเลิกแล้ว');
          }
          if (k === 'bank') await download(`/payroll/runs/${id}/bank.csv`, 'bank.csv');
          if (k === 'pnd1') await download(`/payroll/runs/${id}/pnd1.xlsx`, 'pnd1.xlsx');
          if (k === 'sso') await download(`/payroll/runs/${id}/sso.xlsx`, 'sso.xlsx');
          if (['compute', 'approve', 'pay', 'void'].includes(k)) { onChange(); draw(); }
        } catch (e) { showError(e); }
      }));
    };
    draw().catch(showError);
  }

  async function slipDetail(id, editable, after) {
    const d = await get(`/payroll/payslips/${id}`);
    const s = d.payslip;
    const list = (kind) => d.items.filter((i) => i.kind === kind).map((i) => `<tr><td>${esc(i.label)}${i.manual ? ' <span class="badge">เพิ่มเอง</span>' : ''}${i.kind === 'earning' && !i.taxable ? ' <span class="small muted">(ไม่เสียภาษี)</span>' : ''}</td>
      <td class="r">${money(i.amount)}</td>${editable ? `<td class="actions">${i.manual ? `<button class="btn sm ghost danger" data-rm="${i.id}">${icon('trash')}</button>` : ''}</td>` : ''}</tr>`).join('');
    const m = modal({
      title: `${s.employee_code} ${s.name}`,
      wide: true,
      body: `<div class="grid g-2"><div><h3>รายได้</h3><table class="tbl">${list('earning')}<tr><th>รวม</th><th class="r">${money(s.total_earnings)}</th>${editable ? '<th></th>' : ''}</tr></table></div>
        <div><h3>รายการหัก</h3><table class="tbl">${list('deduction')}<tr><th>รวม</th><th class="r">${money(s.total_deductions)}</th>${editable ? '<th></th>' : ''}</tr></table></div></div>
        <p class="row" style="margin-top:14px"><span class="small muted">เงินได้ที่ต้องเสียภาษี ${money(s.taxable_income)} · ฐานประกันสังคม ${money(s.sso_base)} · นายจ้างสมทบ ${money(s.sso_employer + s.pvd_employer)}</span><span class="spacer"></span><b style="font-size:18px">สุทธิ ${money(s.net_pay)}</b></p>`,
      foot: editable ? `<button class="btn" data-add-earn>${icon('plus')} โบนัส / รายได้อื่น</button><button class="btn" data-add-ded>${icon('plus')} รายการหักอื่น</button>` : '',
    });
    const addItem = (kind) => formModal({
      title: kind === 'earning' ? 'เพิ่มรายได้' : 'เพิ่มรายการหัก',
      fields: [
        ...(kind === 'earning' ? [{ key: 'code', label: 'ประเภท', type: 'select', required: true, options: [{ value: 'bonus', label: 'โบนัส (คำนวณภาษีแบบเงินได้ไม่ประจำ)' }, { value: 'other', label: 'รายได้อื่น' }] }] : []),
        { key: 'label', label: 'รายละเอียด', required: true, full: true },
        { key: 'amount', label: 'จำนวนเงิน', type: 'number', step: '0.01', min: 0.01, required: true },
        ...(kind === 'earning' ? [{ key: 'taxable', label: 'ต้องเสียภาษี', type: 'checkbox', default: true }] : []),
      ],
      onSubmit: async (v) => { await post(`/payroll/payslips/${id}/items`, { kind, code: v.code || 'other', ...v }); toast('เพิ่มแล้ว คำนวณภาษีใหม่แล้ว'); m.close(); after(); slipDetail(id, editable, after); },
    });
    m.el.querySelector('[data-add-earn]')?.addEventListener('click', () => addItem('earning'));
    m.el.querySelector('[data-add-ded]')?.addEventListener('click', () => addItem('deduction'));
    m.body.querySelectorAll('[data-rm]').forEach((x) => x.addEventListener('click', async () => {
      try { await del(`/payroll/items/${x.dataset.rm}`); m.close(); after(); slipDetail(id, editable, after); } catch (e) { showError(e); }
    }));
  }

  async function setup(b) {
    await crudPage(b, {
      title: 'ข้อมูลเงินเดือนพนักงาน', desc: 'ข้อมูลลับ: เฉพาะผู้มีสิทธิ์เงินเดือนเท่านั้นที่เห็น', endpoint: '/payroll/employees', canDelete: false, canEdit: canManage, canAdd: false,
      columns: [
        { key: 'employee_code', label: 'รหัส' }, { key: 'name', label: 'ชื่อ' }, { key: 'department', label: 'แผนก', hideSm: true },
        { label: 'รูปแบบ', render: (r) => (r.configured ? (r.pay_type === 'daily' ? 'รายวัน' : 'รายเดือน') : '<span class="badge warn">ยังไม่ตั้งค่า</span>') },
        { label: 'เงินเดือน / ค่าจ้าง', align: 'r', render: (r) => (r.configured ? money(r.pay_type === 'daily' ? r.daily_rate : r.base_salary) : '-') },
        { label: 'ปกส.', render: (r) => (r.configured ? (r.sso_enabled ? '✓' : '-') : '') },
        { label: 'บัญชีธนาคาร', hideSm: true, render: (r) => esc([r.bank_name, r.bank_account].filter(Boolean).join(' ')) },
      ],
      fields: [
        { key: 'pay_type', label: 'รูปแบบการจ่าย', type: 'select', required: true, options: [{ value: 'monthly', label: 'รายเดือน' }, { value: 'daily', label: 'รายวัน' }] },
        { key: 'base_salary', label: 'เงินเดือน (บาท)', type: 'number', step: '0.01', min: 0 },
        { key: 'daily_rate', label: 'ค่าจ้างรายวัน (บาท)', type: 'number', step: '0.01', min: 0 },
        { key: 'fixed_allowance', label: 'ค่าตำแหน่ง / เบี้ยเลี้ยงประจำต่อเดือน', type: 'number', step: '0.01', min: 0 },
        { key: 'tax_id', label: 'เลขประจำตัวผู้เสียภาษี (13 หลัก)' },
        { key: 'bank_name', label: 'ธนาคาร' }, { key: 'bank_account', label: 'เลขที่บัญชี' },
        { key: 'sso_enabled', label: 'เป็นผู้ประกันตน (หักประกันสังคม)', type: 'checkbox', default: true },
        { key: 'pvd_rate', label: 'กองทุนสำรองเลี้ยงชีพ ลูกจ้าง %', type: 'number', step: '0.5', min: 0, max: 15 },
        { key: 'pvd_employer_rate', label: 'กองทุนสำรองเลี้ยงชีพ นายจ้าง %', type: 'number', step: '0.5', min: 0, max: 15 },
        { key: 'tax_allowance_annual', label: 'ค่าลดหย่อนอื่นต่อปี (คู่สมรส บุตร ประกัน ฯลฯ)', type: 'number', step: '0.01', min: 0, full: true, help: 'ไม่รวมค่าลดหย่อนส่วนตัว 60,000 / ค่าใช้จ่าย 50% / ประกันสังคม / กองทุน ซึ่งระบบคำนวณให้' },
      ],
      toForm: (r) => ({ pay_type: 'monthly', sso_enabled: true, base_salary: 0, daily_rate: 0, fixed_allowance: 0, pvd_rate: 0, pvd_employer_rate: 0, tax_allowance_annual: 0, ...Object.fromEntries(Object.entries(r).filter(([, v]) => v !== null)) }),
      toBody: (v) => ({ ...v, base_salary: v.base_salary || 0, daily_rate: v.daily_rate || 0, fixed_allowance: v.fixed_allowance || 0, pvd_rate: v.pvd_rate || 0, pvd_employer_rate: v.pvd_employer_rate || 0, tax_allowance_annual: v.tax_allowance_annual || 0 }),
    });
  }

  async function paySettings(b) {
    const s = await get('/payroll/settings');
    const fields = [
      { key: 'payroll_sso_rate', label: 'อัตราเงินสมทบประกันสังคม (%)', type: 'number', step: '0.01', min: 0, max: 20 },
      { key: 'payroll_sso_min_base', label: 'ฐานค่าจ้างต่ำสุด (บาท)', type: 'number', step: '1', min: 0 },
      { key: 'payroll_sso_max_base', label: 'ฐานค่าจ้างสูงสุด (บาท)', type: 'number', step: '1', min: 0, help: 'ตรวจสอบเพดานปัจจุบันตามกฎกระทรวง (ปี 2569: 17,500)' },
      { key: 'payroll_ot_rate', label: 'ค่าล่วงเวลาวันทำงาน (เท่า)', type: 'number', step: '0.25', min: 1 },
      { key: 'payroll_holiday_rate', label: 'ทำงานในวันหยุด ในเวลาปกติ (เท่า, รายเดือน)', type: 'number', step: '0.25', min: 0, help: 'ลูกจ้างรายวันได้เพิ่มอีก 1 เท่าโดยอัตโนมัติ' },
      { key: 'payroll_holiday_ot_rate', label: 'ล่วงเวลาในวันหยุด (เท่า)', type: 'number', step: '0.25', min: 1 },
      { key: 'payroll_deduct_absence', label: 'หักเงินวันขาดงาน', type: 'checkbox' },
      { key: 'payroll_deduct_late', label: 'หักเงินมาสาย (ตามนาที)', type: 'checkbox' },
    ];
    b.innerHTML = `<form class="panel panel-body" data-form></form>${canManage ? '<div class="row" style="margin-top:12px"><span class="spacer"></span><button class="btn primary" data-save>บันทึก</button></div>' : ''}`;
    const form = b.querySelector('[data-form]');
    form.innerHTML = await renderFields(fields, s);
    b.querySelector('[data-save]')?.addEventListener('click', async () => {
      try { await put('/payroll/settings', readFields(form, fields)); toast('บันทึกแล้ว (มีผลกับการคำนวณครั้งถัดไป)'); } catch (e) { showError(e); }
    });
  }
}

// ---------------------------------------------------------------- expense claims (approvers / finance)

export async function expenses(el) {
  el.innerHTML = `<div class="page-head"><div><h1>ใบเบิกค่าใช้จ่าย</h1><p>อนุมัติใบเบิกของพนักงานในความดูแล · จ่ายพร้อมเงินเดือนหรือจ่ายแยก · ลงบัญชีค่าใช้จ่ายและภาษีซื้ออัตโนมัติ</p></div></div>
    ${tabs([['submitted', 'รออนุมัติ'], ['approved', 'อนุมัติแล้ว (รอจ่าย)'], ['', 'ทั้งหมด'], ['cats', 'หมวดค่าใช้จ่าย']], 'submitted')}<div data-body></div>`;
  const body = el.querySelector('[data-body]');
  let current = 'submitted';
  const load = async (status) => {
    current = status;
    if (status === 'cats') return categories(body);
    const rows = await get(`/expenses${qs({ scope: 'review', status })}`);
    body.innerHTML = `<div class="panel">${table(claimColumns(true), rows, { empty: 'ไม่มีใบเบิก', actions: (r) => `<button class="btn sm" data-open="${r.id}">เปิด</button>` })}</div>`;
    body.querySelectorAll('[data-open]').forEach((x) => x.addEventListener('click', () => claimDetail(x.dataset.open, () => load(current))));
  };
  tabbed(el, [['submitted']], (t) => load(t).catch(showError));
}

function claimColumns(withEmployee) {
  return [
    { key: 'claim_no', label: 'เลขที่' }, ...(withEmployee ? [{ key: 'employee_name', label: 'พนักงาน' }] : []),
    { key: 'title', label: 'เรื่อง' }, { label: 'วันที่', hideSm: true, render: (r) => fmtDate(r.claim_date) },
    { label: 'ยอดรวม', align: 'r', render: (r) => money(r.total) },
    { label: 'รับเงิน', hideSm: true, render: (r) => (r.pay_method === 'payroll' ? 'พร้อมเงินเดือน' : 'จ่ายแยก') },
    { label: 'ใบเสร็จ', align: 'r', hideSm: true, render: (r) => r.receipts },
    { label: 'สถานะ', render: (r) => CLAIM_STATUS[r.status] + (r.payroll_run_no ? ` <span class="small muted">${esc(r.payroll_run_no)}</span>` : '') },
  ];
}

async function categories(b) {
  const accs = (await get('/accounting/accounts')).filter((a) => a.type === 'expense' || a.type === 'asset');
  await crudPage(b, {
    title: 'หมวดค่าใช้จ่าย', endpoint: '/expense-categories', canDelete: false, canEdit: session.can('expenses.manage', 'accounting.manage'), addLabel: 'เพิ่มหมวด',
    columns: [{ key: 'name', label: 'หมวด' }, { label: 'บัญชี', render: (r) => `${esc(r.account_code)} ${esc(r.account_name)}` }, { label: 'ใช้งาน', render: (r) => (r.is_active ? '✓' : '-') }],
    fields: [
      { key: 'name', label: 'ชื่อหมวด', required: true },
      { key: 'account_id', label: 'ลงบัญชี', type: 'select', required: true, numeric: true, options: accs.map((a) => ({ value: a.id, label: `${a.code} ${a.name}` })) },
      { key: 'is_active', label: 'ใช้งาน', type: 'checkbox', default: true },
    ],
  });
}

export async function claimDetail(id, after) {
  const d = await get(`/expenses/${id}`);
  const c = d.claim;
  const mine = c.employee_id === session.user.employee_id;
  const m = modal({
    title: `${c.claim_no} · ${c.title}`, wide: true,
    body: `<div class="row wrap" style="margin-bottom:10px">${CLAIM_STATUS[c.status]} <span class="small muted">${esc(c.employee_name)} · ${esc(c.department || '')} · รับเงิน${c.pay_method === 'payroll' ? 'พร้อมเงินเดือน' : 'แยก'}</span></div>
      ${table([
        { label: 'วันที่', render: (i) => fmtDate(i.expense_date) }, { key: 'category_name', label: 'หมวด' }, { key: 'description', label: 'รายละเอียด' },
        { label: 'ภาษีซื้อ', align: 'r', render: (i) => (i.vat_amount ? money(i.vat_amount) : '-') }, { label: 'จำนวนเงิน', align: 'r', render: (i) => money(i.amount) },
      ], d.items)}
      <p class="row"><span class="spacer"></span><b style="font-size:17px">รวม ${money(c.total)} บาท</b></p>
      <h3>ใบเสร็จ / หลักฐาน</h3>
      <div data-receipts>${d.receipts.length ? d.receipts.map((r) => `<button class="btn sm" data-dl="${r.id}">${icon('download')} ${esc(r.file_name)}</button>`).join(' ') : '<span class="muted small">ไม่มีไฟล์แนบ</span>'}</div>
      ${mine && ['draft', 'submitted'].includes(c.status) ? '<div class="row" style="margin-top:8px"><input type="file" data-file accept=".pdf,.jpg,.jpeg,.png,.heic,.webp"><button class="btn sm" data-up>แนบไฟล์</button></div>' : ''}
      ${c.approver_note ? `<p class="notice">หมายเหตุผู้อนุมัติ (${esc(c.approver_name || '')}): ${esc(c.approver_note)}</p>` : ''}`,
    foot: [
      mine && c.status === 'draft' ? '<button class="btn danger" data-act="delete">ลบ</button><button class="btn" data-act="edit">แก้ไข</button><button class="btn primary" data-act="submit">ส่งขออนุมัติ</button>' : '',
      d.can_decide ? '<button class="btn danger" data-act="reject">ไม่อนุมัติ</button><button class="btn primary" data-act="approve">อนุมัติ</button>' : '',
      c.status === 'approved' && !c.payroll_run_no && session.can('expenses.manage') ? '<button class="btn primary" data-act="pay">บันทึกจ่ายเงิน</button>' : '',
    ].join(''),
  });
  m.body.querySelectorAll('[data-dl]').forEach((x) => x.addEventListener('click', () => download(`/documents/${x.dataset.dl}/download`, 'receipt').catch(showError)));
  m.body.querySelector('[data-up]')?.addEventListener('click', async () => {
    const f = m.body.querySelector('[data-file]').files[0];
    if (!f) { toast('กรุณาเลือกไฟล์', 'warn'); return; }
    const fd = new FormData(); fd.append('entity', 'expense'); fd.append('entity_id', String(id)); fd.append('title', `ใบเสร็จ ${c.claim_no}`); fd.append('file', f);
    try {
      const res = await fetch('/api/documents', { method: 'POST', headers: { Authorization: `Bearer ${session.token}` }, body: fd });
      if (!res.ok) throw new Error((await res.json().catch(() => ({}))).error || 'อัปโหลดไม่สำเร็จ');
      toast('แนบไฟล์แล้ว'); m.close(); claimDetail(id, after);
    } catch (e) { showError(e); }
  });
  m.el.querySelectorAll('[data-act]').forEach((x) => x.addEventListener('click', async () => {
    const a = x.dataset.act;
    try {
      if (a === 'submit') { await post(`/expenses/${id}/submit`); toast('ส่งขออนุมัติแล้ว'); }
      if (a === 'delete') { if (!(await confirmBox('ลบใบเบิกนี้?', 'ลบ'))) return; await del(`/expenses/${id}`); toast('ลบแล้ว'); }
      if (a === 'edit') { m.close(); claimForm(d, after); return; }
      if (a === 'approve' || a === 'reject') {
        const approve = a === 'approve';
        await new Promise((res) => formModal({
          title: approve ? 'อนุมัติใบเบิก' : 'ไม่อนุมัติใบเบิก', submitLabel: approve ? 'อนุมัติ' : 'ยืนยันไม่อนุมัติ',
          fields: [{ key: 'note', label: approve ? 'หมายเหตุ (ถ้ามี)' : 'เหตุผล', type: 'textarea', required: !approve }],
          onSubmit: async (v) => { await post(`/expenses/${id}/decide`, { approve, ...v }); toast(approve ? 'อนุมัติและลงบัญชีแล้ว' : 'บันทึกแล้ว'); res(); },
        }));
      }
      if (a === 'pay') {
        const f = await payAccountField();
        await new Promise((res) => formModal({ title: 'จ่ายเงินใบเบิก', fields: [f], submitLabel: 'บันทึกจ่าย',
          onSubmit: async (v) => { await post(`/expenses/${id}/pay`, v); toast('บันทึกจ่ายแล้ว'); res(); } }));
      }
      m.close(); after?.();
    } catch (e) { showError(e); }
  }));
}

/** Create / edit an expense claim with any number of lines. */
export async function claimForm(existing, after) {
  const cats = (await get('/expense-categories')).filter((c) => c.is_active);
  const items = existing ? existing.items.map((i) => ({ ...i })) : [{ expense_date: today(), category_id: cats[0]?.id, description: '', amount: '', vat_amount: '' }];
  const c = existing?.claim;
  const m = modal({
    title: c ? `แก้ไข ${c.claim_no}` : 'เบิกค่าใช้จ่าย', wide: true,
    body: `<form data-f><div class="form-grid"><div class="field full"><label for="x-title">เรื่อง *</label><input class="input" id="x-title" name="title" required value="${esc(c?.title || '')}" placeholder="เช่น เดินทางพบลูกค้า บจก. ตัวอย่าง"></div>
      <div class="field"><label for="x-pm">รับเงิน</label><select class="input" id="x-pm" name="pay_method"><option value="payroll">พร้อมเงินเดือน</option><option value="cash" ${c?.pay_method === 'cash' ? 'selected' : ''}>จ่ายแยก (โอน/เงินสด)</option></select></div></div>
      <div class="tbl-wrap" style="margin-top:12px"><table class="tbl"><thead><tr><th>วันที่</th><th>หมวด</th><th>รายละเอียด</th><th class="r">จำนวนเงิน</th><th class="r">ภาษีซื้อ (ถ้ามีใบกำกับ)</th><th></th></tr></thead><tbody data-rows></tbody></table></div>
      <div class="row" style="margin-top:8px"><button type="button" class="btn sm" data-addrow>${icon('plus')} เพิ่มรายการ</button><span class="spacer"></span><b data-total></b></div></form>`,
    foot: '<button class="btn" data-close>ยกเลิก</button><button class="btn primary" data-save>บันทึกฉบับร่าง</button>',
  });
  const rowsEl = m.body.querySelector('[data-rows]');
  const draw = () => {
    rowsEl.innerHTML = items.map((i, n) => `<tr data-n="${n}">
      <td><input class="input" type="date" data-k="expense_date" value="${esc(i.expense_date)}" max="${today()}" required></td>
      <td><select class="input" data-k="category_id">${cats.map((ct) => `<option value="${ct.id}" ${ct.id === Number(i.category_id) ? 'selected' : ''}>${esc(ct.name)}</option>`).join('')}</select></td>
      <td><input class="input" data-k="description" value="${esc(i.description)}" required></td>
      <td><input class="input r" type="number" step="0.01" min="0.01" data-k="amount" value="${esc(i.amount)}" required style="width:110px"></td>
      <td><input class="input r" type="number" step="0.01" min="0" data-k="vat_amount" value="${esc(i.vat_amount || '')}" style="width:110px"></td>
      <td class="actions">${items.length > 1 ? `<button type="button" class="btn sm ghost danger" data-del="${n}">${icon('trash')}</button>` : ''}</td></tr>`).join('');
    m.body.querySelector('[data-total]').textContent = `รวม ${money(items.reduce((s, i) => s + (Number(i.amount) || 0), 0))} บาท`;
  };
  rowsEl.addEventListener('input', (e) => {
    const n = e.target.closest('tr')?.dataset.n; const k = e.target.dataset.k;
    if (n === undefined || !k) return;
    items[n][k] = e.target.value;
    if (k === 'amount') m.body.querySelector('[data-total]').textContent = `รวม ${money(items.reduce((s, i) => s + (Number(i.amount) || 0), 0))} บาท`;
  });
  rowsEl.addEventListener('click', (e) => { const b = e.target.closest('[data-del]'); if (b) { items.splice(Number(b.dataset.del), 1); draw(); } });
  m.body.querySelector('[data-addrow]').addEventListener('click', () => { items.push({ expense_date: today(), category_id: cats[0]?.id, description: '', amount: '', vat_amount: '' }); draw(); });
  m.el.querySelector('[data-save]').addEventListener('click', async () => {
    const f = m.body.querySelector('[data-f]');
    if (!f.reportValidity()) return;
    const payload = {
      title: f.title.value, pay_method: f.pay_method.value,
      items: items.map((i) => ({ expense_date: i.expense_date, category_id: Number(i.category_id), description: i.description, amount: Number(i.amount), vat_amount: Number(i.vat_amount) || 0 })),
    };
    try {
      const r = c ? await put(`/expenses/${c.id}`, payload) : await post('/expenses', payload);
      toast('บันทึกแล้ว แนบใบเสร็จและกด "ส่งขออนุมัติ"');
      m.close(); after?.();
      claimDetail(c ? c.id : r.id, after);
    } catch (e) { showError(e); }
  });
  draw();
}

// ---------------------------------------------------------------- employee self-service

export async function myPayslips(el) {
  el.innerHTML = '<div class="page-head"><div><h1>สลิปเงินเดือน</h1><p>สลิปที่อนุมัติแล้ว · เปิดเพื่อพิมพ์หรือบันทึกเป็น PDF</p></div></div><div class="panel" data-list></div>';
  const rows = await get('/me/payslips');
  const list = el.querySelector('[data-list]');
  list.innerHTML = table([
    { label: 'งวด', render: (r) => period(r.period_year, r.period_month) }, { label: 'วันจ่าย', render: (r) => fmtDate(r.pay_date) },
    { label: 'รายได้', align: 'r', render: (r) => money(r.total_earnings) }, { label: 'รายการหัก', align: 'r', render: (r) => money(r.total_deductions) },
    { label: 'สุทธิ', align: 'r', render: (r) => `<b>${money(r.net_pay)}</b>` },
  ], rows, { empty: 'ยังไม่มีสลิปเงินเดือน', actions: (r) => `<button class="btn sm" data-print="${r.id}">${icon('print')} เปิดสลิป</button>` });
  list.addEventListener('click', (e) => { const b = e.target.closest('[data-print]'); if (b) openHtml(`/payroll/payslips/${b.dataset.print}/print`).catch(showError); });
}

export async function myExpenses(el) {
  el.innerHTML = `<div class="page-head"><div><h1>เบิกค่าใช้จ่าย</h1><p>สร้างใบเบิก แนบใบเสร็จ แล้วส่งให้หัวหน้าอนุมัติ</p></div><span class="spacer"></span>
    <button class="btn primary" data-new>${icon('plus')} สร้างใบเบิก</button></div><div class="panel" data-list></div>`;
  const list = el.querySelector('[data-list]');
  const load = async () => {
    const rows = await get('/expenses?scope=mine');
    list.innerHTML = table(claimColumns(false), rows, { empty: 'ยังไม่มีใบเบิก', actions: (r) => `<button class="btn sm" data-open="${r.id}">เปิด</button>` });
  };
  list.addEventListener('click', (e) => { const b = e.target.closest('[data-open]'); if (b) claimDetail(b.dataset.open, load).catch(showError); });
  el.querySelector('[data-new]').addEventListener('click', () => claimForm(null, load).catch(showError));
  await load();
}

// ---------------------------------------------------------------- accounting

export async function accounting(el) {
  const canManage = session.can('accounting.manage');
  el.innerHTML = `<div class="page-head"><div><h1>บัญชี</h1><p>สมุดรายวัน (ลงอัตโนมัติจากเงินเดือน ใบเบิก การขาย การซื้อ ทรัพย์สิน) · งบทดลอง · ผังบัญชี · ปิดงวด</p></div></div>
    ${tabs([['journal', 'สมุดรายวัน'], ['tb', 'งบทดลอง'], ['coa', 'ผังบัญชี'], ['settings', 'ข้อมูลกิจการ / ปิดงวด']], 'journal')}<div data-body></div>`;
  const body = el.querySelector('[data-body]');
  tabbed(el, [['journal']], (t) => ({ journal, tb: trialBalance, coa, settings }[t](body)).catch(showError));

  async function journal(b) {
    b.innerHTML = `<div class="toolbar" style="padding:0 0 12px"><div data-range></div><span class="spacer"></span>${canManage ? `<button class="btn primary" data-new>${icon('plus')} บันทึกรายการ</button>` : ''}</div><div class="panel" data-list></div>`;
    const range = rangePicker(b.querySelector('[data-range]'), () => load(), { from: monthStart(), to: today() });
    const load = async () => {
      const { from, to } = range.get();
      const rows = await get(`/accounting/journal${qs({ from, to })}`);
      b.querySelector('[data-list]').innerHTML = table([
        { key: 'entry_no', label: 'เลขที่' }, { label: 'วันที่', render: (r) => fmtDate(r.entry_date) }, { key: 'memo', label: 'คำอธิบาย' },
        { label: 'ที่มา', hideSm: true, render: (r) => esc(SOURCES[r.source] || r.source) }, { label: 'จำนวนเงิน', align: 'r', render: (r) => money(r.total) },
        { label: '', render: (r) => (r.status === 'void' ? '<span class="badge warn">ยกเลิก</span>' : '') },
      ], rows, { empty: 'ไม่มีรายการในช่วงนี้', actions: (r) => `<button class="btn sm" data-open="${r.id}">ดู</button>` });
      b.querySelectorAll('[data-open]').forEach((x) => x.addEventListener('click', () => entryDetail(x.dataset.open, load)));
    };
    b.querySelector('[data-new]')?.addEventListener('click', () => manualEntry(load));
    await load();
  }

  async function entryDetail(id, after) {
    const e = await get(`/accounting/journal/${id}`);
    const dr = e.lines.reduce((s, l) => s + l.debit, 0);
    const m = modal({
      title: `${e.entry_no} · ${fmtDate(e.entry_date)}`, wide: true,
      body: `<p>${esc(e.memo || '')} <span class="small muted">· ${esc(SOURCES[e.source] || e.source)}</span> ${e.status === 'void' ? '<span class="badge warn">ยกเลิก</span>' : ''}</p>
        ${table([{ label: 'บัญชี', render: (l) => `${esc(l.account_code)} ${esc(l.account_name)}` }, { key: 'memo', label: 'รายละเอียด' },
          { label: 'เดบิต', align: 'r', render: (l) => (l.debit ? money(l.debit) : '') }, { label: 'เครดิต', align: 'r', render: (l) => (l.credit ? money(l.credit) : '') }], e.lines)}
        <p class="row"><span class="spacer"></span><b>รวม ${money(dr)}</b></p>`,
      foot: canManage && e.source === 'manual' && e.status === 'posted' ? '<button class="btn danger" data-void>ยกเลิกรายการ</button>' : '',
    });
    m.el.querySelector('[data-void]')?.addEventListener('click', async () => {
      if (!(await confirmBox('ยกเลิกรายการบัญชีนี้?', 'ยกเลิกรายการ'))) return;
      try { await post(`/accounting/journal/${id}/void`); toast('ยกเลิกแล้ว'); m.close(); after(); } catch (err) { showError(err); }
    });
  }

  async function manualEntry(after) {
    const accs = (await get('/accounting/accounts')).filter((a) => a.is_active);
    const lines = [{ account_id: '', debit: '', credit: '' }, { account_id: '', debit: '', credit: '' }];
    const opts = accs.map((a) => `<option value="${a.id}">${esc(a.code)} ${esc(a.name)}</option>`).join('');
    const m = modal({
      title: 'บันทึกรายการบัญชี (Journal Voucher)', wide: true,
      body: `<form data-f><div class="form-grid"><div class="field"><label for="j-date">วันที่ *</label><input class="input" type="date" id="j-date" name="entry_date" value="${today()}" required></div>
        <div class="field"><label for="j-memo">คำอธิบาย *</label><input class="input" id="j-memo" name="memo" required></div></div>
        <div class="tbl-wrap" style="margin-top:12px"><table class="tbl"><thead><tr><th>บัญชี</th><th class="r">เดบิต</th><th class="r">เครดิต</th><th></th></tr></thead><tbody data-rows></tbody></table></div>
        <div class="row" style="margin-top:8px"><button type="button" class="btn sm" data-add>${icon('plus')} เพิ่มบรรทัด</button><span class="spacer"></span><b data-sum></b></div></form>`,
      foot: '<button class="btn" data-close>ยกเลิก</button><button class="btn primary" data-save>บันทึก</button>',
    });
    const rowsEl = m.body.querySelector('[data-rows]');
    const sum = () => {
      const dr = lines.reduce((s, l) => s + (Number(l.debit) || 0), 0); const cr = lines.reduce((s, l) => s + (Number(l.credit) || 0), 0);
      const el2 = m.body.querySelector('[data-sum]');
      el2.textContent = `เดบิต ${money(dr)} · เครดิต ${money(cr)}${Math.abs(dr - cr) > 0.004 ? ' · ยังไม่สมดุล' : ' · สมดุล'}`;
      el2.style.color = Math.abs(dr - cr) > 0.004 ? 'var(--danger, #e5484d)' : '';
    };
    const draw = () => {
      rowsEl.innerHTML = lines.map((l, n) => `<tr data-n="${n}"><td><select class="input" data-k="account_id" required><option value="">- เลือกบัญชี -</option>${opts}</select></td>
        <td><input class="input r" type="number" step="0.01" min="0" data-k="debit" value="${esc(l.debit)}" style="width:130px"></td>
        <td><input class="input r" type="number" step="0.01" min="0" data-k="credit" value="${esc(l.credit)}" style="width:130px"></td>
        <td class="actions">${lines.length > 2 ? `<button type="button" class="btn sm ghost danger" data-del="${n}">${icon('trash')}</button>` : ''}</td></tr>`).join('');
      rowsEl.querySelectorAll('tr').forEach((tr) => { tr.querySelector('select').value = lines[tr.dataset.n].account_id; });
      sum();
    };
    rowsEl.addEventListener('input', (e) => { const n = e.target.closest('tr')?.dataset.n; if (n !== undefined && e.target.dataset.k) { lines[n][e.target.dataset.k] = e.target.value; sum(); } });
    rowsEl.addEventListener('change', (e) => { const n = e.target.closest('tr')?.dataset.n; if (n !== undefined && e.target.dataset.k) lines[n][e.target.dataset.k] = e.target.value; });
    rowsEl.addEventListener('click', (e) => { const b = e.target.closest('[data-del]'); if (b) { lines.splice(Number(b.dataset.del), 1); draw(); } });
    m.body.querySelector('[data-add]').addEventListener('click', () => { lines.push({ account_id: '', debit: '', credit: '' }); draw(); });
    m.el.querySelector('[data-save]').addEventListener('click', async () => {
      const f = m.body.querySelector('[data-f]');
      if (!f.reportValidity()) return;
      try {
        await post('/accounting/journal', { entry_date: f.entry_date.value, memo: f.memo.value,
          lines: lines.map((l) => ({ account_id: Number(l.account_id), debit: Number(l.debit) || 0, credit: Number(l.credit) || 0 })) });
        toast('บันทึกรายการแล้ว'); m.close(); after();
      } catch (e) { showError(e); }
    });
    draw();
  }

  async function trialBalance(b) {
    b.innerHTML = '<div class="toolbar" style="padding:0 0 12px"><div data-range></div></div><div class="panel" data-list></div>';
    const range = rangePicker(b.querySelector('[data-range]'), () => load(), { from: `${new Date().getFullYear()}-01-01`, to: today() });
    const load = async () => {
      const { from, to } = range.get();
      const d = await get(`/accounting/trial-balance${qs({ from, to })}`);
      b.querySelector('[data-list]').innerHTML = `${table([
        { key: 'code', label: 'รหัส' }, { key: 'name', label: 'ชื่อบัญชี' },
        { label: 'ยกมา', align: 'r', hideSm: true, render: (r) => money(r.opening) },
        { label: 'เดบิต', align: 'r', render: (r) => money(r.debit) }, { label: 'เครดิต', align: 'r', render: (r) => money(r.credit) },
        { label: 'คงเหลือ เดบิต', align: 'r', render: (r) => (r.closing_debit ? money(r.closing_debit) : '') },
        { label: 'คงเหลือ เครดิต', align: 'r', render: (r) => (r.closing_credit ? money(r.closing_credit) : '') },
      ], d.rows, { empty: 'ยังไม่มีรายการบัญชี' })}
      <div class="row" style="padding:10px 16px"><span class="spacer"></span><b>รวม เดบิต ${money(d.totals.debit)} · เครดิต ${money(d.totals.credit)} · คงเหลือ ${money(d.totals.closing_debit)} / ${money(d.totals.closing_credit)}</b></div>`;
    };
    await load();
  }

  async function coa(b) {
    const TYPES = { asset: 'สินทรัพย์', liability: 'หนี้สิน', equity: 'ส่วนของเจ้าของ', revenue: 'รายได้', expense: 'ค่าใช้จ่าย' };
    await crudPage(b, {
      title: 'ผังบัญชี', endpoint: '/accounting/accounts', canDelete: false, canEdit: canManage, addLabel: 'เพิ่มบัญชี',
      columns: [{ key: 'code', label: 'รหัส' }, { key: 'name', label: 'ชื่อบัญชี' }, { label: 'ประเภท', render: (r) => TYPES[r.type] },
        { label: 'ใช้ลงบัญชีอัตโนมัติ', hideSm: true, render: (r) => (r.system_role ? `<span class="badge info">${esc(r.system_role)}</span>` : '') },
        { label: 'ใช้งาน', render: (r) => (r.is_active ? '✓' : '-') }],
      fields: [
        { key: 'code', label: 'รหัสบัญชี', required: true }, { key: 'name', label: 'ชื่อบัญชี', required: true },
        { key: 'type', label: 'ประเภท', type: 'select', required: true, options: Object.entries(TYPES).map(([value, label]) => ({ value, label })) },
        { key: 'is_active', label: 'ใช้งาน', type: 'checkbox', default: true },
      ],
    });
  }

  async function settings(b) {
    const s = await get('/accounting/settings');
    const fields = [
      { key: 'company_tax_id', label: 'เลขประจำตัวผู้เสียภาษีของกิจการ (13 หลัก)' },
      { key: 'company_branch', label: 'สาขา (เช่น สำนักงานใหญ่ / สาขาที่ 00001)' },
      { key: 'company_address', label: 'ที่อยู่ (แสดงบนเอกสาร)', type: 'textarea' },
      { key: 'accounting_lock_date', label: 'ปิดงวดบัญชีถึงวันที่', type: 'date', help: 'ห้ามบันทึก / แก้ไข / ยกเลิกรายการที่ลงวันที่ถึงวันนี้ (ว่าง = ไม่ปิด)' },
    ];
    b.innerHTML = `<form class="panel panel-body" data-form></form>${canManage ? '<div class="row" style="margin-top:12px"><span class="spacer"></span><button class="btn primary" data-save>บันทึก</button></div>' : ''}`;
    const form = b.querySelector('[data-form]');
    form.innerHTML = await renderFields(fields, s);
    b.querySelector('[data-save]')?.addEventListener('click', async () => {
      const v = readFields(form, fields);
      for (const k of Object.keys(v)) if (v[k] === null) v[k] = '';
      try { await put('/accounting/settings', v); toast('บันทึกแล้ว'); } catch (e) { showError(e); }
    });
  }
}

export const SOURCES = {
  manual: 'บันทึกเอง', payroll: 'เงินเดือน', expense_claim: 'ใบเบิก (อนุมัติ)', expense_payment: 'จ่ายใบเบิก', sales_invoice: 'ขาย / ใบแจ้งหนี้',
  sales_payment: 'รับชำระ', purchase_receipt: 'รับสินค้า / ตั้งหนี้', purchase_payment: 'จ่ายเจ้าหนี้', stock_adjust: 'ปรับปรุงสต็อก',
  depreciation: 'ค่าเสื่อมราคา', asset_disposal: 'จำหน่ายทรัพย์สิน', asset_purchase: 'ซื้อทรัพย์สิน',
};
