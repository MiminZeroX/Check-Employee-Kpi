//! Financial reports: general ledger, profit & loss, balance sheet, AR / AP aging,
//! VAT report (ภ.พ.30 with output / input tax lists), withholding tax (ภ.ง.ด.1 / 3 / 53)
//! and an Excel workbook of the statements.

use axum::{
    extract::{Path, Query, State},
    http::header,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use chrono::{Datelike, NaiveDate};
use rust_decimal::{prelude::ToPrimitive, Decimal};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    auth::{audit, AuthUser},
    error::{bad, AppError, AppResult},
    routes::accounting::{balances, period, BalanceRow, PeriodQ},
    state::AppState,
    util::local_today,
    xlsx::{self, Cell, Sheet},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/accounting/ledger/{account_id}", get(ledger))
        .route("/accounting/profit-loss", get(profit_loss))
        .route("/accounting/balance-sheet", get(balance_sheet))
        .route("/accounting/aging/{kind}", get(aging))
        .route("/accounting/vat-report", get(vat_report))
        .route("/accounting/wht-report", get(wht_report))
        .route("/accounting/financials.xlsx", get(financials_xlsx))
        .route("/accounting/tax-reports.xlsx", get(tax_xlsx))
}

fn view(u: &AuthUser) -> AppResult<()> {
    u.need_any(&["accounting.view", "accounting.manage"])
}

fn f(v: Decimal) -> f64 {
    v.to_f64().unwrap_or(0.0)
}

// ---------------------------------------------------------------- general ledger

async fn ledger(State(st): State<AppState>, u: AuthUser, Path(account): Path<i64>, Query(q): Query<PeriodQ>) -> AppResult<Json<Value>> {
    view(&u)?;
    let (from, to) = period(&st, &q)?;
    let acc: Option<(String, String, String)> = sqlx::query_as("SELECT code, name, type FROM accounts WHERE id = ?").bind(account).fetch_optional(&st.db).await?;
    let (code, name, kind) = acc.ok_or(AppError::NotFound)?;
    // credit-normal accounts show balances as positive credits
    let sign = if ["liability", "equity", "revenue"].contains(&kind.as_str()) { -Decimal::ONE } else { Decimal::ONE };
    let opening: Decimal = sqlx::query_scalar(
        "SELECT COALESCE(SUM(l.debit - l.credit), 0) FROM journal_lines l JOIN journal_entries e ON e.id = l.entry_id
          WHERE l.account_id = ? AND e.status = 'posted' AND e.entry_date < ?",
    )
    .bind(account)
    .bind(from)
    .fetch_one(&st.db)
    .await?;
    let rows: Vec<(i64, String, NaiveDate, Option<String>, Option<String>, String, Decimal, Decimal)> = sqlx::query_as(
        "SELECT e.id, e.entry_no, e.entry_date, e.memo, l.memo, e.source, l.debit, l.credit
           FROM journal_lines l JOIN journal_entries e ON e.id = l.entry_id
          WHERE l.account_id = ? AND e.status = 'posted' AND e.entry_date BETWEEN ? AND ?
          ORDER BY e.entry_date, e.id, l.id LIMIT 5000",
    )
    .bind(account)
    .bind(from)
    .bind(to)
    .fetch_all(&st.db)
    .await?;
    let mut bal = opening;
    let lines: Vec<Value> = rows
        .into_iter()
        .map(|r| {
            bal += r.6 - r.7;
            json!({ "entry_id": r.0, "entry_no": r.1, "entry_date": r.2, "memo": r.4.filter(|m| !m.is_empty()).or(r.3), "source": r.5,
                    "debit": r.6, "credit": r.7, "balance": bal * sign })
        })
        .collect();
    Ok(Json(json!({ "account": { "id": account, "code": code, "name": name, "type": kind }, "from": from, "to": to,
                    "opening": opening * sign, "closing": bal * sign, "lines": lines })))
}

// ---------------------------------------------------------------- statements

struct Pl {
    revenue: Vec<(BalanceRow, Decimal)>,
    cogs: Vec<(BalanceRow, Decimal)>,
    expenses: Vec<(BalanceRow, Decimal)>,
}

