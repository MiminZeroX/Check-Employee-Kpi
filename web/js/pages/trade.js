// ERP trade pages: sales documents, products / services, inventory (purchasing is added to this file too).
import { get, post, put, del, qs, openHtml, session } from '../api.js';
import { esc, icon, table, toast, showError, modal, formModal, confirmBox, crudPage, fmtDate, today, opt } from '../ui.js';
import { money } from './erp.js';

const tabs = (items, active) => `<div class="tabs">${items.map(([k, l]) => `<button data-tab="${k}" class="${k === active ? 'active' : ''}">${l}</button>`).join('')}</div>`;
const num = (v) => Number(v || 0);
const qtyFmt = (n) => Number(n || 0).toLocaleString('th-TH', { maximumFractionDigits: 3 });

export const DOC_LABEL = { quotation: 'ใบเสนอราคา', invoice: 'ใบแจ้งหนี้ / ใบกำกับภาษี', receipt: 'ใบเสร็จรับเงิน' };
const STATUS = {
  open: '<span class="badge">เปิด</span>', accepted: '<span class="badge info">ลูกค้าตอบรับ</span>', rejected: '<span class="badge warn">ปฏิเสธ</span>',
  converted: '<span class="badge ok">ออกใบแจ้งหนี้แล้ว</span>', draft: '<span class="badge">ฉบับร่าง</span>', issued: '<span class="badge info">รอชำระ</span>',
  partial: '<span class="badge warn">ชำระบางส่วน</span>', paid: '<span class="badge ok">ชำระครบ</span>', void: '<span class="badge danger">ยกเลิก</span>',
};
const badge = (type, status) => (type === 'receipt' && status === 'issued' ? '<span class="badge ok">ออกแล้ว</span>' : STATUS[status]);
const VAT_MODES = [{ value: 'exclusive', label: 'ราคาไม่รวม VAT (บวก 7%)' }, { value: 'inclusive', label: 'ราคารวม VAT แล้ว' }, { value: 'none', label: 'ไม่มี VAT' }];
const PAY_METHODS = [{ value: 'transfer', label: 'โอนเงิน' }, { value: 'cash', label: 'เงินสด' }, { value: 'cheque', label: 'เช็ค' }, { value: 'card', label: 'บัตร' }];

/** Same rules as the server (sales.rs `totals`): document discount spread over lines, VAT exclusive / inclusive / none. */
export function calcTotals(lines, discount, vatMode, rate = 7, whtRate = 0) {
  const r2 = (x) => Math.round((x + Number.EPSILON) * 100) / 100;
  const amounts = lines.map((l) => r2(num(l.qty) * num(l.unit_price) * (100 - num(l.discount_pct)) / 100));
  const subtotal = r2(amounts.reduce((s, a) => s + a, 0));
  const disc = Math.min(Math.max(num(discount), 0), subtotal);
  const after = amounts.map((a) => (subtotal ? r2(a - disc * a / subtotal) : a));
  const taxable = after.reduce((s, a, i) => s + (lines[i].vat_exempt ? 0 : a), 0);
  const exempt = after.reduce((s, a, i) => s + (lines[i].vat_exempt ? a : 0), 0);
  let vat = 0; let total = taxable + exempt;
  if (vatMode === 'exclusive') { vat = r2(taxable * rate / 100); total = taxable + exempt + vat; }
  if (vatMode === 'inclusive') vat = r2(taxable * rate / (100 + rate));
  return { subtotal, discount: disc, vat, total: r2(total), wht: r2((total - vat) * whtRate / 100) };
}

/** Line editor shared by sales and purchase documents. `priceKey` = sale_price | purchase_price. */
export function lineEditor(container, lines, products, { priceKey = 'sale_price', onChange, showExempt = true } = {}) {
  const prodOpts = `<option value="">- พิมพ์เอง -</option>${products.filter((p) => p.is_active).map((p) => `<option value="${p.id}">${esc(p.sku)} ${esc(p.name)}</option>`).join('')}`;
  const draw = () => {
    container.innerHTML = `<div class="tbl-wrap"><table class="tbl"><thead><tr><th>สินค้า / บริการ</th><th>รายละเอียด</th><th class="r">จำนวน</th><th>หน่วย</th><th class="r">ราคา/หน่วย</th><th class="r">ส่วนลด %</th>${showExempt ? '<th>ยกเว้น VAT</th>' : ''}<th class="r">จำนวนเงิน</th><th></th></tr></thead><tbody>
      ${lines.map((l, n) => `<tr data-n="${n}">
        <td><select class="input" data-k="product_id" style="min-width:150px">${prodOpts}</select></td>
        <td><input class="input" data-k="description" value="${esc(l.description || '')}" required style="min-width:160px"></td>
        <td><input class="input r" type="number" step="0.001" min="0.001" data-k="qty" value="${esc(l.qty ?? 1)}" required style="width:80px"></td>
        <td><input class="input" data-k="unit" value="${esc(l.unit || '')}" style="width:70px"></td>
        <td><input class="input r" type="number" step="0.01" min="0" data-k="unit_price" value="${esc(l.unit_price ?? 0)}" required style="width:110px"></td>
        <td><input class="input r" type="number" step="0.01" min="0" max="100" data-k="discount_pct" value="${esc(l.discount_pct || 0)}" style="width:70px"></td>
        ${showExempt ? `<td class="c"><input type="checkbox" data-k="vat_exempt" ${l.vat_exempt ? 'checked' : ''}></td>` : ''}
        <td class="r" data-amt>${money(num(l.qty) * num(l.unit_price) * (100 - num(l.discount_pct)) / 100)}</td>
        <td class="actions">${lines.length > 1 ? `<button type="button" class="btn sm ghost danger" data-del="${n}">${icon('trash')}</button>` : ''}</td></tr>`).join('')}
      </tbody></table></div><button type="button" class="btn sm" data-addline style="margin-top:8px">${icon('plus')} เพิ่มบรรทัด</button>`;
    container.querySelectorAll('tr[data-n]').forEach((tr) => { tr.querySelector('[data-k=product_id]').value = lines[tr.dataset.n].product_id || ''; });
    onChange?.();
  };
  container.addEventListener('input', (e) => {
    const tr = e.target.closest('tr[data-n]'); const k = e.target.dataset.k; if (!tr || !k) return;
    const l = lines[tr.dataset.n];
    l[k] = e.target.type === 'checkbox' ? e.target.checked : e.target.value;
    tr.querySelector('[data-amt]').textContent = money(num(l.qty) * num(l.unit_price) * (100 - num(l.discount_pct)) / 100);
    onChange?.();
  });
  container.addEventListener('change', (e) => {
    if (e.target.dataset.k !== 'product_id') return;
    const n = e.target.closest('tr').dataset.n; const p = products.find((x) => String(x.id) === e.target.value);
    lines[n].product_id = p ? p.id : null;
    if (p) Object.assign(lines[n], { description: p.name, unit: p.unit, unit_price: p[priceKey], vat_exempt: !p.vat_applicable });
    draw();
  });
  container.addEventListener('click', (e) => {
    const d = e.target.closest('[data-del]'); if (d) { lines.splice(Number(d.dataset.del), 1); draw(); return; }
    if (e.target.closest('[data-addline]')) { lines.push({ qty: 1, unit_price: 0, discount_pct: 0 }); draw(); }
  });
  draw();
  return () => lines.map((l) => ({ product_id: l.product_id ? Number(l.product_id) : null, description: l.description, qty: num(l.qty), unit: l.unit || null,
    unit_price: num(l.unit_price), discount_pct: num(l.discount_pct), vat_exempt: !!l.vat_exempt }));
}

// ---------------------------------------------------------------- sales

