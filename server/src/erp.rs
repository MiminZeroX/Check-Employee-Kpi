//! Shared ERP building blocks: money rounding, running document numbers,
//! double-entry journal posting, Thai personal income tax and amounts in Thai words.

use chrono::{Datelike, NaiveDate};
use rust_decimal::{prelude::*, Decimal, RoundingStrategy};
use rust_decimal_macros::dec;
use sqlx::{MySql, Transaction};

use crate::error::{bad, AppError, AppResult};

pub type Tx<'a> = Transaction<'a, MySql>;

/// Rounds to satang (2 decimals, half away from zero).
pub fn r2(v: Decimal) -> Decimal {
    v.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero)
}

/// Rounds to whole baht (social security contributions).
pub fn r0(v: Decimal) -> Decimal {
    v.round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
}

pub fn d(v: f64) -> Decimal {
    Decimal::from_f64(v).unwrap_or_default()
}

/// Thai Buddhist year used in document numbers.
pub fn be_year(date: NaiveDate) -> i32 {
    date.year() + 543
}

/// Next running number: e.g. ("IV", 2026-03-05) -> "IV2569-00001". Must run inside the
/// caller's transaction so a rolled-back document does not consume a number.
pub async fn next_doc_no(tx: &mut Tx<'_>, doc_type: &str, date: NaiveDate) -> AppResult<String> {
    let year = be_year(date);
    sqlx::query("INSERT IGNORE INTO doc_sequences (doc_type, year, last_no) VALUES (?, ?, 0)")
        .bind(doc_type)
        .bind(year)
        .execute(&mut **tx)
        .await?;
    // the UPDATE locks the row until commit, so concurrent documents get different numbers
    sqlx::query("UPDATE doc_sequences SET last_no = last_no + 1 WHERE doc_type = ? AND year = ?")
        .bind(doc_type)
        .bind(year)
        .execute(&mut **tx)
        .await?;
    let (n,): (i32,) = sqlx::query_as("SELECT last_no FROM doc_sequences WHERE doc_type = ? AND year = ?")
        .bind(doc_type)
        .bind(year)
        .fetch_one(&mut **tx)
        .await?;
    Ok(format!("{doc_type}{year}-{n:05}"))
}