impl Pl {
    fn total(v: &[(BalanceRow, Decimal)]) -> Decimal {
        v.iter().map(|x| x.1).sum()
    }
    fn net(&self) -> Decimal {
        Self::total(&self.revenue) - Self::total(&self.cogs) - Self::total(&self.expenses)
    }
}

async fn pl_data(st: &AppState, from: NaiveDate, to: NaiveDate) -> AppResult<Pl> {
    let mut pl = Pl { revenue: vec![], cogs: vec![], expenses: vec![] };
    for r in balances(st, from, to).await? {
        let movement = r.debit - r.credit;
        if movement.is_zero() {
            continue;
        }
        match r.kind.as_str() {
            "revenue" => pl.revenue.push((r, -movement)),
            "expense" if r.code.starts_with("51") => pl.cogs.push((r, movement)),
            "expense" => pl.expenses.push((r, movement)),
            _ => {}
        }
    }
    Ok(pl)
}

fn rows_json(v: &[(BalanceRow, Decimal)]) -> Vec<Value> {
    v.iter().map(|(r, a)| json!({ "id": r.id, "code": r.code, "name": r.name, "amount": a })).collect()
}

async fn profit_loss(State(st): State<AppState>, u: AuthUser, Query(q): Query<PeriodQ>) -> AppResult<Json<Value>> {
    view(&u)?;
    let (from, to) = period(&st, &q)?;
    let pl = pl_data(&st, from, to).await?;
    let (rev, cogs, exp) = (Pl::total(&pl.revenue), Pl::total(&pl.cogs), Pl::total(&pl.expenses));
    Ok(Json(json!({
        "from": from, "to": to,
        "revenue": rows_json(&pl.revenue), "cogs": rows_json(&pl.cogs), "expenses": rows_json(&pl.expenses),
        "total_revenue": rev, "total_cogs": cogs, "gross_profit": rev - cogs, "total_expenses": exp, "net_profit": pl.net(),
    })))
}

struct Bs {
    assets: Vec<(BalanceRow, Decimal)>,
    liabilities: Vec<(BalanceRow, Decimal)>,
    equity: Vec<(BalanceRow, Decimal)>,
    earnings: Decimal,
}

async fn bs_data(st: &AppState, as_of: NaiveDate) -> AppResult<Bs> {
    let mut bs = Bs { assets: vec![], liabilities: vec![], equity: vec![], earnings: Decimal::ZERO };
    let start = NaiveDate::from_ymd_opt(1900, 1, 1).unwrap();
    for r in balances(st, start, as_of).await? {
        let bal = r.opening + r.debit - r.credit;
        if bal.is_zero() {
            continue;
        }
        match r.kind.as_str() {
            "asset" => bs.assets.push((r, bal)),
            "liability" => bs.liabilities.push((r, -bal)),
            "equity" => bs.equity.push((r, -bal)),
            // revenue and expenses not yet closed to retained earnings
            _ => bs.earnings -= bal,
        }
    }
    Ok(bs)
}

async fn balance_sheet(State(st): State<AppState>, u: AuthUser, Query(q): Query<PeriodQ>) -> AppResult<Json<Value>> {
    view(&u)?;
    let as_of = q.as_of.or(q.to).unwrap_or_else(|| local_today(st.cfg.tz_offset_min));
    let bs = bs_data(&st, as_of).await?;
    let ta = Pl::total(&bs.assets);
    let tl = Pl::total(&bs.liabilities);
    let te = Pl::total(&bs.equity) + bs.earnings;
    Ok(Json(json!({
        "as_of": as_of, "assets": rows_json(&bs.assets), "liabilities": rows_json(&bs.liabilities), "equity": rows_json(&bs.equity),
        "earnings": bs.earnings, "total_assets": ta, "total_liabilities": tl, "total_equity": te,
        "balanced": ta == tl + te,
    })))
}

// ---------------------------------------------------------------- aging