export async function sales(el) {
  const canManage = session.can('sales.manage');
  el.innerHTML = `<div class="page-head"><div><h1>ขายและออกเอกสาร</h1><p>ใบเสนอราคา → ใบแจ้งหนี้ / ใบกำกับภาษี → รับชำระและใบเสร็จ · VAT · หัก ณ ที่จ่าย · ตัดสต็อกและลงบัญชีอัตโนมัติ</p></div><span class="spacer"></span>
      ${canManage ? `<button class="btn" data-new="invoice">${icon('plus')} ใบแจ้งหนี้</button><button class="btn primary" data-new="quotation">${icon('plus')} ใบเสนอราคา</button>` : ''}</div>
    <div class="grid g-4" style="margin-bottom:16px" data-stats></div>
    ${tabs([['quotation', 'ใบเสนอราคา'], ['invoice', 'ใบแจ้งหนี้ / ใบกำกับภาษี'], ['receipt', 'ใบเสร็จรับเงิน']], 'invoice')}
    <div class="panel" data-list></div>`;
  let type = 'invoice';
  const load = async () => {
    const [s, rows] = await Promise.all([get('/sales/summary'), get(`/sales/docs${qs({ doc_type: type })}`)]);
    el.querySelector('[data-stats]').innerHTML = `
      <div class="panel stat"><div><b>${money(s.month_sales)}</b><span>ยอดขายเดือนนี้ (ก่อน VAT)</span></div></div>
      <div class="panel stat"><div><b>${money(s.receivable)}</b><span>ลูกหนี้คงค้าง</span></div></div>
      <div class="panel stat ${s.overdue_count ? 'warn' : ''}"><div><b>${money(s.overdue)}</b><span>เกินกำหนด ${s.overdue_count} ใบ</span></div></div>
      <div class="panel stat"><div><b>${money(s.open_quotation_value)}</b><span>ใบเสนอราคารอปิด ${s.open_quotations} ใบ</span></div></div>`;
    el.querySelector('[data-list]').innerHTML = table([
      { label: 'เลขที่', render: (r) => esc(r.doc_no || '(ร่าง)') }, { label: 'วันที่', render: (r) => fmtDate(r.doc_date) },
      { key: 'customer_name', label: 'ลูกค้า' },
      ...(type === 'receipt' ? [{ label: 'อ้างอิง', render: (r) => esc(r.reference_no || '') }] : [{ label: type === 'quotation' ? 'ยืนราคาถึง' : 'ครบกำหนด', hideSm: true, render: (r) => fmtDate(r.due_date) }]),
      { label: 'ยอดรวม', align: 'r', render: (r) => money(r.total) },
      ...(type === 'invoice' ? [{ label: 'ค้างชำระ', align: 'r', render: (r) => (['issued', 'partial'].includes(r.status) ? `<b>${money(r.total - r.paid_amount)}</b>` : '-') }] : []),
      { label: 'สถานะ', render: (r) => badge(r.doc_type, r.status) + (type === 'invoice' && ['issued', 'partial'].includes(r.status) && r.due_date < today() ? ' <span class="badge danger">เกินกำหนด</span>' : '') },
    ], rows, { empty: 'ยังไม่มีเอกสาร', actions: (r) => `<button class="btn sm" data-open="${r.id}">เปิด</button>` });
  };
  el.querySelector('.tabs').addEventListener('click', (e) => {
    const b = e.target.closest('[data-tab]'); if (!b) return;
    el.querySelectorAll('.tabs button').forEach((x) => x.classList.toggle('active', x === b));
    type = b.dataset.tab; load().catch(showError);
  });
  el.addEventListener('click', (e) => {
    const o = e.target.closest('[data-open]'); if (o) { docDetail(o.dataset.open, load).catch(showError); return; }
    const n = e.target.closest('[data-new]'); if (n) docEditor({ doc_type: n.dataset.new }, load).catch(showError);
  });
  await load();
}

async function docEditor(doc, after) {
  const [customers, products, warehouses] = await Promise.all([get('/customers'), get('/products'), get('/warehouses')]);
  const isNew = !doc.id;
  const lines = doc.lines ? doc.lines.map((l) => ({ ...l })) : [{ qty: 1, unit_price: 0, discount_pct: 0 }];
  const d = doc.doc || { doc_type: doc.doc_type, vat_mode: 'exclusive', discount: 0, wht_rate: 0, doc_date: today() };
  const m = modal({
    title: `${isNew ? 'สร้าง' : 'แก้ไข'}${DOC_LABEL[d.doc_type]}${d.doc_no ? ` ${d.doc_no}` : ''}`, wide: true,
    body: `<form data-f><div class="form-grid">
        <div class="field"><label for="s-cust">ลูกค้า *</label><select class="input" id="s-cust" name="customer_id" required><option value="">- เลือก -</option>
          ${customers.map((c) => `<option value="${c.id}" ${c.id === d.customer_id ? 'selected' : ''}>${esc(c.company || c.name)}${c.tax_id ? '' : ' (ไม่มีเลขผู้เสียภาษี)'}</option>`).join('')}</select></div>
        <div class="field"><label for="s-date">วันที่</label><input class="input" type="date" id="s-date" name="doc_date" value="${esc(d.doc_date)}"></div>
        <div class="field"><label for="s-due">${d.doc_type === 'quotation' ? 'ยืนราคาถึง' : 'ครบกำหนดชำระ'} (ว่าง = ตามเครดิตลูกค้า)</label><input class="input" type="date" id="s-due" name="due_date" value="${esc(d.due_date || '')}"></div>
        <div class="field"><label for="s-vat">ภาษีมูลค่าเพิ่ม</label><select class="input" id="s-vat" name="vat_mode">${VAT_MODES.map((v) => `<option value="${v.value}" ${v.value === d.vat_mode ? 'selected' : ''}>${v.label}</option>`).join('')}</select></div>
        <div class="field"><label for="s-wht">ลูกค้าหัก ณ ที่จ่าย</label><select class="input" id="s-wht" name="wht_rate">${[0, 1, 2, 3, 5].map((r) => `<option value="${r}" ${num(d.wht_rate) === r ? 'selected' : ''}>${r ? `${r}%` : 'ไม่หัก'}</option>`).join('')}</select></div>
        <div class="field"><label for="s-wh">ตัดสต็อกจากคลัง</label><select class="input" id="s-wh" name="warehouse_id">${warehouses.filter((w) => w.is_active).map((w) => `<option value="${w.id}" ${w.id === d.warehouse_id ? 'selected' : ''}>${esc(w.name)}</option>`).join('')}</select></div>
      </div>
      <div data-lines style="margin-top:12px"></div>
      <div class="grid g-2" style="margin-top:12px"><div class="field"><label for="s-note">หมายเหตุบนเอกสาร</label><textarea class="input" id="s-note" name="note">${esc(d.note || '')}</textarea></div>
        <div><div class="field"><label for="s-disc">ส่วนลดท้ายบิล (บาท)</label><input class="input r" type="number" step="0.01" min="0" id="s-disc" name="discount" value="${esc(d.discount || 0)}"></div>
        <table class="tbl" data-sum></table></div></div></form>`,
    foot: '<button class="btn" data-close>ยกเลิก</button><button class="btn primary" data-save>บันทึก</button>',
  });
  const f = m.body.querySelector('[data-f]');
  const sum = () => {
    const t = calcTotals(lines, f.discount.value, f.vat_mode.value, 7, num(f.wht_rate.value));
    m.body.querySelector('[data-sum]').innerHTML = `<tr><td>รวมเงิน</td><td class="r">${money(t.subtotal)}</td></tr><tr><td>ส่วนลด</td><td class="r">${money(t.discount)}</td></tr>
      ${f.vat_mode.value !== 'none' ? `<tr><td>ภาษีมูลค่าเพิ่ม 7%</td><td class="r">${money(t.vat)}</td></tr>` : ''}<tr><td><b>รวมทั้งสิ้น</b></td><td class="r"><b>${money(t.total)}</b></td></tr>
      ${t.wht ? `<tr><td>หัก ณ ที่จ่าย</td><td class="r">${money(t.wht)}</td></tr><tr><td>ลูกค้าชำระสุทธิ</td><td class="r">${money(t.total - t.wht)}</td></tr>` : ''}`;
  };
  const read = lineEditor(m.body.querySelector('[data-lines]'), lines, products, { onChange: sum });
  ['discount', 'vat_mode', 'wht_rate'].forEach((k) => f[k].addEventListener('input', sum));
  m.el.querySelector('[data-save]').addEventListener('click', async () => {
    if (!f.reportValidity()) return;
    const body = { doc_type: d.doc_type, customer_id: Number(f.customer_id.value), doc_date: f.doc_date.value || null, due_date: f.due_date.value || null,
      vat_mode: f.vat_mode.value, wht_rate: num(f.wht_rate.value), warehouse_id: Number(f.warehouse_id.value) || null, discount: num(f.discount.value), note: f.note.value, lines: read() };
    try {
      const r = isNew ? await post('/sales/docs', body) : await put(`/sales/docs/${doc.doc.id}`, body);
      toast('บันทึกแล้ว'); m.close(); after?.(); docDetail(isNew ? r.id : doc.doc.id, after);
    } catch (e) { showError(e); }
  });
}

