//! Report Center: A4 employee KPI report (HTML ready for print / Save as PDF) and CSV exports.

use axum::{
    extract::{Path, Query, State},
    http::header,
    response::{Html, IntoResponse, Response},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde::Serialize;
use serde_json::json;

use crate::{
    auth::{audit, AuthUser},
    error::{AppError, AppResult},
    routes::{
        activity::{daily_rows, summaries, DailyRow, SummaryRow},
        kpi::{compute, KpiResult},
        org::{Employee, EMPLOYEE_SELECT},
        Range,
    },
    state::AppState,
    util::{csv_field, date_range, fmt_duration, html_escape as esc, setting_str, state_label, to_local},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/reports/employee/{id}", get(employee_report))
        .route("/reports/employee/{id}/csv", get(employee_csv))
        .route("/reports/summary.csv", get(summary_csv))
        .route("/reports/batch", get(batch_report))
        .route("/reports/history", get(history))
}

const STATE_COLORS: [(&str, &str); 6] = [
    ("focused", "#0f9f8f"),
    ("present", "#3b9ee8"),
    ("phone", "#e8a23b"),
    ("looking_away", "#9b7be0"),
    ("away", "#a3adbb"),
    ("unknown", "#d9dee5"),
];

fn thai_date(d: NaiveDate) -> String {
    format!("{:02}/{:02}/{}", d.day(), d.month(), d.year() + 543)
}

fn can_view(u: &AuthUser, emp: i64) -> AppResult<()> {
    if u.role() == "employee" {
        if u.0.emp == Some(emp) {
            Ok(())
        } else {
            Err(AppError::Forbidden)
        }
    } else {
        u.require(&["hr", "manager"])
    }
}

fn sec_of(s: &SummaryRow, k: &str) -> i64 {
    match k {
        "focused" => s.focused_sec,
        "present" => s.present_sec,
        "phone" => s.phone_sec,
        "looking_away" => s.looking_away_sec,
        "away" => s.away_sec,
        _ => s.unknown_sec,
    }
}

fn activity_bar(s: &SummaryRow) -> String {
    let total: i64 = STATE_COLORS.iter().map(|(k, _)| sec_of(s, k)).sum();
    if total == 0 {
        return "<p class=\"muted\">ไม่มีข้อมูลกิจกรรมจากกล้องในช่วงเวลานี้</p>".into();
    }
    let mut x = 0.0;
    let mut rects = String::new();
    let mut legend = String::new();
    for (k, c) in STATE_COLORS {
        let v = sec_of(s, k);
        let w = v as f64 * 100.0 / total as f64;
        if w > 0.0 {
            rects.push_str(&format!("<rect x=\"{x:.2}%\" y=\"0\" width=\"{w:.2}%\" height=\"18\" fill=\"{c}\"/>"));
        }
        x += w;
        legend.push_str(&format!(
            "<tr><td><span class=\"sw\" style=\"background:{c}\"></span>{}</td><td class=\"r\">{}</td><td class=\"r\">{:.1}%</td></tr>",
            state_label(k),
            fmt_duration(v),
            w
        ));
    }
    format!(
        "<svg width=\"100%\" height=\"18\" class=\"bar\" role=\"img\" aria-label=\"สัดส่วนกิจกรรม\">{rects}</svg>
         <table class=\"t compact\"><thead><tr><th>กิจกรรมที่ระบบสังเกตได้</th><th class=\"r\">ระยะเวลา</th><th class=\"r\">สัดส่วน</th></tr></thead><tbody>{legend}</tbody></table>"
    )
}

fn trend_chart(days: &[DailyRow]) -> String {
    if days.is_empty() {
        return "<p class=\"muted\">ไม่มีข้อมูลรายวัน</p>".into();
    }
    let at_desk = |d: &DailyRow| (d.focused_sec + d.present_sec + d.phone_sec + d.looking_away_sec) as f64 / 3600.0;
    let max = days.iter().map(at_desk).fold(8.0_f64, f64::max).ceil();
    let n = days.len().min(62);
    let w = 680.0;
    let h = 120.0;
    let top = 10.0;
    let bw = (w - 30.0) / n as f64;
    let mut out = format!("<svg viewBox=\"0 {} {w} {}\" width=\"100%\" class=\"trend\" role=\"img\" aria-label=\"แนวโน้มรายวัน\">", -top, h + 22.0 + top);
    for g in [0.25, 0.5, 0.75, 1.0] {
        let y = h - h * g;
        out.push_str(&format!(
            "<line x1=\"30\" x2=\"{w}\" y1=\"{y:.1}\" y2=\"{y:.1}\" stroke=\"#e5e8ec\"/><text x=\"26\" y=\"{:.1}\" class=\"ax\" text-anchor=\"end\">{:.0}ช.</text>",
            y + 3.0,
            max * g
        ));
    }
    for (i, d) in days.iter().take(n).enumerate() {
        let x = 30.0 + i as f64 * bw + bw * 0.15;
        let focused = d.focused_sec as f64 / 3600.0;
        let total = at_desk(d);
        let ht = h * total / max;
        let hf = h * focused / max;
        out.push_str(&format!(
            "<rect x=\"{x:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{ht:.1}\" fill=\"#b9dcf5\"/><rect x=\"{x:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{hf:.1}\" fill=\"#0f9f8f\"/>",
            h - ht,
            bw * 0.7,
            h - hf,
            bw * 0.7
        ));
        if n <= 16 || i % (n / 10).max(1) == 0 {
            out.push_str(&format!(
                "<text x=\"{:.1}\" y=\"{}\" class=\"ax\" text-anchor=\"middle\">{}/{}</text>",
                x + bw * 0.35,
                h + 14.0,
                d.work_date.day(),
                d.work_date.month()
            ));
        }
    }
    out.push_str("</svg><div class=\"legend\"><span class=\"sw\" style=\"background:#0f9f8f\"></span>มองหน้าจอ / ทำงานที่โต๊ะ <span class=\"sw\" style=\"background:#b9dcf5;margin-left:12px\"></span>อยู่ที่โต๊ะทั้งหมด (ชั่วโมง/วัน)</div>");
    out
}

fn kpi_table(k: &KpiResult) -> String {
    let mut rows = String::new();
    for i in &k.items {
        let (score, bar) = match i.score {
            Some(s) => (
                format!("{s:.1}"),
                format!("<svg width=\"90\" height=\"8\"><rect width=\"90\" height=\"8\" fill=\"#eef1f4\"/><rect width=\"{:.1}\" height=\"8\" fill=\"#3b6fd8\"/></svg>", s * 0.9),
            ),
            None => ("-".into(), String::new()),
        };
        rows.push_str(&format!(
            "<tr><td>{}<div class=\"muted s\">{}</div></td><td class=\"r\">{:.0}%</td><td class=\"r\">{score}</td><td>{bar}</td></tr>",
            esc(&i.name),
            esc(&i.detail),
            i.weight
        ));
    }
    let total = k.total.map(|t| format!("{t:.1}")).unwrap_or_else(|| "-".into());
    let grade = k.grade.clone().unwrap_or_else(|| "ยังไม่มีข้อมูลเพียงพอ".into());
    format!(
        "<table class=\"t\"><thead><tr><th>ตัวชี้วัด (KPI)</th><th class=\"r\">น้ำหนัก</th><th class=\"r\">คะแนน</th><th></th></tr></thead>
         <tbody>{rows}</tbody><tfoot><tr><td><b>คะแนนรวมถ่วงน้ำหนัก</b></td><td></td><td class=\"r\"><b>{total}</b></td><td><b>{}</b></td></tr></tfoot></table>",
        esc(&grade)
    )
}

fn recommendations(k: &KpiResult, s: &SummaryRow, late_days: usize, days: usize) -> Vec<String> {
    let mut r = Vec::new();
    let item = |code: &str| k.items.iter().find(|i| i.source == code).and_then(|i| i.score);
    if let Some(t) = k.total {
        if t >= 85.0 {
            r.push("ผลการปฏิบัติงานโดยรวมอยู่ในเกณฑ์ดีมาก ควรรักษามาตรฐานและพิจารณามอบหมายงานที่ท้าทายมากขึ้น".into());
        } else if t < 70.0 {
            r.push("คะแนนรวมต่ำกว่าเป้าหมาย ควรจัดประชุมหารือร่วมกับหัวหน้างานเพื่อวางแผนพัฒนา (IDP) และหาสาเหตุที่แท้จริงก่อนสรุปผล".into());
        }
    }
    if let Some(v) = item("task_on_time") {
        if v < 80.0 {
            r.push("อัตราการส่งงานตรงเวลายังไม่ถึงเป้าหมาย ควรทบทวนการวางแผนงานและภาระงานที่ได้รับมอบหมาย".into());
        }
    }
    if let Some(v) = item("task_quality") {
        if v < 70.0 {
            r.push("ควรเพิ่มการ Review งานและการอบรมเพื่อพัฒนาคุณภาพของผลงาน".into());
        }
    }
    if days > 0 && late_days * 5 > days {
        r.push(format!("มาสาย {late_days} วันจาก {days} วัน ควรพูดคุยเพื่อหาสาเหตุ เช่น การเดินทางหรือตารางเวลา"));
    }
    if s.longest_sitting_sec > 90 * 60 {
        r.push(format!(
            "พบการนั่งทำงานต่อเนื่องนานสุด {} แนะนำให้ลุกเดินหรือยืดเส้นทุก 50-60 นาที และพักสายตาตามหลัก 20-20-20",
            fmt_duration(s.longest_sitting_sec)
        ));
    }
    if r.is_empty() {
        r.push("ผลการปฏิบัติงานอยู่ในเกณฑ์ปกติ ควรรักษาความสม่ำเสมอและความสมดุลระหว่างการทำงานและการพักผ่อน".into());
    }
    r
}

#[derive(Serialize, sqlx::FromRow)]
struct HistoryRow {
    id: i64,
    kind: String,
    employee_id: Option<i64>,
    employee_name: Option<String>,
    period_start: NaiveDate,
    period_end: NaiveDate,
    doc_no: Option<String>,
    generated_by_name: Option<String>,
    created_at: DateTime<Utc>,
}

async fn history(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Vec<HistoryRow>>> {
    u.require(&["hr", "manager"])?;
    Ok(Json(
        sqlx::query_as(
            "SELECT r.id, r.kind, r.employee_id, CONCAT(e.first_name, ' ', e.last_name) AS employee_name,
               r.period_start, r.period_end, r.doc_no, us.display_name AS generated_by_name, r.created_at
             FROM reports r LEFT JOIN employees e ON e.id = r.employee_id LEFT JOIN users us ON us.id = r.generated_by
             ORDER BY r.id DESC LIMIT 300",
        )
        .fetch_all(&st.db)
        .await?,
    ))
}

async fn employee_report(
    State(st): State<AppState>,
    u: AuthUser,
    Path(id): Path<i64>,
    Query(r): Query<Range>,
) -> AppResult<Html<String>> {
    can_view(&u, id)?;
    let (from, to) = date_range(r.from, r.to, st.cfg.tz_offset_min)?;
    Ok(Html(render_employee(&st, &u, id, from, to).await?))
}

/// All active employees (optionally one department) in a single printable document,
/// one A4 report per employee.
async fn batch_report(State(st): State<AppState>, u: AuthUser, Query(r): Query<Range>) -> AppResult<Html<String>> {
    u.require(&["hr", "manager"])?;
    let (from, to) = date_range(r.from, r.to, st.cfg.tz_offset_min)?;
    let ids: Vec<(i64,)> = match r.department_id {
        Some(d) => sqlx::query_as("SELECT id FROM employees WHERE status = 'active' AND department_id = ? ORDER BY employee_code")
            .bind(d)
            .fetch_all(&st.db)
            .await?,
        None => sqlx::query_as("SELECT id FROM employees WHERE status = 'active' ORDER BY employee_code").fetch_all(&st.db).await?,
    };
    if ids.len() > 300 {
        return Err(crate::error::bad("พนักงานมากเกินไปสำหรับรายงานฉบับเดียว (สูงสุด 300 คน) กรุณาเลือกแผนก"));
    }
    let mut head = String::new();
    let mut pages = String::new();
    for (id,) in ids {
        let html = render_employee(&st, &u, id, from, to).await?;
        if let (Some(a), Some(b)) = (html.find("<div class=\"page\">"), html.rfind("</body>")) {
            if head.is_empty() {
                head = html[..a].to_string();
            }
            pages.push_str(&html[a..b]);
        }
    }
    if head.is_empty() {
        return Ok(Html("<!doctype html><meta charset=utf-8><p>ไม่มีพนักงาน</p>".into()));
    }
    Ok(Html(format!("{head}{pages}</body></html>")))
}

async fn render_employee(st: &AppState, u: &AuthUser, id: i64, from: NaiveDate, to: NaiveDate) -> AppResult<String> {
    let st = st.clone();
    let u = u.clone();
    let off = st.cfg.tz_offset_min;
    let emp = sqlx::query_as::<_, Employee>(&format!("{EMPLOYEE_SELECT} WHERE e.id = ?"))
        .bind(id)
        .fetch_one(&st.db)
        .await?;
    let summary = summaries(&st, from, to, None, Some(id)).await?.into_iter().next().ok_or(AppError::NotFound)?;
    let days = daily_rows(&st, id, from, to).await?;
    let kpi = compute(&st, from, to, None, Some(id)).await?.into_iter().next().ok_or(AppError::NotFound)?;

    let org = setting_str(&st.db, "org_name", "").await;
    let header = setting_str(&st.db, "report_header", "รายงานผลการปฏิบัติงานและ KPI").await;
    let footer = setting_str(&st.db, "report_footer", "").await;
    let logo = setting_str(&st.db, "logo_url", "").await;

    let rid = sqlx::query("INSERT INTO reports (kind, employee_id, period_start, period_end, generated_by) VALUES ('employee_kpi', ?, ?, ?, ?)")
        .bind(id)
        .bind(from)
        .bind(to)
        .bind(u.id())
        .execute(&st.db)
        .await?
        .last_insert_id();
    let doc_no = format!("RPT-{}-{:06}", Utc::now().year() + 543, rid);
    sqlx::query("UPDATE reports SET doc_no = ? WHERE id = ?").bind(&doc_no).bind(rid).execute(&st.db).await?;
    audit(&st, Some(&u), "generate_report", "employee", id, json!({ "doc_no": doc_no, "from": from, "to": to })).await;

    // attendance statistics
    let present_days = days.iter().filter(|d| d.first_scan_at.is_some()).count();
    let late_days = days
        .iter()
        .filter_map(|d| d.first_scan_at)
        .filter(|f| to_local(*f, off).time() > emp.work_start)
        .count();
    let s2d: Vec<i32> = days.iter().filter_map(|d| d.scan_to_desk_sec).collect();
    let avg_s2d = if s2d.is_empty() { "-".to_string() } else { fmt_duration(s2d.iter().map(|v| *v as i64).sum::<i64>() / s2d.len() as i64) };
    let at_desk = summary.focused_sec + summary.present_sec + summary.phone_sec + summary.looking_away_sec;

    let mut day_rows = String::new();
    for d in &days {
        let t = |x: Option<DateTime<Utc>>| x.map(|v| to_local(v, off).format("%H:%M").to_string()).unwrap_or_else(|| "-".into());
        day_rows.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td class=\"r\">{}</td><td class=\"r\">{}</td><td class=\"r\">{}</td></tr>",
            thai_date(d.work_date),
            t(d.first_scan_at),
            t(d.first_desk_at),
            if d.last_scan_at != d.first_scan_at { t(d.last_scan_at) } else { "-".into() },
            d.scan_to_desk_sec.map(|v| fmt_duration(v as i64)).unwrap_or_else(|| "-".into()),
            fmt_duration((d.focused_sec + d.present_sec + d.phone_sec + d.looking_away_sec) as i64),
            fmt_duration(d.longest_sitting_sec as i64),
        ));
    }
    if day_rows.is_empty() {
        day_rows = "<tr><td colspan=\"7\" class=\"muted\">ไม่มีข้อมูล</td></tr>".into();
    }
    let recs: String = recommendations(&kpi, &summary, late_days, present_days)
        .iter()
        .map(|r| format!("<li>{}</li>", esc(r)))
        .collect();
    let logo_html = if logo.is_empty() { String::new() } else { format!("<img src=\"{}\" class=\"logo\" alt=\"\">", esc(&logo)) };
    let now_local = to_local(Utc::now(), off);
    let generated = format!("{} {}", thai_date(now_local.date()), now_local.format("%H:%M"));

    let html = format!(
        r#"<!doctype html><html lang="th"><head><meta charset="utf-8"><title>{doc_no} - {name}</title>
<link rel="preconnect" href="https://fonts.googleapis.com"><link href="https://fonts.googleapis.com/css2?family=Sarabun:wght@400;600;700&display=swap" rel="stylesheet">
<style>
@page {{ size: A4; margin: 14mm 13mm 16mm; }}
* {{ box-sizing: border-box; }}
body {{ font-family: "Sarabun", "Noto Sans Thai", Tahoma, sans-serif; color: #1c2430; font-size: 12.5px; margin: 0; background: #e9edf2; }}
.page {{ width: 210mm; min-height: 297mm; margin: 12px auto; background: #fff; padding: 14mm 13mm; box-shadow: 0 2px 12px rgba(0,0,0,.12); }}
header {{ display: flex; align-items: center; gap: 14px; border-bottom: 2px solid #1f3b64; padding-bottom: 10px; }}
.logo {{ height: 48px; }}
h1 {{ font-size: 18px; margin: 0; color: #1f3b64; }}
h2 {{ font-size: 13.5px; margin: 16px 0 6px; color: #1f3b64; border-left: 4px solid #3b6fd8; padding-left: 8px; }}
.org {{ font-size: 13px; color: #44516a; }}
.meta {{ margin-left: auto; text-align: right; font-size: 11.5px; color: #44516a; }}
.grid {{ display: grid; grid-template-columns: 1fr 1fr; gap: 4px 24px; margin-top: 10px; }}
.grid div span {{ color: #6a768a; display: inline-block; min-width: 92px; }}
.cards {{ display: grid; grid-template-columns: repeat(4, 1fr); gap: 8px; }}
.card {{ border: 1px solid #dfe4ea; border-radius: 6px; padding: 8px 10px; }}
.card b {{ display: block; font-size: 16px; color: #1f3b64; }}
.card small {{ color: #6a768a; }}
.t {{ width: 100%; border-collapse: collapse; margin-top: 6px; }}
.t th, .t td {{ border-bottom: 1px solid #e5e8ec; padding: 4px 6px; text-align: left; vertical-align: top; }}
.t th {{ background: #f3f6f9; font-weight: 600; }}
.t tfoot td {{ border-top: 2px solid #1f3b64; }}
.t.compact td {{ padding: 2px 6px; }}
.r {{ text-align: right !important; }}
.muted {{ color: #7b8798; }} .s {{ font-size: 11px; }}
.sw {{ display: inline-block; width: 10px; height: 10px; border-radius: 2px; margin-right: 6px; vertical-align: middle; }}
.bar {{ border-radius: 4px; overflow: hidden; display: block; margin: 6px 0; }}
.ax {{ font-size: 9px; fill: #7b8798; }}
.legend {{ font-size: 11px; color: #44516a; }}
.note {{ background: #f6f8fb; border: 1px solid #e2e7ee; border-radius: 6px; padding: 8px 10px; font-size: 11.5px; color: #44516a; margin-top: 8px; }}
.sign {{ display: grid; grid-template-columns: repeat(3, 1fr); gap: 18px; margin-top: 26px; text-align: center; }}
.sign div {{ padding-top: 34px; border-top: 1px dotted #7b8798; }}
.comment {{ border: 1px solid #dfe4ea; border-radius: 6px; min-height: 60px; margin-top: 6px; }}
footer {{ margin-top: 18px; font-size: 10.5px; color: #7b8798; border-top: 1px solid #e5e8ec; padding-top: 6px; display: flex; justify-content: space-between; }}
.toolbar {{ position: sticky; top: 0; background: #1f3b64; color: #fff; padding: 8px 16px; display: flex; gap: 8px; align-items: center; }}
.toolbar button {{ background: #fff; color: #1f3b64; border: 0; border-radius: 4px; padding: 6px 14px; font: inherit; cursor: pointer; }}
.avoid {{ break-inside: avoid; }}
.page + .page {{ break-before: page; }}
@media print {{ body {{ background: #fff; }} .page {{ margin: 0; box-shadow: none; width: auto; min-height: 0; padding: 0; }} .toolbar {{ display: none; }} }}
</style></head><body>
<div class="toolbar"><button onclick="window.print()">พิมพ์ / บันทึกเป็น PDF (A4)</button><span>{doc_no}</span></div>
<div class="page">
<header>{logo_html}<div><h1>{header}</h1><div class="org">{org}</div></div>
<div class="meta">เลขที่เอกสาร: <b>{doc_no}</b><br>ช่วงประเมิน: {from_th} - {to_th}<br>วันที่ออกรายงาน: {generated}</div></header>

<div class="grid">
<div><span>ชื่อ-สกุล</span> <b>{name}</b></div><div><span>รหัสพนักงาน</span> {code}</div>
<div><span>ตำแหน่ง</span> {position}</div><div><span>แผนก</span> {dept}</div>
<div><span>เวลาทำงาน</span> {ws} - {we} น.</div><div><span>ยินยอมการวิเคราะห์</span> {consent}</div>
</div>

<h2>1. สรุปผล KPI</h2>
<div class="cards">
<div class="card"><small>คะแนน KPI รวม</small><b>{kpi_total}</b><small>{kpi_grade}</small></div>
<div class="card"><small>วันที่มาทำงาน</small><b>{present_days} วัน</b><small>มาสาย {late_days} วัน</small></div>
<div class="card"><small>เวลาอยู่ที่โต๊ะรวม</small><b>{at_desk}</b><small>เฉลี่ย {avg_desk}/วัน</small></div>
<div class="card"><small>สแกนนิ้ว/หน้า ถึงโต๊ะ (เฉลี่ย)</small><b>{avg_s2d}</b><small>&nbsp;</small></div>
</div>
<div class="avoid">{kpi_table}</div>

<h2>2. สถิติการเข้างาน (เครื่องสแกนลายนิ้วมือ / ใบหน้า)</h2>
<table class="t"><thead><tr><th>วันที่</th><th>สแกนเข้า</th><th>ถึงโต๊ะ</th><th>สแกนออก</th><th class="r">สแกน→โต๊ะ</th><th class="r">อยู่ที่โต๊ะ</th><th class="r">นั่งต่อเนื่องนานสุด</th></tr></thead><tbody>{day_rows}</tbody></table>

<h2 class="avoid">3. สรุปกิจกรรมที่ระบบตรวจพบ (ข้อมูลประกอบ ไม่ใช้คำนวณคะแนน)</h2>
<div class="avoid">{activity}
<div class="note">ใช้โทรศัพท์ {phone_count} ครั้ง · ลุกจากโต๊ะ {away_count} ครั้ง · แจ้งเตือนสุขภาพ {alerts} ครั้ง · นั่งต่อเนื่องนานสุด {longest}<br>
ข้อมูลส่วนนี้เป็นสิ่งที่กล้องสังเกตได้เท่านั้น การมองออกจากจอ การใช้โทรศัพท์ หรือการไม่อยู่ที่โต๊ะ อาจเป็นส่วนหนึ่งของการปฏิบัติงาน (เช่น ประชุม ติดต่อลูกค้า) จึงไม่นำมาตัดสินผลงานโดยอัตโนมัติ และพนักงานสามารถขอตรวจสอบ/แก้ไขข้อมูลที่คลาดเคลื่อนได้</div></div>

<h2 class="avoid">4. แนวโน้มการทำงานรายวัน</h2>
<div class="avoid">{trend}</div>

<h2>5. ข้อเสนอแนะ</h2>
<ul>{recs}</ul>

<div class="avoid"><h2>6. ความเห็นของผู้ประเมิน</h2><div class="comment"></div>
<div class="sign"><div>พนักงาน<br>(&nbsp;{name}&nbsp;)<br>วันที่ ____/____/______</div><div>ผู้ประเมิน / หัวหน้างาน<br>(..................................)<br>วันที่ ____/____/______</div><div>ผู้อนุมัติ / ฝ่ายบุคคล<br>(..................................)<br>วันที่ ____/____/______</div></div></div>

<footer><span>{footer}</span><span>{doc_no}</span></footer>
</div></body></html>"#,
        doc_no = esc(&doc_no),
        name = esc(&format!("{} {}", emp.first_name, emp.last_name)),
        header = esc(&header),
        org = esc(&org),
        footer = esc(&footer),
        logo_html = logo_html,
        from_th = thai_date(from),
        to_th = thai_date(to),
        generated = generated,
        code = esc(&emp.employee_code),
        position = esc(emp.position.as_deref().unwrap_or("-")),
        dept = esc(emp.department_name.as_deref().unwrap_or("-")),
        ws = emp.work_start.format("%H:%M"),
        we = emp.work_end.format("%H:%M"),
        consent = if emp.monitoring_consent { "ยินยอมแล้ว" } else { "ยังไม่ได้บันทึก" },
        kpi_total = kpi.total.map(|t| format!("{t:.1}")).unwrap_or_else(|| "-".into()),
        kpi_grade = esc(kpi.grade.as_deref().unwrap_or("ข้อมูลไม่เพียงพอ")),
        present_days = present_days,
        late_days = late_days,
        at_desk = fmt_duration(at_desk),
        avg_desk = fmt_duration(if summary.days > 0 { at_desk / summary.days } else { 0 }),
        avg_s2d = avg_s2d,
        kpi_table = kpi_table(&kpi),
        day_rows = day_rows,
        activity = activity_bar(&summary),
        phone_count = summary.phone_count,
        away_count = summary.away_count,
        alerts = summary.wellness_alerts,
        longest = fmt_duration(summary.longest_sitting_sec),
        trend = trend_chart(&days),
        recs = recs,
    );
    Ok(html)
}

fn csv_response(name: &str, body: String) -> Response {
    // BOM so that Excel opens Thai text correctly
    let mut out = String::from("\u{feff}");
    out.push_str(&body);
    (
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\"")),
        ],
        out,
    )
        .into_response()
}

async fn employee_csv(
    State(st): State<AppState>,
    u: AuthUser,
    Path(id): Path<i64>,
    Query(r): Query<Range>,
) -> AppResult<Response> {
    can_view(&u, id)?;
    let off = st.cfg.tz_offset_min;
    let (from, to) = date_range(r.from, r.to, off)?;
    let days = daily_rows(&st, id, from, to).await?;
    let t = |x: Option<DateTime<Utc>>| x.map(|v| to_local(v, off).format("%H:%M:%S").to_string()).unwrap_or_default();
    let mut s = String::from(
        "date,first_scan,first_desk,last_scan,scan_to_desk_min,focused_min,present_min,phone_min,looking_away_min,away_min,unknown_min,phone_count,away_count,longest_sitting_min,wellness_alerts\n",
    );
    for d in days {
        s.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            d.work_date,
            t(d.first_scan_at),
            t(d.first_desk_at),
            if d.last_scan_at != d.first_scan_at { t(d.last_scan_at) } else { "-".into() },
            d.scan_to_desk_sec.map(|v| format!("{:.1}", v as f64 / 60.0)).unwrap_or_default(),
            d.focused_sec / 60,
            d.present_sec / 60,
            d.phone_sec / 60,
            d.looking_away_sec / 60,
            d.away_sec / 60,
            d.unknown_sec / 60,
            d.phone_count,
            d.away_count,
            d.longest_sitting_sec / 60,
            d.wellness_alerts
        ));
    }
    audit(&st, Some(&u), "export_csv", "employee", id, json!({ "from": from, "to": to })).await;
    Ok(csv_response(&format!("employee-{id}-{from}-{to}.csv"), s))
}

async fn summary_csv(State(st): State<AppState>, u: AuthUser, Query(r): Query<Range>) -> AppResult<Response> {
    u.require(&["hr", "manager"])?;
    let (from, to) = date_range(r.from, r.to, st.cfg.tz_offset_min)?;
    let rows = summaries(&st, from, to, r.department_id, None).await?;
    let kpis = compute(&st, from, to, r.department_id, None).await?;
    let mut s = String::from(
        "employee_code,name,department,position,days,kpi_total,kpi_grade,at_desk_hours,focused_hours,phone_min,away_min,avg_scan_to_desk_min,longest_sitting_min,wellness_alerts\n",
    );
    for row in rows {
        let k = kpis.iter().find(|k| k.employee_id == row.employee_id);
        let at_desk = row.focused_sec + row.present_sec + row.phone_sec + row.looking_away_sec;
        s.push_str(&format!(
            "{},{},{},{},{},{},{},{:.2},{:.2},{},{},{},{},{}\n",
            csv_field(&row.employee_code),
            csv_field(&row.name),
            csv_field(row.department_name.as_deref().unwrap_or("")),
            csv_field(row.position.as_deref().unwrap_or("")),
            row.days,
            k.and_then(|k| k.total).map(|t| format!("{t:.1}")).unwrap_or_default(),
            csv_field(k.and_then(|k| k.grade.as_deref()).unwrap_or("")),
            at_desk as f64 / 3600.0,
            row.focused_sec as f64 / 3600.0,
            row.phone_sec / 60,
            row.away_sec / 60,
            row.avg_scan_to_desk_sec.map(|v| format!("{:.1}", v as f64 / 60.0)).unwrap_or_default(),
            row.longest_sitting_sec / 60,
            row.wellness_alerts
        ));
    }
    audit(&st, Some(&u), "export_csv", "summary", format!("{from}..{to}"), json!({})).await;
    Ok(csv_response(&format!("kpi-summary-{from}-{to}.csv"), s))
}