async fn aging(State(st): State<AppState>, u: AuthUser, Path(kind): Path<String>, Query(q): Query<PeriodQ>) -> AppResult<Json<Value>> {
    let as_of = q.as_of.or(q.to).unwrap_or_else(|| local_today(st.cfg.tz_offset_min));
    let rows: Vec<(i64, String, i64, String, NaiveDate, Option<NaiveDate>, Decimal)> = match kind.as_str() {
        "ar" => {
            u.need_any(&["accounting.view", "accounting.manage", "sales.view", "sales.manage"])?;
            sqlx::query_as(
                "SELECT id, COALESCE(doc_no, ''), customer_id, customer_name, doc_date, due_date, total - paid_amount
                   FROM sales_docs WHERE doc_type = 'invoice' AND status IN ('issued', 'partial') AND doc_date <= ? ORDER BY customer_name, due_date",
            )
            .bind(as_of)
            .fetch_all(&st.db)
            .await?
        }
        "ap" => {
            u.need_any(&["accounting.view", "accounting.manage", "purchasing.view", "purchasing.manage"])?;
            sqlx::query_as(
                "SELECT b.id, b.bill_no, b.supplier_id, s.name, b.bill_date, b.due_date, b.total - b.paid_amount
                   FROM purchase_bills b JOIN suppliers s ON s.id = b.supplier_id
                  WHERE b.status IN ('posted', 'partial') AND b.bill_date <= ? ORDER BY s.name, b.due_date",
            )
            .bind(as_of)
            .fetch_all(&st.db)
            .await?
        }
        _ => return Err(AppError::NotFound),
    };
    const BUCKETS: [&str; 5] = ["current", "d1_30", "d31_60", "d61_90", "over_90"];
    let bucket = |due: Option<NaiveDate>| {
        let days = due.map(|d| (as_of - d).num_days()).unwrap_or(0);
        match days {
            i64::MIN..=0 => 0,
            1..=30 => 1,
            31..=60 => 2,
            61..=90 => 3,
            _ => 4,
        }
    };
    let mut parties: Vec<(i64, String, [Decimal; 5], Vec<Value>)> = Vec::new();
    let mut totals = [Decimal::ZERO; 5];
    for (id, no, pid, pname, date, due, left) in rows {
        let b = bucket(due);
        totals[b] += left;
        let doc = json!({ "id": id, "doc_no": no, "date": date, "due_date": due, "outstanding": left, "bucket": BUCKETS[b],
                          "days_overdue": due.map(|d| (as_of - d).num_days().max(0)).unwrap_or(0) });
        match parties.iter_mut().find(|p| p.0 == pid) {
            Some(p) => {
                p.2[b] += left;
                p.3.push(doc);
            }
            None => {
                let mut amounts = [Decimal::ZERO; 5];
                amounts[b] = left;
                parties.push((pid, pname, amounts, vec![doc]));
            }
        }
    }
    let to_obj = |a: &[Decimal; 5]| {
        let mut m = serde_json::Map::new();
        for (i, k) in BUCKETS.iter().enumerate() {
            m.insert((*k).into(), json!(a[i]));
        }
        m.insert("total".into(), json!(a.iter().sum::<Decimal>()));
        Value::Object(m)
    };
    Ok(Json(json!({
        "kind": kind, "as_of": as_of, "totals": to_obj(&totals),
        "parties": parties.iter().map(|p| json!({ "id": p.0, "name": p.1, "amounts": to_obj(&p.2), "documents": p.3 })).collect::<Vec<_>>(),
    })))
}

// ---------------------------------------------------------------- tax reports

#[derive(Deserialize)]
struct MonthQ {
    year: Option<i32>,
    month: Option<u32>,
}

fn month_range(st: &AppState, q: &MonthQ) -> AppResult<(NaiveDate, NaiveDate)> {
    let today = local_today(st.cfg.tz_offset_min);
    let start = NaiveDate::from_ymd_opt(q.year.unwrap_or(today.year()), q.month.unwrap_or(today.month()), 1).ok_or_else(|| bad("เดือน/ปีไม่ถูกต้อง"))?;
    Ok((start, start.checked_add_months(chrono::Months::new(1)).unwrap().pred_opt().unwrap()))
}

type TaxRow = (NaiveDate, String, String, Option<String>, Option<String>, Decimal, Decimal);