async function docDetail(id, after) {
  const x = await get(`/sales/docs/${id}`);
  const d = x.doc; const can = session.can('sales.manage');
  const btn = (k, l, cls = '') => `<button class="btn ${cls}" data-act="${k}">${l}</button>`;
  const acts = [];
  if (d.doc_type !== 'invoice' || d.status !== 'draft') acts.push(btn('print', `${icon('print')} พิมพ์`));
  if (can) {
    if (d.doc_type === 'quotation' && ['open', 'accepted'].includes(d.status)) acts.push(btn('edit', 'แก้ไข'), btn('reject', 'ลูกค้าปฏิเสธ'), btn('accept', 'ลูกค้าตอบรับ'), btn('convert', 'ออกใบแจ้งหนี้', 'primary'));
    if (d.doc_type === 'quotation' && d.status === 'rejected') acts.push(btn('reopen', 'เปิดใหม่'));
    if (d.doc_type === 'quotation' && d.status !== 'converted') acts.push(btn('delete', 'ลบ', 'danger'));
    if (d.doc_type === 'invoice' && d.status === 'draft') acts.push(btn('delete', 'ลบ', 'danger'), btn('edit', 'แก้ไข'), btn('issue', 'ออกใบกำกับภาษี + ลงบัญชี', 'primary'));
    if (d.doc_type === 'invoice' && d.status === 'issued') acts.push(btn('void', 'ยกเลิกเอกสาร', 'danger'));
    if (d.doc_type === 'invoice' && ['issued', 'partial'].includes(d.status)) acts.push(btn('pay', 'รับชำระเงิน', 'primary'));
  }
  const m = modal({
    title: `${DOC_LABEL[d.doc_type]} ${d.doc_no || '(ฉบับร่าง)'}`, wide: true,
    body: `<div class="row wrap" style="margin-bottom:10px">${badge(d.doc_type, d.status)} <span class="small muted">${esc(d.customer_name)} ${d.customer_tax_id ? `· เลขผู้เสียภาษี ${esc(d.customer_tax_id)}` : ''} · วันที่ ${fmtDate(d.doc_date)}${d.due_date ? ` · ครบกำหนด ${fmtDate(d.due_date)}` : ''}${d.reference_no ? ` · อ้างอิง ${esc(d.reference_no)}` : ''}</span></div>
      ${d.doc_type === 'receipt' ? `<p>รับชำระ ${money(d.paid_amount)} บาท ${d.wht_amount ? `+ ภาษีหัก ณ ที่จ่าย ${money(d.wht_amount)}` : ''}</p>` : table([
        { key: 'description', label: 'รายการ' }, { label: 'จำนวน', align: 'r', render: (l) => `${qtyFmt(l.qty)} ${esc(l.unit || '')}` },
        { label: 'ราคา/หน่วย', align: 'r', render: (l) => money(l.unit_price) }, { label: 'ส่วนลด', align: 'r', render: (l) => (num(l.discount_pct) ? `${l.discount_pct}%` : '') },
        { label: 'จำนวนเงิน', align: 'r', render: (l) => money(l.amount) },
      ], x.lines)}
      <div class="row" style="margin-top:8px"><span class="spacer"></span><table class="tbl" style="width:auto;min-width:300px">
        ${d.doc_type !== 'receipt' ? `<tr><td>รวมเงิน</td><td class="r">${money(d.subtotal)}</td></tr>${d.discount ? `<tr><td>ส่วนลด</td><td class="r">${money(d.discount)}</td></tr>` : ''}
        ${d.vat_mode !== 'none' ? `<tr><td>VAT ${d.vat_rate}%</td><td class="r">${money(d.vat_amount)}</td></tr>` : ''}` : ''}
        <tr><td><b>รวมทั้งสิ้น</b></td><td class="r"><b>${money(d.total)}</b></td></tr>
        ${d.doc_type === 'invoice' && d.status !== 'draft' ? `<tr><td>ชำระแล้ว</td><td class="r">${money(d.paid_amount)}</td></tr><tr><td><b>คงค้าง</b></td><td class="r"><b>${money(d.total - d.paid_amount)}</b></td></tr>` : ''}</table></div>
      ${x.payments.length ? `<h3>การรับชำระ</h3>${table([
        { label: 'วันที่', render: (p) => fmtDate(p.pay_date) }, { label: 'ใบเสร็จ', render: (p) => esc(p.receipt_no || '') }, { key: 'account_name', label: 'เข้าบัญชี' },
        { label: 'รับเงิน', align: 'r', render: (p) => money(p.amount) }, { label: 'หัก ณ ที่จ่าย', align: 'r', render: (p) => money(p.wht_amount) },
        { label: '', render: (p) => (p.status === 'void' ? STATUS.void : '') },
      ], x.payments, { actions: (p) => (p.status === 'active' ? `<button class="btn sm" data-rc="${p.receipt_id}">${icon('print')}</button>${can ? `<button class="btn sm ghost danger" data-voidpay="${p.id}">ยกเลิก</button>` : ''}` : '') })}` : ''}
      ${x.children.length ? `<p class="small muted">เอกสารที่เกี่ยวข้อง: ${x.children.map((c) => `${esc(c.doc_no || '(ร่าง)')} (${badge(c.doc_type, c.status).replace(/<[^>]+>/g, '')})`).join(', ')}</p>` : ''}`,
    foot: acts.join(''),
  });
  const done = (msg) => { toast(msg); m.close(); after?.(); };
  m.body.querySelectorAll('[data-rc]').forEach((b) => b.addEventListener('click', () => openHtml(`/sales/docs/${b.dataset.rc}/print`).catch(showError)));
  m.body.querySelectorAll('[data-voidpay]').forEach((b) => b.addEventListener('click', async () => {
    if (!(await confirmBox('ยกเลิกการรับชำระและใบเสร็จนี้?', 'ยกเลิก'))) return;
    try { await post(`/sales/payments/${b.dataset.voidpay}/void`); done('ยกเลิกแล้ว'); docDetail(id, after); } catch (e) { showError(e); }
  }));
  m.el.querySelectorAll('[data-act]').forEach((b) => b.addEventListener('click', async () => {
    const a = b.dataset.act;
    try {
      if (a === 'print') return void openHtml(`/sales/docs/${id}/print`).catch(showError);
      if (a === 'edit') { m.close(); return void docEditor(x, after); }
      if (a === 'convert') { const r = await post(`/sales/docs/${id}/convert`); done('สร้างใบแจ้งหนี้ฉบับร่างแล้ว'); return void docDetail(r.id, after); }
      if (a === 'accept' || a === 'reject' || a === 'reopen') { await post(`/sales/docs/${id}/status`, { status: { accept: 'accepted', reject: 'rejected', reopen: 'open' }[a] }); return void done('บันทึกแล้ว'); }
      if (a === 'delete') { if (!(await confirmBox('ลบเอกสารนี้?', 'ลบ'))) return; await del(`/sales/docs/${id}`); return void done('ลบแล้ว'); }
      if (a === 'issue') {
        if (!(await confirmBox('ออกใบกำกับภาษี? ระบบจะให้เลขที่เอกสาร ตัดสต็อก และลงบัญชี หลังจากนี้แก้ไขไม่ได้ (ยกเลิกได้)', 'ออกเอกสาร', false))) return;
        const r = await post(`/sales/docs/${id}/issue`); done(`ออกเอกสาร ${r.doc_no} แล้ว`); return void docDetail(id, after);
      }
      if (a === 'void') { if (!(await confirmBox('ยกเลิกใบกำกับภาษีนี้? เลขที่เอกสารจะถูกเก็บไว้พร้อมสถานะยกเลิก สต็อกและรายการบัญชีจะถูกกลับรายการ', 'ยกเลิกเอกสาร'))) return; await post(`/sales/docs/${id}/void`); return void done('ยกเลิกแล้ว'); }
      if (a === 'pay') {
        const accs = (await get('/accounting/accounts')).filter((ac) => ac.type === 'asset' && ac.is_active && /^11/.test(ac.code));
        const left = Math.round((d.total - d.paid_amount) * 100) / 100;
        const wht = Math.round(Math.min(d.wht_amount, left) * 100) / 100;
        formModal({
          title: `รับชำระ ${d.doc_no} (คงค้าง ${money(left)})`,
          fields: [
            { key: 'pay_date', label: 'วันที่รับเงิน', type: 'date', default: today() },
            { key: 'amount', label: 'จำนวนเงินที่ได้รับ', type: 'number', step: '0.01', min: 0, required: true, default: Math.round((left - wht) * 100) / 100 },
            { key: 'wht_amount', label: 'ภาษีที่ลูกค้าหัก ณ ที่จ่าย (ตามหนังสือรับรอง)', type: 'number', step: '0.01', min: 0, default: wht },
            { key: 'method', label: 'วิธีชำระ', type: 'select', options: PAY_METHODS, default: 'transfer' },
            { key: 'account_id', label: 'เข้าบัญชี', type: 'select', numeric: true, required: true, options: accs.map((ac) => ({ value: ac.id, label: `${ac.code} ${ac.name}` })), default: accs.find((ac) => ac.system_role === 'bank')?.id },
            { key: 'reference', label: 'อ้างอิง (เลขที่โอน / เช็ค)' },
          ],
          submitLabel: 'บันทึกรับชำระ + ออกใบเสร็จ',
          onSubmit: async (v) => { const r = await post(`/sales/docs/${id}/payments`, v); done(`ออกใบเสร็จ ${r.receipt_no} แล้ว`); docDetail(id, after); },
        });
      }
    } catch (e) { showError(e); }
  }));
}