/// An account given by its system role ("cash", "vat_output", ...) or by id.
#[derive(Clone, Debug)]
pub enum Acc {
    Role(&'static str),
    Id(i64),
}

#[derive(Clone, Debug)]
pub struct Line {
    pub acc: Acc,
    pub debit: Decimal,
    pub credit: Decimal,
    pub memo: Option<String>,
    pub party: Option<(&'static str, i64)>,
}

impl Line {
    pub fn dr(acc: Acc, amount: Decimal) -> Self {
        Self { acc, debit: amount, credit: Decimal::ZERO, memo: None, party: None }
    }
    pub fn cr(acc: Acc, amount: Decimal) -> Self {
        Self { acc, debit: Decimal::ZERO, credit: amount, memo: None, party: None }
    }
    pub fn party(mut self, kind: &'static str, id: i64) -> Self {
        self.party = Some((kind, id));
        self
    }
    pub fn memo(mut self, m: impl Into<String>) -> Self {
        self.memo = Some(m.into());
        self
    }
}

pub async fn account_id(tx: &mut Tx<'_>, acc: &Acc) -> AppResult<i64> {
    match acc {
        Acc::Id(i) => Ok(*i),
        Acc::Role(r) => sqlx::query_scalar("SELECT id FROM accounts WHERE system_role = ?")
            .bind(*r)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(|| bad(format!("ไม่พบบัญชีสำหรับ '{r}' ในผังบัญชี"))),
    }
}

/// Postings on or before this date are refused (closed period).
pub async fn lock_date(tx: &mut Tx<'_>) -> AppResult<Option<NaiveDate>> {
    let v: Option<String> = sqlx::query_scalar("SELECT setting_value FROM system_settings WHERE setting_key = 'accounting_lock_date'")
        .fetch_optional(&mut **tx)
        .await?;
    Ok(v.and_then(|s| serde_json::from_str::<String>(&s).ok()).and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok()))
}

pub async fn check_open(tx: &mut Tx<'_>, date: NaiveDate) -> AppResult<()> {
    if let Some(l) = lock_date(tx).await? {
        if date <= l {
            return Err(bad(format!("งวดบัญชีถึงวันที่ {l} ปิดแล้ว ไม่สามารถบันทึกรายการวันที่ {date} ได้")));
        }
    }
    Ok(())
}

/// Posts a balanced journal entry and returns its id. Zero lines are dropped;
/// lines on the same account / side / party are kept separate for a readable ledger.
pub async fn post(
    tx: &mut Tx<'_>,
    date: NaiveDate,
    memo: &str,
    source: &str,
    source_id: Option<i64>,
    lines: Vec<Line>,
    user: Option<i64>,
) -> AppResult<i64> {
    check_open(tx, date).await?;
    let lines: Vec<Line> = lines
        .into_iter()
        .map(|mut l| {
            l.debit = r2(l.debit);
            l.credit = r2(l.credit);
            // a negative amount on one side is a positive amount on the other
            if l.debit < Decimal::ZERO {
                l.credit -= l.debit;
                l.debit = Decimal::ZERO;
            }
            if l.credit < Decimal::ZERO {
                l.debit -= l.credit;
                l.credit = Decimal::ZERO;
            }
            l
        })
        .filter(|l| !l.debit.is_zero() || !l.credit.is_zero())
        .collect();
    let dr: Decimal = lines.iter().map(|l| l.debit).sum();
    let cr: Decimal = lines.iter().map(|l| l.credit).sum();
    if dr != cr {
        return Err(AppError::Internal(anyhow::anyhow!("unbalanced journal entry ({source}): debit {dr} credit {cr}")));
    }
    if dr.is_zero() {
        return Err(bad("รายการบัญชีไม่มียอดเงิน"));
    }
    let no = next_doc_no(tx, "JV", date).await?;
    let id = sqlx::query(
        "INSERT INTO journal_entries (entry_no, entry_date, memo, source, source_id, created_by) VALUES (?,?,?,?,?,?)",
    )
    .bind(&no)
    .bind(date)
    .bind(memo.chars().take(300).collect::<String>())
    .bind(source)
    .bind(source_id)
    .bind(user)
    .execute(&mut **tx)
    .await?
    .last_insert_id() as i64;
    for l in lines {
        let acc = account_id(tx, &l.acc).await?;
        sqlx::query(
            "INSERT INTO journal_lines (entry_id, account_id, debit, credit, memo, party_type, party_id) VALUES (?,?,?,?,?,?,?)",
        )
        .bind(id)
        .bind(acc)
        .bind(l.debit)
        .bind(l.credit)
        .bind(l.memo)
        .bind(l.party.map(|p| p.0))
        .bind(l.party.map(|p| p.1))
        .execute(&mut **tx)
        .await?;
    }
    Ok(id)
}

/// Marks the journal entries of a document as void (they disappear from all reports).
pub async fn void_entries(tx: &mut Tx<'_>, source: &str, source_id: i64) -> AppResult<u64> {
    // voiding is a change to the books too: not allowed inside a closed period
    let dates: Vec<(NaiveDate,)> =
        sqlx::query_as("SELECT entry_date FROM journal_entries WHERE source = ? AND source_id = ? AND status = 'posted'")
            .bind(source)
            .bind(source_id)
            .fetch_all(&mut **tx)
            .await?;
    for (dt,) in &dates {
        check_open(tx, *dt).await?;
    }
    Ok(sqlx::query("UPDATE journal_entries SET status = 'void' WHERE source = ? AND source_id = ? AND status = 'posted'")
        .bind(source)
        .bind(source_id)
        .execute(&mut **tx)
        .await?
        .rows_affected())
}

// ---------------------------------------------------------------- Thai personal income tax

/// Progressive rates on net income (Revenue Code s.48).
pub fn thai_income_tax(net: Decimal) -> Decimal {
    const BRACKETS: [(i64, i64); 8] = [
        (150_000, 0),
        (300_000, 5),
        (500_000, 10),
        (750_000, 15),
        (1_000_000, 20),
        (2_000_000, 25),
        (5_000_000, 30),
        (i64::MAX, 35),
    ];
    let mut tax = Decimal::ZERO;
    let mut lower = Decimal::ZERO;
    for (upper, rate) in BRACKETS {
        if net <= lower {
            break;
        }
        let upper = Decimal::from(upper);
        let part = net.min(upper) - lower;
        tax += part * Decimal::from(rate) / dec!(100);
        lower = upper;
    }
    tax
}

/// Yearly tax for employment income (40(1)): 50% expense deduction up to 100,000,
/// personal allowance 60,000, plus social security, provident fund and other allowances.
pub fn annual_employment_tax(annual_income: Decimal, deductions: Decimal) -> Decimal {
    let expense = (annual_income * dec!(0.5)).min(dec!(100000));
    let net = annual_income - expense - dec!(60000) - deductions;
    thai_income_tax(net.max(Decimal::ZERO))
}

// ---------------------------------------------------------------- amount in Thai words

const DIGITS: [&str; 10] = ["ศูนย์", "หนึ่ง", "สอง", "สาม", "สี่", "ห้า", "หก", "เจ็ด", "แปด", "เก้า"];
const PLACES: [&str; 6] = ["", "สิบ", "ร้อย", "พัน", "หมื่น", "แสน"];

fn words_below_million(n: u64) -> String {
    let s: Vec<u32> = n.to_string().chars().map(|c| c.to_digit(10).unwrap()).collect();
    let len = s.len();
    let mut out = String::new();
    for (i, &dg) in s.iter().enumerate() {
        let place = len - i - 1;
        if dg == 0 {
            continue;
        }
        match (place, dg) {
            (0, 1) if len > 1 => out.push_str("เอ็ด"),
            (1, 1) => {}
            (1, 2) => out.push_str("ยี่"),
            _ => out.push_str(DIGITS[dg as usize]),
        }
        out.push_str(PLACES[place]);
    }
    out
}

fn int_words(n: u64) -> String {
    if n == 0 {
        return DIGITS[0].to_string();
    }
    let mut parts = Vec::new();
    let mut n = n;
    while n > 0 {
        parts.push(n % 1_000_000);
        n /= 1_000_000;
    }
    let mut out = String::new();
    for (i, p) in parts.iter().enumerate().rev() {
        if *p > 0 {
            out.push_str(&words_below_million(*p));
        }
        if i > 0 {
            out.push_str("ล้าน");
        }
    }
    out
}

/// 1234.50 -> "หนึ่งพันสองร้อยสามสิบสี่บาทห้าสิบสตางค์"
pub fn baht_text(v: Decimal) -> String {
    let v = r2(v.abs());
    let baht = v.trunc().to_u64().unwrap_or(0);
    let satang = ((v - v.trunc()) * dec!(100)).round().to_u64().unwrap_or(0);
    let mut s = String::new();
    if baht > 0 || satang == 0 {
        s.push_str(&int_words(baht));
        s.push_str("บาท");
    }
    if satang == 0 {
        s.push_str("ถ้วน");
    } else {
        s.push_str(&words_below_million(satang));
        s.push_str("สตางค์");
    }
    s
}

/// 1234567.5 -> "1,234,567.50"
pub fn fmt_money(v: Decimal) -> String {
    let v = r2(v);
    let neg = v.is_sign_negative() && !v.is_zero();
    let s = format!("{:.2}", v.abs());
    let (i, f) = s.split_once('.').unwrap_or((&s, "00"));
    let mut out = String::new();
    for (n, c) in i.chars().enumerate() {
        if n > 0 && (i.len() - n) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    format!("{}{out}.{f}", if neg { "-" } else { "" })
}

pub fn parse_date(s: &str) -> AppResult<NaiveDate> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").map_err(|_| bad(format!("วันที่ไม่ถูกต้อง: {s}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baht_words() {
        assert_eq!(baht_text(dec!(0)), "ศูนย์บาทถ้วน");
        assert_eq!(baht_text(dec!(1)), "หนึ่งบาทถ้วน");
        assert_eq!(baht_text(dec!(11)), "สิบเอ็ดบาทถ้วน");
        assert_eq!(baht_text(dec!(21)), "ยี่สิบเอ็ดบาทถ้วน");
        assert_eq!(baht_text(dec!(101)), "หนึ่งร้อยเอ็ดบาทถ้วน");
        assert_eq!(baht_text(dec!(1234.50)), "หนึ่งพันสองร้อยสามสิบสี่บาทห้าสิบสตางค์");
        assert_eq!(baht_text(dec!(1000000)), "หนึ่งล้านบาทถ้วน");
        assert_eq!(baht_text(dec!(21000011.25)), "ยี่สิบเอ็ดล้านสิบเอ็ดบาทยี่สิบห้าสตางค์");
        assert_eq!(baht_text(dec!(0.75)), "เจ็ดสิบห้าสตางค์");
    }

    #[test]
    fn income_tax() {
        assert_eq!(thai_income_tax(dec!(150000)), dec!(0));
        assert_eq!(thai_income_tax(dec!(300000)), dec!(7500));
        assert_eq!(thai_income_tax(dec!(500000)), dec!(27500));
        assert_eq!(thai_income_tax(dec!(1000000)), dec!(115000));
        // salary 50,000/month, SSO 875/month: 600,000 - 100,000 - 60,000 - 10,500 = 429,500 -> 7,500 + 12,950
        assert_eq!(annual_employment_tax(dec!(600000), dec!(10500)), dec!(20450));
        assert_eq!(annual_employment_tax(dec!(180000), dec!(9000)), dec!(0));
    }

    #[test]
    fn money_format() {
        assert_eq!(fmt_money(dec!(1234567.5)), "1,234,567.50");
        assert_eq!(fmt_money(dec!(-12.345)), "-12.35");
        assert_eq!(fmt_money(dec!(0)), "0.00");
        assert_eq!(r0(dec!(874.5)), dec!(875));
    }
}