async fn vat_lists(st: &AppState, from: NaiveDate, to: NaiveDate) -> AppResult<(Vec<TaxRow>, Vec<TaxRow>)> {
    // output tax: issued tax invoices of the month (void ones stay in the list at zero, as the law requires the number to be accounted for)
    let output: Vec<TaxRow> = sqlx::query_as(
        "SELECT doc_date, COALESCE(doc_no, ''), CASE WHEN status = 'void' THEN CONCAT(customer_name, ' (ยกเลิก)') ELSE customer_name END,
                customer_tax_id, customer_branch,
                CASE WHEN status = 'void' THEN 0 ELSE vat_base END, CASE WHEN status = 'void' THEN 0 ELSE vat_amount END
           FROM sales_docs WHERE doc_type = 'invoice' AND doc_no IS NOT NULL AND vat_mode <> 'none' AND doc_date BETWEEN ? AND ?
          ORDER BY doc_date, doc_no",
    )
    .bind(from)
    .bind(to)
    .fetch_all(&st.db)
    .await?;
    // input tax: supplier tax invoices received + VAT on approved expense claims
    let mut input: Vec<TaxRow> = sqlx::query_as(
        "SELECT b.bill_date, COALESCE(b.supplier_invoice_no, b.bill_no), s.name, s.tax_id,
                CASE WHEN s.branch_code IS NULL OR s.branch_code = '00000' THEN 'สำนักงานใหญ่' ELSE CONCAT('สาขา ', s.branch_code) END,
                b.vat_base, b.vat_amount
           FROM purchase_bills b JOIN suppliers s ON s.id = b.supplier_id
          WHERE b.status <> 'void' AND b.vat_amount > 0 AND b.bill_date BETWEEN ? AND ? ORDER BY b.bill_date, b.id",
    )
    .bind(from)
    .bind(to)
    .fetch_all(&st.db)
    .await?;
    let claims: Vec<TaxRow> = sqlx::query_as(
        "SELECT e.entry_date, c.claim_no, CONCAT('ใบเบิก: ', i.description), NULL, NULL, i.amount - i.vat_amount, i.vat_amount
           FROM expense_items i JOIN expense_claims c ON c.id = i.claim_id JOIN journal_entries e ON e.id = c.journal_entry_id
          WHERE i.vat_amount > 0 AND e.status = 'posted' AND e.entry_date BETWEEN ? AND ? ORDER BY e.entry_date",
    )
    .bind(from)
    .bind(to)
    .fetch_all(&st.db)
    .await?;
    input.extend(claims);
    Ok((output, input))
}

/// VAT for the month (ภ.พ.30): totals from the ledger, lists from the documents.
async fn vat_report(State(st): State<AppState>, u: AuthUser, Query(q): Query<MonthQ>) -> AppResult<Json<Value>> {
    view(&u)?;
    let (from, to) = month_range(&st, &q)?;
    let gl = |role: &'static str| {
        let st = st.clone();
        async move {
            sqlx::query_scalar::<_, Decimal>(
                "SELECT COALESCE(SUM(l.credit - l.debit), 0) FROM journal_lines l JOIN journal_entries e ON e.id = l.entry_id
                   JOIN accounts a ON a.id = l.account_id WHERE a.system_role = ? AND e.status = 'posted' AND e.entry_date BETWEEN ? AND ?",
            )
            .bind(role)
            .bind(from)
            .bind(to)
            .fetch_one(&st.db)
            .await
        }
    };
    let output_vat = gl("vat_output").await?;
    let input_vat = -gl("vat_input").await?;
    let (out_rows, in_rows) = vat_lists(&st, from, to).await?;
    let row = |r: &TaxRow| json!({ "date": r.0, "doc_no": r.1, "name": r.2, "tax_id": r.3, "branch": r.4, "base": r.5, "vat": r.6 });
    Ok(Json(json!({
        "from": from, "to": to, "output_vat": output_vat, "input_vat": input_vat, "net_vat": output_vat - input_vat,
        "output_base": out_rows.iter().map(|r| r.5).sum::<Decimal>(), "input_base": in_rows.iter().map(|r| r.5).sum::<Decimal>(),
        "output": out_rows.iter().map(row).collect::<Vec<_>>(), "input": in_rows.iter().map(row).collect::<Vec<_>>(),
        "due": "ยื่น ภ.พ.30 และชำระภาษีภายในวันที่ 15 ของเดือนถัดไป (ยื่นออนไลน์ได้ถึงวันที่ 23)",
    })))
}