// ---------------------------------------------------------------- products

export async function products(el) {
  const accs = session.can('accounting.view', 'accounting.manage', 'sales.manage', 'purchasing.manage', 'inventory.manage') ? await get('/accounting/accounts').catch(() => []) : [];
  const canEdit = session.can('sales.manage', 'purchasing.manage', 'inventory.manage', 'accounting.manage');
  await crudPage(el, {
    title: 'สินค้าและบริการ', desc: 'ราคาขาย / ราคาซื้อ · ต้นทุนเฉลี่ยเคลื่อนที่ · จุดสั่งซื้อ · VAT', endpoint: '/products', canDelete: false, canEdit, addLabel: 'เพิ่มสินค้า / บริการ',
    columns: [
      { key: 'sku', label: 'รหัส' }, { key: 'name', label: 'ชื่อ' }, { label: 'ชนิด', render: (r) => (r.kind === 'service' ? 'บริการ' : 'สินค้า') },
      { label: 'ราคาขาย', align: 'r', render: (r) => money(r.sale_price) }, { label: 'ต้นทุนเฉลี่ย', align: 'r', hideSm: true, render: (r) => (r.track_stock ? money(r.avg_cost) : '-') },
      { label: 'คงเหลือ', align: 'r', render: (r) => (r.track_stock ? `${qtyFmt(r.on_hand)} ${esc(r.unit)}${num(r.reorder_point) && num(r.on_hand) <= num(r.reorder_point) ? ' <span class="badge warn">ต่ำ</span>' : ''}` : '-') },
      { label: 'VAT', hideSm: true, render: (r) => (r.vat_applicable ? '7%' : 'ยกเว้น') }, { label: '', render: (r) => (r.is_active ? '' : '<span class="badge">ปิด</span>') },
    ],
    extraActions: (r) => (r.track_stock ? `<button class="btn sm" data-act="${r.id}" data-action="card">สต็อก</button>` : ''),
    onAction: (_, row) => stockCard(row.id),
    fields: [
      { key: 'sku', label: 'รหัสสินค้า', required: true }, { key: 'name', label: 'ชื่อ', required: true },
      { key: 'kind', label: 'ชนิด', type: 'select', required: true, default: 'product', options: [{ value: 'product', label: 'สินค้า' }, { value: 'service', label: 'บริการ' }] },
      { key: 'unit', label: 'หน่วยนับ', default: 'ชิ้น' },
      { key: 'sale_price', label: 'ราคาขาย', type: 'number', step: '0.01', min: 0 }, { key: 'purchase_price', label: 'ราคาซื้อล่าสุด', type: 'number', step: '0.01', min: 0 },
      { key: 'vat_applicable', label: 'มีภาษีมูลค่าเพิ่ม (ไม่ติ๊ก = ยกเว้น VAT)', type: 'checkbox', default: true },
      { key: 'track_stock', label: 'นับสต็อก (เฉพาะสินค้า)', type: 'checkbox', default: true },
      { key: 'reorder_point', label: 'จุดสั่งซื้อ (แจ้งเตือนเมื่อต่ำกว่า)', type: 'number', step: '0.001', min: 0 },
      { key: 'opening_qty', label: 'ยอดยกมา (เฉพาะตอนสร้าง)', type: 'number', step: '0.001', min: 0 },
      { key: 'revenue_account_id', label: 'บัญชีรายได้ (ว่าง = ค่าเริ่มต้น)', type: 'select', numeric: true, options: accs.filter((a) => a.type === 'revenue').map((a) => ({ value: a.id, label: `${a.code} ${a.name}` })) },
      { key: 'expense_account_id', label: 'บัญชีค่าใช้จ่ายเมื่อซื้อ (บริการ / ไม่นับสต็อก)', type: 'select', numeric: true, options: accs.filter((a) => a.type === 'expense' || a.type === 'asset').map((a) => ({ value: a.id, label: `${a.code} ${a.name}` })) },
      { key: 'is_active', label: 'ใช้งาน', type: 'checkbox', default: true },
    ],
    toBody: (v) => ({ ...v, sale_price: v.sale_price || 0, purchase_price: v.purchase_price || 0, reorder_point: v.reorder_point || 0, opening_qty: v.opening_qty || 0 }),
  });
}

const SOURCE_TH = { opening: 'ยอดยกมา', sales_invoice: 'ขาย', sales_void: 'ยกเลิกการขาย', purchase_receipt: 'รับสินค้า', purchase_void: 'ยกเลิกการรับ', adjustment: 'ตรวจนับ', transfer: 'โอนคลัง' };

async function stockCard(id) {
  const d = await get(`/products/${id}/stock`);
  modal({
    title: `บัตรสต็อก ${d.product.sku} ${d.product.name}`, wide: true,
    body: `<p>คงเหลือ ${d.balances.map((b) => `${esc(b.name)} <b>${qtyFmt(b.qty)}</b>`).join(' · ')} · ต้นทุนเฉลี่ย ${money(d.product.avg_cost)}</p>
      ${table([{ label: 'วันที่', render: (m) => fmtDate(m.move_date) }, { key: 'warehouse', label: 'คลัง' }, { label: 'รายการ', render: (m) => `${esc(SOURCE_TH[m.source] || m.source)} ${esc(m.note || '')}` },
        { label: 'รับ', align: 'r', render: (m) => (m.qty > 0 ? qtyFmt(m.qty) : '') }, { label: 'จ่าย', align: 'r', render: (m) => (m.qty < 0 ? qtyFmt(-m.qty) : '') },
        { label: 'ต้นทุน/หน่วย', align: 'r', render: (m) => money(m.unit_cost) }, { label: 'มูลค่า', align: 'r', render: (m) => money(m.value) }], d.moves, { empty: 'ยังไม่มีความเคลื่อนไหว' })}`,
  });
}

// ---------------------------------------------------------------- inventory

export async function inventory(el) {
  const canManage = session.can('inventory.manage');
  el.innerHTML = `<div class="page-head"><div><h1>คลังสินค้า</h1><p>สต็อกคงเหลือและมูลค่า · ตรวจนับ · โอนระหว่างคลัง · หลายคลัง / หลายสาขา</p></div><span class="spacer"></span>
      ${canManage ? `<button class="btn" data-transfer>โอนระหว่างคลัง</button><button class="btn primary" data-count>ตรวจนับ / ปรับยอด</button>` : ''}</div>
    ${tabs([['stock', 'สต็อกคงเหลือ'], ['low', 'ต่ำกว่าจุดสั่งซื้อ'], ['wh', 'คลังสินค้า']], 'stock')}<div data-body></div>`;
  const body = el.querySelector('[data-body]');
  let tab = 'stock';
  const warehouses = await get('/warehouses');
  const load = async () => {
    if (tab === 'wh') {
      return crudPage(body, {
        title: 'คลังสินค้า', endpoint: '/warehouses', canDelete: false, canEdit: canManage, addLabel: 'เพิ่มคลัง',
        columns: [{ key: 'code', label: 'รหัส' }, { key: 'name', label: 'ชื่อ' }, { label: 'หลัก', render: (r) => (r.is_default ? '✓' : '') }, { label: 'ใช้งาน', render: (r) => (r.is_active ? '✓' : '-') }],
        fields: [{ key: 'code', label: 'รหัส', required: true }, { key: 'name', label: 'ชื่อ', required: true },
          { key: 'branch_id', label: 'สาขา', type: 'select', numeric: true, options: opt.branches },
          { key: 'is_default', label: 'คลังหลัก', type: 'checkbox' }, { key: 'is_active', label: 'ใช้งาน', type: 'checkbox', default: true }],
      });
    }
    const whSel = body.querySelector('[data-wh]')?.value || '';
    const d = await get(`/inventory/stock${qs({ warehouse_id: whSel, low: tab === 'low' ? 'true' : '' })}`);
    body.innerHTML = `<div class="toolbar" style="padding:0 0 12px"><select class="input" data-wh style="width:200px"><option value="">ทุกคลัง</option>${warehouses.map((w) => `<option value="${w.id}" ${String(w.id) === whSel ? 'selected' : ''}>${esc(w.name)}</option>`).join('')}</select>
      <span class="spacer"></span><b>มูลค่าสต็อกรวม ${money(d.total_value)} บาท</b></div>
      <div class="panel">${table([{ key: 'sku', label: 'รหัส' }, { key: 'name', label: 'สินค้า' }, { label: 'คงเหลือ', align: 'r', render: (r) => `${qtyFmt(r.qty)} ${esc(r.unit)}` },
        { label: 'จุดสั่งซื้อ', align: 'r', hideSm: true, render: (r) => (num(r.reorder_point) ? qtyFmt(r.reorder_point) : '-') },
        { label: 'ต้นทุนเฉลี่ย', align: 'r', render: (r) => money(r.avg_cost) }, { label: 'มูลค่า', align: 'r', render: (r) => money(r.value) },
        { label: '', render: (r) => (r.low ? '<span class="badge warn">ต่ำกว่าจุดสั่งซื้อ</span>' : '') }], d.items, { empty: tab === 'low' ? 'ไม่มีสินค้าต่ำกว่าจุดสั่งซื้อ' : 'ยังไม่มีสินค้าที่นับสต็อก', actions: (r) => `<button class="btn sm" data-card="${r.id}">บัตรสต็อก</button>` })}</div>`;
    body.querySelector('[data-wh]').addEventListener('change', () => load().catch(showError));
    body.querySelectorAll('[data-card]').forEach((b) => b.addEventListener('click', () => stockCard(b.dataset.card).catch(showError)));
  };
  el.querySelector('.tabs').addEventListener('click', (e) => {
    const b = e.target.closest('[data-tab]'); if (!b) return;
    el.querySelectorAll('.tabs button').forEach((x) => x.classList.toggle('active', x === b));
    tab = b.dataset.tab; body.innerHTML = ''; load().catch(showError);
  });
  const prods = async () => (await get('/products')).filter((p) => p.track_stock && p.is_active).map((p) => ({ value: p.id, label: `${p.sku} ${p.name} (คงเหลือ ${qtyFmt(p.on_hand)})` }));
  const whOpts = warehouses.filter((w) => w.is_active).map((w) => ({ value: w.id, label: w.name }));
  el.querySelector('[data-count]')?.addEventListener('click', async () => formModal({
    title: 'ตรวจนับ / ปรับยอดสต็อก', intro: '<p class="small muted" style="margin-top:0">ใส่จำนวนที่นับได้จริง ระบบบันทึกผลต่างและลงบัญชีผลต่างจากการตรวจนับให้</p>',
    fields: [{ key: 'product_id', label: 'สินค้า', type: 'select', required: true, numeric: true, options: await prods() },
      { key: 'warehouse_id', label: 'คลัง', type: 'select', required: true, numeric: true, options: whOpts },
      { key: 'counted_qty', label: 'จำนวนที่นับได้', type: 'number', step: '0.001', min: 0, required: true }, { key: 'note', label: 'หมายเหตุ' }],
    onSubmit: async (v) => { const r = await post('/inventory/adjust', v); toast(r.difference ? `ปรับยอด ${r.difference > 0 ? '+' : ''}${r.difference} แล้ว` : 'ยอดตรงกับระบบ'); load(); },
  }));
  el.querySelector('[data-transfer]')?.addEventListener('click', async () => formModal({
    title: 'โอนสินค้าระหว่างคลัง',
    fields: [{ key: 'product_id', label: 'สินค้า', type: 'select', required: true, numeric: true, options: await prods() },
      { key: 'from_warehouse_id', label: 'จากคลัง', type: 'select', required: true, numeric: true, options: whOpts },
      { key: 'to_warehouse_id', label: 'ไปคลัง', type: 'select', required: true, numeric: true, options: whOpts },
      { key: 'qty', label: 'จำนวน', type: 'number', step: '0.001', min: 0.001, required: true }, { key: 'note', label: 'หมายเหตุ' }],
    onSubmit: async (v) => { await post('/inventory/transfer', v); toast('โอนแล้ว'); load(); },
  }));
  await load();
}

// ---------------------------------------------------------------- purchasing

const PO_STATUS = {
  draft: '<span class="badge">ฉบับร่าง</span>', approved: '<span class="badge info">อนุมัติแล้ว</span>', partial: '<span class="badge warn">รับบางส่วน</span>',
  received: '<span class="badge ok">รับครบ</span>', closed: '<span class="badge">ปิด</span>', cancelled: '<span class="badge danger">ยกเลิก</span>',
};
const BILL_STATUS = { posted: '<span class="badge info">ค้างจ่าย</span>', partial: '<span class="badge warn">จ่ายบางส่วน</span>', paid: '<span class="badge ok">จ่ายแล้ว</span>', void: '<span class="badge danger">ยกเลิก</span>' };