type WhtRow = (NaiveDate, String, String, Option<String>, String, Option<String>, Decimal, Decimal, Decimal);

async fn wht_rows(st: &AppState, from: NaiveDate, to: NaiveDate) -> AppResult<Vec<WhtRow>> {
    Ok(sqlx::query_as(
        "SELECT p.pay_date, COALESCE(p.wht_cert_no, p.voucher_no), s.name, s.tax_id, s.kind, p.wht_income_type, p.wht_rate,
                CASE WHEN p.wht_rate > 0 THEN ROUND(p.wht_amount * 100 / p.wht_rate, 2) ELSE 0 END, p.wht_amount
           FROM purchase_payments p JOIN purchase_bills b ON b.id = p.bill_id JOIN suppliers s ON s.id = b.supplier_id
          WHERE p.status = 'active' AND p.wht_amount > 0 AND p.pay_date BETWEEN ? AND ? ORDER BY p.pay_date, p.id",
    )
    .bind(from)
    .bind(to)
    .fetch_all(&st.db)
    .await?)
}

/// Tax withheld in the month: ภ.ง.ด.1 (salaries), ภ.ง.ด.3 (individuals), ภ.ง.ด.53 (companies).
async fn wht_report(State(st): State<AppState>, u: AuthUser, Query(q): Query<MonthQ>) -> AppResult<Json<Value>> {
    view(&u)?;
    let (from, to) = month_range(&st, &q)?;
    let rows = wht_rows(&st, from, to).await?;
    let (pnd1_income, pnd1_tax, pnd1_n): (Decimal, Decimal, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(s.taxable_income), 0), COALESCE(SUM(s.wht), 0), CAST(COALESCE(SUM(s.wht > 0), 0) AS SIGNED)
           FROM payslips s JOIN payroll_runs r ON r.id = s.run_id WHERE r.status = 'paid' AND r.pay_date BETWEEN ? AND ?",
    )
    .bind(from)
    .bind(to)
    .fetch_one(&st.db)
    .await?;
    let group = |kind: &str| {
        let list: Vec<Value> = rows
            .iter()
            .filter(|r| r.4 == kind)
            .map(|r| json!({ "date": r.0, "cert_no": r.1, "name": r.2, "tax_id": r.3, "income_type": r.5, "rate": r.6, "base": r.7, "tax": r.8 }))
            .collect();
        let base: Decimal = rows.iter().filter(|r| r.4 == kind).map(|r| r.7).sum();
        let tax: Decimal = rows.iter().filter(|r| r.4 == kind).map(|r| r.8).sum();
        json!({ "rows": list, "base": base, "tax": tax })
    };
    Ok(Json(json!({
        "from": from, "to": to,
        "pnd1": { "income": pnd1_income, "tax": pnd1_tax, "people": pnd1_n },
        "pnd3": group("individual"), "pnd53": group("company"),
        "due": "ยื่น ภ.ง.ด.1 / 3 / 53 และนำส่งภาษีภายในวันที่ 7 ของเดือนถัดไป (ยื่นออนไลน์ได้ถึงวันที่ 15)",
    })))
}

// ---------------------------------------------------------------- Excel

fn xlsx_response(name: &str, sheets: &[Sheet]) -> Response {
    (
        [
            (header::CONTENT_TYPE, "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\"")),
        ],
        xlsx::workbook(sheets),
    )
        .into_response()
}