export async function purchasing(el) {
  const canManage = session.can('purchasing.manage');
  el.innerHTML = `<div class="page-head"><div><h1>จัดซื้อ</h1><p>ใบสั่งซื้อ (อนุมัติโดยผู้อื่น) → รับสินค้าพร้อมใบกำกับภาษีผู้ขาย → จ่ายชำระพร้อมหัก ณ ที่จ่ายและหนังสือรับรอง 50 ทวิ</p></div><span class="spacer"></span>
      ${canManage ? `<button class="btn" data-newbill>${icon('plus')} รับสินค้า / บันทึกค่าใช้จ่าย</button><button class="btn primary" data-newpo>${icon('plus')} ใบสั่งซื้อ</button>` : ''}</div>
    <div class="grid g-4" style="margin-bottom:16px" data-stats></div>
    ${tabs([['po', 'ใบสั่งซื้อ'], ['bills', 'รับสินค้า / เจ้าหนี้'], ['sup', 'ผู้ขาย']], 'po')}<div data-body></div>`;
  const body = el.querySelector('[data-body]');
  let tab = 'po';
  const load = async () => {
    const s = await get('/purchasing/summary');
    el.querySelector('[data-stats]').innerHTML = `
      <div class="panel stat"><div><b>${money(s.payable)}</b><span>เจ้าหนี้คงค้าง</span></div></div>
      <div class="panel stat ${s.overdue_count ? 'warn' : ''}"><div><b>${money(s.overdue)}</b><span>เกินกำหนดจ่าย ${s.overdue_count} รายการ</span></div></div>
      <div class="panel stat"><div><b>${money(s.open_order_value)}</b><span>ใบสั่งซื้อค้างรับ ${s.open_orders} ใบ</span></div></div>
      <div class="panel stat ${s.low_stock ? 'warn' : ''}"><div><b>${s.low_stock}</b><span>สินค้าต่ำกว่าจุดสั่งซื้อ</span></div></div>`;
    if (tab === 'sup') return suppliers(body, canManage);
    if (tab === 'po') {
      const rows = await get('/purchasing/orders');
      body.innerHTML = `<div class="panel">${table([{ key: 'po_no', label: 'เลขที่' }, { label: 'วันที่', render: (r) => fmtDate(r.po_date) }, { key: 'supplier_name', label: 'ผู้ขาย' },
        { label: 'ยอดรวม', align: 'r', render: (r) => money(r.total) }, { label: 'สถานะ', render: (r) => PO_STATUS[r.status] }], rows,
      { empty: 'ยังไม่มีใบสั่งซื้อ', actions: (r) => `<button class="btn sm" data-po="${r.id}">เปิด</button>` })}</div>`;
    } else {
      const rows = await get('/purchasing/bills');
      body.innerHTML = `<div class="panel">${table([{ key: 'bill_no', label: 'เลขที่รับ' }, { label: 'วันที่', render: (r) => fmtDate(r.bill_date) }, { key: 'supplier_name', label: 'ผู้ขาย' },
        { label: 'ใบกำกับผู้ขาย', hideSm: true, render: (r) => esc(r.supplier_invoice_no || '') }, { label: 'ครบกำหนด', hideSm: true, render: (r) => fmtDate(r.due_date) },
        { label: 'ยอดรวม', align: 'r', render: (r) => money(r.total) }, { label: 'ค้างจ่าย', align: 'r', render: (r) => (['posted', 'partial'].includes(r.status) ? `<b>${money(r.total - r.paid_amount)}</b>` : '-') },
        { label: 'สถานะ', render: (r) => BILL_STATUS[r.status] }], rows, { empty: 'ยังไม่มีรายการรับสินค้า', actions: (r) => `<button class="btn sm" data-bill="${r.id}">เปิด</button>` })}</div>`;
    }
  };
  el.querySelector('.tabs').addEventListener('click', (e) => {
    const b = e.target.closest('[data-tab]'); if (!b) return;
    el.querySelectorAll('.tabs button').forEach((x) => x.classList.toggle('active', x === b));
    tab = b.dataset.tab; body.innerHTML = ''; load().catch(showError);
  });
  el.addEventListener('click', (e) => {
    const t = e.target;
    if (t.closest('[data-po]')) poDetail(t.closest('[data-po]').dataset.po, load).catch(showError);
    else if (t.closest('[data-bill]')) billDetail(t.closest('[data-bill]').dataset.bill, load).catch(showError);
    else if (t.closest('[data-newpo]')) poEditor(null, load).catch(showError);
    else if (t.closest('[data-newbill]')) billEditor(null, load).catch(showError);
  });
  await load();
}

async function suppliers(b, canManage) {
  await crudPage(b, {
    title: 'ผู้ขาย', endpoint: '/suppliers', canDelete: false, canEdit: canManage, addLabel: 'เพิ่มผู้ขาย',
    columns: [{ key: 'name', label: 'ชื่อ' }, { label: 'ประเภท', render: (r) => (r.kind === 'individual' ? 'บุคคลธรรมดา (ภ.ง.ด.3)' : 'นิติบุคคล (ภ.ง.ด.53)') },
      { key: 'tax_id', label: 'เลขผู้เสียภาษี', hideSm: true }, { label: 'หัก ณ ที่จ่าย', render: (r) => (num(r.wht_rate) ? `${r.wht_rate}%` : '-') },
      { key: 'phone', label: 'โทร', hideSm: true }, { label: '', render: (r) => (r.is_active ? '' : '<span class="badge">ปิด</span>') }],
    fields: [
      { key: 'name', label: 'ชื่อผู้ขาย', required: true }, { key: 'code', label: 'รหัส' },
      { key: 'kind', label: 'ประเภท', type: 'select', required: true, default: 'company', options: [{ value: 'company', label: 'นิติบุคคล' }, { value: 'individual', label: 'บุคคลธรรมดา' }] },
      { key: 'tax_id', label: 'เลขประจำตัวผู้เสียภาษี' }, { key: 'branch_code', label: 'รหัสสาขา (00000 = สนญ.)' },
      { key: 'wht_rate', label: 'หัก ณ ที่จ่ายเมื่อจ่ายเงิน (%)', type: 'select', default: 0, options: [0, 1, 2, 3, 5].map((v) => ({ value: v, label: v ? `${v}%` : 'ไม่หัก' })) },
      { key: 'credit_days', label: 'เครดิต (วัน)', type: 'number', min: 0, max: 365, default: 30 },
      { key: 'contact_name', label: 'ผู้ติดต่อ' }, { key: 'phone', label: 'โทรศัพท์' }, { key: 'email', label: 'อีเมล', type: 'email' },
      { key: 'bank_name', label: 'ธนาคาร' }, { key: 'bank_account', label: 'เลขที่บัญชี' },
      { key: 'address', label: 'ที่อยู่', type: 'textarea' }, { key: 'is_active', label: 'ใช้งาน', type: 'checkbox', default: true },
    ],
    toBody: (v) => ({ ...v, wht_rate: Number(v.wht_rate) || 0, credit_days: v.credit_days ?? 30 }),
  });
}

async function poEditor(existing, after) {
  const [sups, products, warehouses] = await Promise.all([get('/suppliers'), get('/products'), get('/warehouses')]);
  const o = existing?.order || { vat_mode: 'exclusive', discount: 0 };
  const lines = existing ? existing.lines.map((l) => ({ ...l })) : [{ qty: 1, unit_price: 0, discount_pct: 0 }];
  const m = modal({
    title: existing ? `แก้ไขใบสั่งซื้อ ${o.po_no}` : 'สร้างใบสั่งซื้อ', wide: true,
    body: `<form data-f><div class="form-grid">
      <div class="field"><label for="p-sup">ผู้ขาย *</label><select class="input" id="p-sup" name="supplier_id" required><option value="">- เลือก -</option>${sups.filter((s) => s.is_active).map((s) => `<option value="${s.id}" ${s.id === o.supplier_id ? 'selected' : ''}>${esc(s.name)}</option>`).join('')}</select></div>
      <div class="field"><label for="p-exp">ต้องการรับสินค้า</label><input class="input" type="date" id="p-exp" name="expected_date" value="${esc(o.expected_date || '')}"></div>
      <div class="field"><label for="p-vat">ภาษีมูลค่าเพิ่ม</label><select class="input" id="p-vat" name="vat_mode">${VAT_MODES.map((v) => `<option value="${v.value}" ${v.value === o.vat_mode ? 'selected' : ''}>${v.label}</option>`).join('')}</select></div>
      <div class="field"><label for="p-wh">รับเข้าคลัง</label><select class="input" id="p-wh" name="warehouse_id">${warehouses.filter((w) => w.is_active).map((w) => `<option value="${w.id}" ${w.id === o.warehouse_id ? 'selected' : ''}>${esc(w.name)}</option>`).join('')}</select></div></div>
      <div data-lines style="margin-top:12px"></div>
      <div class="grid g-2" style="margin-top:12px"><div class="field"><label for="p-note">หมายเหตุ</label><textarea class="input" id="p-note" name="note">${esc(o.note || '')}</textarea></div>
      <div><div class="field"><label for="p-disc">ส่วนลดท้ายบิล</label><input class="input r" type="number" step="0.01" min="0" id="p-disc" name="discount" value="${esc(o.discount || 0)}"></div><p class="r" data-sum></p></div></div></form>`,
    foot: '<button class="btn" data-close>ยกเลิก</button><button class="btn primary" data-save>บันทึก</button>',
  });
  const f = m.body.querySelector('[data-f]');
  const sum = () => { const t = calcTotals(lines, f.discount.value, f.vat_mode.value); m.body.querySelector('[data-sum]').innerHTML = `VAT ${money(t.vat)} · <b>รวม ${money(t.total)}</b>`; };
  const read = lineEditor(m.body.querySelector('[data-lines]'), lines, products, { priceKey: 'purchase_price', onChange: sum });
  ['discount', 'vat_mode'].forEach((k) => f[k].addEventListener('input', sum));
  m.el.querySelector('[data-save]').addEventListener('click', async () => {
    if (!f.reportValidity()) return;
    const bodyv = { supplier_id: Number(f.supplier_id.value), expected_date: f.expected_date.value || null, vat_mode: f.vat_mode.value, warehouse_id: Number(f.warehouse_id.value) || null,
      discount: num(f.discount.value), note: f.note.value, lines: read() };
    try {
      const r = existing ? await put(`/purchasing/orders/${o.id}`, bodyv) : await post('/purchasing/orders', bodyv);
      toast('บันทึกแล้ว'); m.close(); after?.(); poDetail(existing ? o.id : r.id, after);
    } catch (e) { showError(e); }
  });
}

async function poDetail(id, after) {
  const x = await get(`/purchasing/orders/${id}`);
  const o = x.order; const can = session.can('purchasing.manage');
  const acts = [`<button class="btn" data-act="print">${icon('print')} พิมพ์</button>`];
  if (can && o.status === 'draft') acts.push('<button class="btn" data-act="edit">แก้ไข</button>', '<button class="btn primary" data-act="approve">อนุมัติ</button>');
  if (can && ['draft', 'approved', 'partial'].includes(o.status)) acts.push('<button class="btn danger" data-act="cancel">ยกเลิก / ปิด</button>');
  if (can && ['approved', 'partial'].includes(o.status)) acts.push('<button class="btn primary" data-act="receive">รับสินค้า</button>');
  const m = modal({
    title: `ใบสั่งซื้อ ${o.po_no}`, wide: true,
    body: `<div class="row wrap" style="margin-bottom:10px">${PO_STATUS[o.status]} <span class="small muted">${esc(o.supplier_name)} · ${fmtDate(o.po_date)}${o.approved_by_name ? ` · อนุมัติโดย ${esc(o.approved_by_name)}` : ''}</span></div>
      ${table([{ key: 'description', label: 'รายการ' }, { label: 'สั่ง', align: 'r', render: (l) => `${qtyFmt(l.qty)} ${esc(l.unit || '')}` }, { label: 'รับแล้ว', align: 'r', render: (l) => qtyFmt(l.received_qty) },
        { label: 'ราคา/หน่วย', align: 'r', render: (l) => money(l.unit_price) }, { label: 'จำนวนเงิน', align: 'r', render: (l) => money(l.amount) }], x.lines)}
      <p class="r">VAT ${money(o.vat_amount)} · <b>รวม ${money(o.total)}</b></p>
      ${x.bills.length ? `<p class="small muted">การรับสินค้า: ${x.bills.map((b) => `${esc(b.bill_no)} (${BILL_STATUS[b.status].replace(/<[^>]+>/g, '')})`).join(', ')}</p>` : ''}`,
    foot: acts.join(''),
  });
  m.el.querySelectorAll('[data-act]').forEach((b) => b.addEventListener('click', async () => {
    const a = b.dataset.act;
    try {
      if (a === 'print') return void openHtml(`/purchasing/orders/${id}/print`).catch(showError);
      if (a === 'edit') { m.close(); return void poEditor(x, after); }
      if (a === 'approve') { await post(`/purchasing/orders/${id}/approve`); toast('อนุมัติแล้ว'); }
      if (a === 'cancel') { if (!(await confirmBox('ยกเลิก / ปิดใบสั่งซื้อนี้? (ส่วนที่ยังไม่รับจะไม่รับอีก)', 'ยืนยัน'))) return; await post(`/purchasing/orders/${id}/cancel`); toast('บันทึกแล้ว'); }
      if (a === 'receive') { m.close(); return void billEditor(x, after); }
      m.close(); after?.(); poDetail(id, after);
    } catch (e) { showError(e); }
  }));
}

/** Goods receipt / supplier bill; `po` pre-fills the remaining quantities of a purchase order. */
async function billEditor(po, after) {
  const [sups, products, warehouses] = await Promise.all([get('/suppliers'), get('/products'), get('/warehouses')]);
  const o = po?.order;
  const lines = po ? po.lines.filter((l) => num(l.qty) > num(l.received_qty)).map((l) => ({ po_line_id: l.id, product_id: l.product_id, description: l.description, qty: num(l.qty) - num(l.received_qty),
    unit: l.unit, unit_price: l.unit_price, discount_pct: l.discount_pct, vat_exempt: l.vat_exempt })) : [{ qty: 1, unit_price: 0, discount_pct: 0 }];
  const m = modal({
    title: o ? `รับสินค้าตามใบสั่งซื้อ ${o.po_no}` : 'รับสินค้า / บันทึกค่าใช้จ่าย', wide: true,
    body: `<form data-f><div class="form-grid">
      <div class="field"><label for="b-sup">ผู้ขาย *</label><select class="input" id="b-sup" name="supplier_id" required ${o ? 'disabled' : ''}><option value="">- เลือก -</option>${sups.filter((s) => s.is_active).map((s) => `<option value="${s.id}" data-wht="${s.wht_rate}" ${s.id === o?.supplier_id ? 'selected' : ''}>${esc(s.name)}</option>`).join('')}</select></div>
      <div class="field"><label for="b-inv">เลขที่ใบกำกับภาษีของผู้ขาย</label><input class="input" id="b-inv" name="supplier_invoice_no"></div>
      <div class="field"><label for="b-date">วันที่ (ตามใบกำกับ)</label><input class="input" type="date" id="b-date" name="bill_date" value="${today()}"></div>
      <div class="field"><label for="b-vat">ภาษีมูลค่าเพิ่ม</label><select class="input" id="b-vat" name="vat_mode">${VAT_MODES.map((v) => `<option value="${v.value}" ${v.value === (o?.vat_mode || 'exclusive') ? 'selected' : ''}>${v.label}</option>`).join('')}</select></div>
      <div class="field"><label for="b-wht">เราหัก ณ ที่จ่ายเมื่อจ่ายเงิน</label><select class="input" id="b-wht" name="wht_rate">${[0, 1, 2, 3, 5].map((r) => `<option value="${r}">${r ? `${r}%` : 'ไม่หัก'}</option>`).join('')}</select></div>
      <div class="field"><label for="b-wh">รับเข้าคลัง</label><select class="input" id="b-wh" name="warehouse_id">${warehouses.filter((w) => w.is_active).map((w) => `<option value="${w.id}" ${w.id === o?.warehouse_id ? 'selected' : ''}>${esc(w.name)}</option>`).join('')}</select></div></div>
      <p class="small muted">สินค้าที่นับสต็อกจะเข้าคลังตามต้นทุนสุทธิ (ไม่รวม VAT) · รายการอื่นลงบัญชีค่าใช้จ่ายของสินค้า/บริการนั้น หรือค่าใช้จ่ายเบ็ดเตล็ด</p>
      <div data-lines></div><p class="r" data-sum></p></form>`,
    foot: '<button class="btn" data-close>ยกเลิก</button><button class="btn primary" data-save>บันทึกรับและตั้งหนี้</button>',
  });
  const f = m.body.querySelector('[data-f]');
  const sum = () => { const t = calcTotals(lines, 0, f.vat_mode.value, 7, num(f.wht_rate.value)); m.body.querySelector('[data-sum]').innerHTML = `VAT (ภาษีซื้อ) ${money(t.vat)} · <b>รวม ${money(t.total)}</b>${t.wht ? ` · หัก ณ ที่จ่าย ${money(t.wht)}` : ''}`; };
  f.supplier_id.addEventListener('change', () => { f.wht_rate.value = String(num(f.supplier_id.selectedOptions[0]?.dataset.wht)); sum(); });
  if (o) f.wht_rate.value = String(num(f.supplier_id.selectedOptions[0]?.dataset.wht));
  const read = lineEditor(m.body.querySelector('[data-lines]'), lines, products, { priceKey: 'purchase_price', onChange: sum });
  ['vat_mode', 'wht_rate'].forEach((k) => f[k].addEventListener('input', sum));
  m.el.querySelector('[data-save]').addEventListener('click', async () => {
    if (!f.reportValidity()) return;
    const ls = read().map((l, i) => ({ ...l, po_line_id: lines[i].po_line_id || null }));
    try {
      const r = await post('/purchasing/bills', { supplier_id: Number(f.supplier_id.value), po_id: o?.id || null, supplier_invoice_no: f.supplier_invoice_no.value, bill_date: f.bill_date.value || null,
        vat_mode: f.vat_mode.value, wht_rate: num(f.wht_rate.value), warehouse_id: Number(f.warehouse_id.value) || null, lines: ls });
      toast(`บันทึก ${r.bill_no} แล้ว`); m.close(); after?.(); billDetail(r.id, after);
    } catch (e) { showError(e); }
  });
}