async fn financials_xlsx(State(st): State<AppState>, u: AuthUser, Query(q): Query<PeriodQ>) -> AppResult<Response> {
    view(&u)?;
    let (from, to) = period(&st, &q)?;
    let mut tb = Sheet::new("งบทดลอง", &["รหัส", "ชื่อบัญชี", "ยกมา", "เดบิต", "เครดิต", "คงเหลือ"]);
    for r in balances(&st, from, to).await? {
        if r.opening.is_zero() && r.debit.is_zero() && r.credit.is_zero() {
            continue;
        }
        let closing = r.opening + r.debit - r.credit;
        tb.push(vec![r.code.clone().into(), r.name.clone().into(), f(r.opening).into(), f(r.debit).into(), f(r.credit).into(), f(closing).into()]);
    }
    let pl = pl_data(&st, from, to).await?;
    let mut ps = Sheet::new("งบกำไรขาดทุน", &["หมวด", "รหัส", "ชื่อบัญชี", "จำนวนเงิน"]);
    for (label, rows) in [("รายได้", &pl.revenue), ("ต้นทุนขาย", &pl.cogs), ("ค่าใช้จ่าย", &pl.expenses)] {
        for (r, a) in rows.iter() {
            ps.push(vec![label.into(), r.code.clone().into(), r.name.clone().into(), f(*a).into()]);
        }
        ps.push(vec![format!("รวม{label}").into(), Cell::Empty, Cell::Empty, f(Pl::total(rows)).into()]);
    }
    ps.push(vec!["กำไร (ขาดทุน) สุทธิ".into(), Cell::Empty, Cell::Empty, f(pl.net()).into()]);
    let bs = bs_data(&st, to).await?;
    let mut bsh = Sheet::new("งบแสดงฐานะการเงิน", &["หมวด", "รหัส", "ชื่อบัญชี", "จำนวนเงิน"]);
    for (label, rows) in [("สินทรัพย์", &bs.assets), ("หนี้สิน", &bs.liabilities), ("ส่วนของเจ้าของ", &bs.equity)] {
        for (r, a) in rows.iter() {
            bsh.push(vec![label.into(), r.code.clone().into(), r.name.clone().into(), f(*a).into()]);
        }
        if label == "ส่วนของเจ้าของ" {
            bsh.push(vec![label.into(), Cell::Empty, "กำไร (ขาดทุน) ที่ยังไม่ได้ปิดบัญชี".into(), f(bs.earnings).into()]);
        }
    }
    audit(&st, Some(&u), "export_xlsx", "financials", format!("{from}..{to}"), json!({})).await;
    Ok(xlsx_response(&format!("financials-{from}-{to}.xlsx"), &[tb, ps, bsh]))
}

async fn tax_xlsx(State(st): State<AppState>, u: AuthUser, Query(q): Query<MonthQ>) -> AppResult<Response> {
    view(&u)?;
    let (from, to) = month_range(&st, &q)?;
    let (out_rows, in_rows) = vat_lists(&st, from, to).await?;
    let vat_sheet = |name: &str, rows: &[TaxRow]| {
        let mut sh = Sheet::new(name, &["ลำดับ", "วันที่", "เลขที่ใบกำกับภาษี", "ชื่อ", "เลขประจำตัวผู้เสียภาษี", "สาขา", "มูลค่าสินค้า/บริการ", "ภาษีมูลค่าเพิ่ม"]);
        for (i, r) in rows.iter().enumerate() {
            sh.push(vec![((i + 1) as i64).into(), th_short(r.0).into(), r.1.clone().into(), r.2.clone().into(),
                r.3.clone().into(), r.4.clone().into(), f(r.5).into(), f(r.6).into()]);
        }
        sh
    };
    let mut wht = Sheet::new("ภงด3-53", &["แบบ", "วันที่จ่าย", "เลขที่ 50 ทวิ", "ผู้ถูกหักภาษี", "เลขประจำตัวผู้เสียภาษี", "ประเภทเงินได้", "อัตรา %", "เงินได้", "ภาษีที่หัก"]);
    for r in wht_rows(&st, from, to).await? {
        wht.push(vec![(if r.4 == "individual" { "ภ.ง.ด.3" } else { "ภ.ง.ด.53" }).into(), th_short(r.0).into(), r.1.into(),
            r.2.into(), r.3.into(), r.5.into(), f(r.6).into(), f(r.7).into(), f(r.8).into()]);
    }
    audit(&st, Some(&u), "export_xlsx", "tax_reports", from.to_string(), json!({})).await;
    Ok(xlsx_response(&format!("tax-{}.xlsx", from.format("%Y-%m")), &[vat_sheet("รายงานภาษีขาย", &out_rows), vat_sheet("รายงานภาษีซื้อ", &in_rows), wht]))
}

/// 05/03/2569
fn th_short(d: NaiveDate) -> String {
    format!("{}/{}", d.format("%d/%m"), d.year() + 543)
}