async function billDetail(id, after) {
  const x = await get(`/purchasing/bills/${id}`);
  const b = x.bill; const can = session.can('purchasing.manage');
  const acts = [];
  if (can && ['posted', 'partial'].includes(b.status)) acts.push('<button class="btn primary" data-act="pay">จ่ายชำระ</button>');
  if (can && b.status === 'posted') acts.push('<button class="btn danger" data-act="void">ยกเลิกรายการ</button>');
  const m = modal({
    title: `รับสินค้า ${b.bill_no}`, wide: true,
    body: `<div class="row wrap" style="margin-bottom:10px">${BILL_STATUS[b.status]} <span class="small muted">${esc(b.supplier_name)} · ใบกำกับ ${esc(b.supplier_invoice_no || '-')} · ${fmtDate(b.bill_date)}${b.po_no ? ` · ${esc(b.po_no)}` : ''}</span></div>
      ${table([{ key: 'description', label: 'รายการ' }, { label: 'จำนวน', align: 'r', render: (l) => `${qtyFmt(l.qty)} ${esc(l.unit || '')}` }, { label: 'ราคา', align: 'r', render: (l) => money(l.unit_price) },
        { label: 'ลงบัญชี', hideSm: true, render: (l) => esc(l.account_name || (l.product_id ? 'สินค้าคงเหลือ' : '')) }, { label: 'จำนวนเงิน', align: 'r', render: (l) => money(l.amount) }], x.lines)}
      <p class="r">ภาษีซื้อ ${money(b.vat_amount)} · <b>รวม ${money(b.total)}</b> · จ่ายแล้ว ${money(b.paid_amount)} · <b>ค้าง ${money(b.total - b.paid_amount)}</b></p>
      ${x.payments.length ? `<h3>การจ่ายชำระ</h3>${table([{ key: 'voucher_no', label: 'ใบสำคัญจ่าย' }, { label: 'วันที่', render: (p) => fmtDate(p.pay_date) }, { key: 'account_name', label: 'จากบัญชี' },
        { label: 'จ่าย', align: 'r', render: (p) => money(p.amount) }, { label: 'หัก ณ ที่จ่าย', align: 'r', render: (p) => money(p.wht_amount) }, { label: '50 ทวิ', render: (p) => esc(p.wht_cert_no || '-') },
        { label: '', render: (p) => (p.status === 'void' ? BILL_STATUS.void : '') }], x.payments, { actions: (p) => (p.status === 'active' ? `<button class="btn sm" data-pv="${p.id}">${icon('print')}</button>${can ? `<button class="btn sm ghost danger" data-vp="${p.id}">ยกเลิก</button>` : ''}` : '') })}` : ''}`,
    foot: acts.join(''),
  });
  m.body.querySelectorAll('[data-pv]').forEach((x2) => x2.addEventListener('click', () => openHtml(`/purchasing/payments/${x2.dataset.pv}/print`).catch(showError)));
  m.body.querySelectorAll('[data-vp]').forEach((x2) => x2.addEventListener('click', async () => {
    if (!(await confirmBox('ยกเลิกการจ่ายชำระนี้?', 'ยกเลิก'))) return;
    try { await post(`/purchasing/payments/${x2.dataset.vp}/void`); toast('ยกเลิกแล้ว'); m.close(); after?.(); billDetail(id, after); } catch (e) { showError(e); }
  }));
  m.el.querySelectorAll('[data-act]').forEach((x2) => x2.addEventListener('click', async () => {
    try {
      if (x2.dataset.act === 'void') {
        if (!(await confirmBox('ยกเลิกรายการรับสินค้านี้? สต็อกและรายการบัญชีจะถูกกลับรายการ', 'ยกเลิกรายการ'))) return;
        await post(`/purchasing/bills/${id}/void`); toast('ยกเลิกแล้ว'); m.close(); after?.(); return;
      }
      const accs = (await get('/accounting/accounts')).filter((ac) => ac.type === 'asset' && ac.is_active && /^11/.test(ac.code));
      const left = Math.round((b.total - b.paid_amount) * 100) / 100;
      const wht = Math.round(Math.min(b.wht_amount, left) * 100) / 100;
      formModal({
        title: `จ่ายชำระ ${b.bill_no} (ค้าง ${money(left)})`,
        fields: [
          { key: 'pay_date', label: 'วันที่จ่าย', type: 'date', default: today() },
          { key: 'amount', label: 'จำนวนเงินที่โอน / จ่าย', type: 'number', step: '0.01', min: 0, required: true, default: Math.round((left - wht) * 100) / 100 },
          { key: 'wht_amount', label: 'ภาษีที่เราหัก ณ ที่จ่าย', type: 'number', step: '0.01', min: 0, default: wht },
          { key: 'wht_income_type', label: 'ประเภทเงินได้ (บน 50 ทวิ)', type: 'select', default: 'ค่าบริการ / ค่าจ้างทำของ (มาตรา 40(8))',
            options: ['ค่าบริการ / ค่าจ้างทำของ (มาตรา 40(8))', 'ค่าจ้างทำของ / ค่าวิชาชีพ (มาตรา 40(2))', 'ค่าเช่า (มาตรา 40(5))', 'ค่าขนส่ง (มาตรา 40(8))', 'ค่าโฆษณา (มาตรา 40(8))'].map((v) => ({ value: v, label: v })) },
          { key: 'method', label: 'วิธีจ่าย', type: 'select', options: PAY_METHODS, default: 'transfer' },
          { key: 'account_id', label: 'จ่ายจากบัญชี', type: 'select', numeric: true, required: true, options: accs.map((ac) => ({ value: ac.id, label: `${ac.code} ${ac.name}` })), default: accs.find((ac) => ac.system_role === 'bank')?.id },
          { key: 'reference', label: 'อ้างอิง' },
        ],
        submitLabel: 'บันทึกจ่าย',
        onSubmit: async (v) => { const r = await post(`/purchasing/bills/${id}/payments`, v); toast(`ออก ${r.voucher_no}${r.wht_cert_no ? ` และ 50 ทวิ ${r.wht_cert_no}` : ''} แล้ว`); m.close(); after?.(); billDetail(id, after); },
      });
    } catch (e) { showError(e); }
  }));
}
