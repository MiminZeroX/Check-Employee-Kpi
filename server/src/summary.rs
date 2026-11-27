//! Background jobs: daily summary aggregation and data-retention cleanup.

use std::collections::HashMap;

use chrono::{DateTime, Datelike, Duration, NaiveDate, Timelike, Utc};

use crate::{
    schedule::{overlap, Calendar},
    state::AppState,
    util::{day_range, is_at_desk, local_today, setting_f64, to_local},
};

#[derive(Default, Debug)]
struct Acc {
    secs: HashMap<String, i64>,
    phone_count: i32,
    away_count: i32,
    meeting_count: i32,
    break_sec: i64,
    desk_starts: Vec<DateTime<Utc>>,
    run_start: Option<DateTime<Utc>>,
    run_end: Option<DateTime<Utc>>,
    longest: i64,
    /// lengths (seconds) of every continuous sitting run
    runs: Vec<i64>,
    first_scan: Option<DateTime<Utc>>,
    last_scan: Option<DateTime<Utc>>,
    office_entry: Option<DateTime<Utc>>,
    check_in: Option<DateTime<Utc>>,
    check_out: Option<DateTime<Utc>>,
    alerts: i64,
}

/// A break shorter than this does not end a "continuous sitting" run.
const SITTING_GAP_SEC: i64 = 120;

pub async fn recompute_day(state: &AppState, date: NaiveDate) -> anyhow::Result<usize> {
    let off = state.cfg.tz_offset_min;
    let (start, end) = day_range(date, off);
    let cal = Calendar::load(&state.db, off, date, date).await?;

    let events: Vec<(i64, String, DateTime<Utc>, DateTime<Utc>)> = sqlx::query_as(
        "SELECT employee_id, COALESCE(corrected_state, state), started_at, ended_at
         FROM activity_events
         WHERE employee_id IS NOT NULL AND review_status <> 'rejected'
           AND started_at < ? AND ended_at > ?
         ORDER BY employee_id, started_at",
    )
    .bind(end.naive_utc())
    .bind(start.naive_utc())
    .fetch_all(&state.db)
    .await?;

    // scans with the role of the terminal: entrance (entered the office) or office (check-in/out)
    let scans: Vec<(i64, DateTime<Utc>, String)> = sqlx::query_as(
        "SELECT a.employee_id, a.scan_time, COALESCE(d.location_role, 'office') FROM attendance_logs a
         LEFT JOIN devices d ON d.id = a.device_id
         WHERE a.employee_id IS NOT NULL AND a.scan_time >= ? AND a.scan_time < ? ORDER BY a.scan_time",
    )
    .bind(start.naive_utc())
    .bind(end.naive_utc())
    .fetch_all(&state.db)
    .await?;

    let alerts: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT employee_id, COUNT(*) FROM notifications
         WHERE kind = 'wellness_sitting' AND employee_id IS NOT NULL
           AND created_at >= ? AND created_at < ?
         GROUP BY employee_id",
    )
    .bind(start.naive_utc())
    .bind(end.naive_utc())
    .fetch_all(&state.db)
    .await?;

    // approved leave / off-site / overtime overlapping the day
    let requests: Vec<(i64, String, DateTime<Utc>, DateTime<Utc>, Option<f64>)> = sqlx::query_as(
        "SELECT employee_id, kind, start_at, end_at, hours FROM requests
         WHERE status = 'approved' AND start_at < ? AND end_at > ?",
    )
    .bind(end.naive_utc())
    .bind(start.naive_utc())
    .fetch_all(&state.db)
    .await?;

    let active: Vec<(i64,)> = sqlx::query_as("SELECT id FROM employees WHERE status = 'active'").fetch_all(&state.db).await?;

    let mut acc: HashMap<i64, Acc> = HashMap::new();
    for (emp, t, role) in scans {
        let a = acc.entry(emp).or_default();
        a.first_scan = Some(a.first_scan.map_or(t, |f| f.min(t)));
        a.last_scan = Some(a.last_scan.map_or(t, |l| l.max(t)));
        if role == "entrance" {
            a.office_entry = Some(a.office_entry.map_or(t, |f| f.min(t)));
        } else {
            a.check_in = Some(a.check_in.map_or(t, |f| f.min(t)));
            a.check_out = Some(a.check_out.map_or(t, |l| l.max(t)));
        }
    }
    for a in acc.values_mut() {
        // sites with only entrance terminals: the first entrance scan counts as check-in
        if a.check_in.is_none() {
            a.check_in = a.office_entry;
            a.check_out = a.last_scan;
        }
    }
    for (emp, n) in alerts {
        acc.entry(emp).or_default().alerts = n;
    }

    for (emp, st, s, e) in events {
        let s = s.max(start);
        let e = e.min(end);
        if e <= s {
            continue;
        }
        let dur = (e - s).num_seconds();
        let sched = cal.day(emp, date);
        let a = acc.entry(emp).or_default();
        *a.secs.entry(st.clone()).or_default() += dur;
        match st.as_str() {
            "phone" => a.phone_count += 1,
            "away" => {
                a.away_count += 1;
                // time away from the desk inside the shift's break window is a break, not absence
                if let (Some(bs), Some(be)) = (sched.break_start, sched.break_end) {
                    a.break_sec += overlap(s, e, bs, be);
                }
            }
            "meeting" => a.meeting_count += 1,
            _ => {}
        }
        if is_at_desk(&st) {
            a.desk_starts.push(s);
            match (a.run_start, a.run_end) {
                (Some(_), Some(re)) if (s - re).num_seconds() <= SITTING_GAP_SEC => {
                    a.run_end = Some(re.max(e));
                }
                _ => {
                    if let (Some(rs), Some(re)) = (a.run_start, a.run_end) {
                        a.runs.push((re - rs).num_seconds());
                    }
                    a.run_start = Some(s);
                    a.run_end = Some(e);
                }
            }
            if let (Some(rs), Some(re)) = (a.run_start, a.run_end) {
                a.longest = a.longest.max((re - rs).num_seconds());
            }
        }
    }

    let alert_sec = (setting_f64(&state.db, "sitting_alert_min", 50.0).await.max(10.0) * 60.0) as i64;
    let now = Utc::now();
    let n = acc.len();
    // every active employee gets a work_sessions row, with or without data
    for (emp,) in &active {
        acc.entry(*emp).or_default();
    }
    for (emp, mut a) in acc {
        if let (Some(rs), Some(re)) = (a.run_start, a.run_end) {
            a.runs.push((re - rs).num_seconds());
        }
        // a wellness alert is due for every sitting run longer than the threshold;
        // take the larger of that and the alerts actually pushed live
        let derived = a.runs.iter().filter(|r| **r >= alert_sec).count() as i64;
        a.alerts = a.alerts.max(derived);
        let anchor = a.check_in.or(a.first_scan);
        let first_desk = match anchor {
            Some(fs) => a.desk_starts.iter().copied().filter(|d| *d >= fs).min(),
            None => a.desk_starts.iter().copied().min(),
        };
        let scan_to_desk = match (anchor, first_desk) {
            (Some(fs), Some(fd)) => Some((fd - fs).num_seconds() as i32),
            _ => None,
        };
        let g = |k: &str| *a.secs.get(k).unwrap_or(&0) as i32;
        let desk_sec: i64 = ["focused", "present", "phone", "looking_away", "meeting"].iter().map(|k| g(k) as i64).sum();
        let has_activity = !a.secs.is_empty() || a.first_scan.is_some();
        if has_activity {
            sqlx::query(
                "INSERT INTO daily_summaries
                  (employee_id, work_date, first_scan_at, last_scan_at, first_desk_at, scan_to_desk_sec,
                   focused_sec, present_sec, phone_sec, looking_away_sec, away_sec, unknown_sec, meeting_sec,
                   phone_count, away_count, meeting_count, break_sec, longest_sitting_sec, wellness_alerts, updated_at)
                 VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,UTC_TIMESTAMP())
                 ON DUPLICATE KEY UPDATE
                   first_scan_at=VALUES(first_scan_at), last_scan_at=VALUES(last_scan_at),
                   first_desk_at=VALUES(first_desk_at), scan_to_desk_sec=VALUES(scan_to_desk_sec),
                   focused_sec=VALUES(focused_sec), present_sec=VALUES(present_sec),
                   phone_sec=VALUES(phone_sec), looking_away_sec=VALUES(looking_away_sec),
                   away_sec=VALUES(away_sec), unknown_sec=VALUES(unknown_sec), meeting_sec=VALUES(meeting_sec),
                   phone_count=VALUES(phone_count), away_count=VALUES(away_count), meeting_count=VALUES(meeting_count),
                   break_sec=VALUES(break_sec), longest_sitting_sec=VALUES(longest_sitting_sec),
                   wellness_alerts=VALUES(wellness_alerts), updated_at=UTC_TIMESTAMP()",
            )
            .bind(emp)
            .bind(date)
            .bind(anchor)
            .bind(a.check_out.or(a.last_scan))
            .bind(first_desk)
            .bind(scan_to_desk)
            .bind(g("focused"))
            .bind(g("present"))
            .bind(g("phone"))
            .bind(g("looking_away"))
            .bind(g("away"))
            .bind(g("unknown"))
            .bind(g("meeting"))
            .bind(a.phone_count)
            .bind(a.away_count)
            .bind(a.meeting_count)
            .bind(a.break_sec as i32)
            .bind(a.longest as i32)
            .bind(a.alerts as i32)
            .execute(&state.db)
            .await?;
        }

        // ---- work session: schedule vs. reality
        let sched = cal.day(emp, date);
        let mine: Vec<_> = requests.iter().filter(|r| r.0 == emp).collect();
        let covers = |kind: &str, t: Option<DateTime<Utc>>| {
            t.is_some_and(|t| mine.iter().any(|r| r.1 == kind && r.2 <= t && r.3 >= t))
        };
        let leave_start = covers("leave", sched.start) || covers("offsite", sched.start);
        let leave_end = covers("leave", sched.end) || covers("offsite", sched.end);
        let full_leave = covers("leave", sched.start) && covers("leave", sched.end);
        let full_offsite = covers("offsite", sched.start) && covers("offsite", sched.end);
        let approved_ot: f64 = mine.iter().filter(|r| r.1 == "overtime").map(|r| r.4.unwrap_or(0.0) * 60.0).sum();

        let check_in = a.check_in;
        // one scan only: still at work (today) or forgot to scan out (past days)
        let check_out = a.check_out.filter(|o| Some(*o) != check_in);
        let present = check_in.is_some() || desk_sec > 0;
        let status = if present {
            "present"
        } else if full_leave {
            "leave"
        } else if full_offsite {
            "offsite"
        } else if sched.day_type != "workday" {
            sched.day_type
        } else if sched.start.is_some_and(|s| now < s + Duration::hours(2)) {
            "not_yet"
        } else {
            "absent"
        };
        let mut late_min = 0;
        let mut early_min = 0;
        let mut overtime_min = 0;
        let mut worked_sec = 0;
        if let Some(ci) = check_in {
            let co = check_out.unwrap_or(if date == local_today(off) { now.min(end) } else { ci });
            let brk = match (sched.break_start, sched.break_end) {
                (Some(bs), Some(be)) => overlap(ci, co, bs, be),
                _ => 0,
            };
            worked_sec = ((co - ci).num_seconds() - brk).max(0);
            if sched.day_type == "workday" {
                if let Some(ss) = sched.start {
                    if !leave_start {
                        late_min = ((ci - (ss + Duration::minutes(sched.grace_min))).num_minutes()).max(0);
                    }
                }
                if let (Some(se), Some(co)) = (sched.end, check_out) {
                    if !leave_end && now > se {
                        early_min = (se - co).num_minutes().max(0);
                    }
                    overtime_min = (co - se).num_minutes().max(0);
                }
            } else {
                overtime_min = worked_sec / 60; // all work on a holiday / day off is overtime
            }
        }
        sqlx::query(
            "INSERT INTO work_sessions
              (employee_id, work_date, day_type, status, shift_id, scheduled_start, scheduled_end, office_entry_at,
               check_in, check_out, first_desk_at, worked_sec, desk_sec, break_sec, late_min, early_leave_min,
               overtime_min, approved_overtime_min, updated_at)
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,UTC_TIMESTAMP())
             ON DUPLICATE KEY UPDATE day_type=VALUES(day_type), status=VALUES(status), shift_id=VALUES(shift_id),
               scheduled_start=VALUES(scheduled_start), scheduled_end=VALUES(scheduled_end),
               office_entry_at=VALUES(office_entry_at), check_in=VALUES(check_in), check_out=VALUES(check_out),
               first_desk_at=VALUES(first_desk_at), worked_sec=VALUES(worked_sec), desk_sec=VALUES(desk_sec),
               break_sec=VALUES(break_sec), late_min=VALUES(late_min), early_leave_min=VALUES(early_leave_min),
               overtime_min=VALUES(overtime_min), approved_overtime_min=VALUES(approved_overtime_min),
               updated_at=UTC_TIMESTAMP()",
        )
        .bind(emp)
        .bind(date)
        .bind(sched.day_type)
        .bind(status)
        .bind(sched.shift_id)
        .bind(sched.start)
        .bind(sched.end)
        .bind(a.office_entry)
        .bind(check_in)
        .bind(check_out)
        .bind(first_desk)
        .bind(worked_sec as i32)
        .bind(desk_sec as i32)
        .bind(a.break_sec as i32)
        .bind(late_min as i32)
        .bind(early_min as i32)
        .bind(overtime_min as i32)
        .bind(approved_ot.round() as i32)
        .execute(&state.db)
        .await?;
    }
    Ok(n)
}

/// Monthly partitions of activity_events (after scripts/partition_activity.sql was run):
/// keeps `AHEAD` future months available and drops months that are completely older
/// than the retention cutoff. Does nothing when the table is not partitioned.
async fn maintain_partitions(state: &AppState, cutoff: DateTime<Utc>) -> anyhow::Result<()> {
    const AHEAD: u32 = 3;
    let parts: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT PARTITION_NAME, PARTITION_DESCRIPTION FROM information_schema.PARTITIONS
          WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'activity_events' AND PARTITION_NAME IS NOT NULL
          ORDER BY PARTITION_ORDINAL_POSITION",
    )
    .fetch_all(&state.db)
    .await?;
    if parts.is_empty() {
        return Ok(());
    }
    let bound = |d: &Option<String>| {
        d.as_deref().and_then(|v| NaiveDate::parse_from_str(v.trim_matches('\'').get(..10).unwrap_or(""), "%Y-%m-%d").ok())
    };
    let monthly: Vec<(String, NaiveDate)> = parts
        .iter()
        .filter(|(n, _)| n.len() == 7 && n.starts_with('p') && n[1..].chars().all(|c| c.is_ascii_digit()))
        .filter_map(|(n, d)| bound(d).map(|b| (n.clone(), b)))
        .collect();
    let has_pmax = parts.iter().any(|(n, _)| n == "pmax");
    // add the coming months by splitting the catch-all partition (empty, so this is instant)
    let first_of = |d: NaiveDate| d.with_day(1).unwrap();
    let today = Utc::now().date_naive();
    let want = first_of(today).checked_add_months(chrono::Months::new(AHEAD + 1)).unwrap();
    let mut last = monthly.last().map(|(_, b)| *b).unwrap_or_else(|| first_of(today));
    if has_pmax {
        let mut new_parts = Vec::new();
        while last < want {
            let next = last.checked_add_months(chrono::Months::new(1)).unwrap();
            new_parts.push(format!("PARTITION p{} VALUES LESS THAN ('{}')", last.format("%Y%m"), next.format("%Y-%m-%d")));
            last = next;
        }
        if !new_parts.is_empty() {
            let sql = format!(
                "ALTER TABLE activity_events REORGANIZE PARTITION pmax INTO ({}, PARTITION pmax VALUES LESS THAN (MAXVALUE))",
                new_parts.join(", ")
            );
            sqlx::query(&sql).execute(&state.db).await?;
            tracing::info!("activity_events: added {} monthly partition(s)", new_parts.len());
        }
    }
    // whole months older than the retention period: drop instead of deleting row by row
    let old: Vec<&String> = monthly.iter().filter(|(_, b)| *b <= cutoff.date_naive()).map(|(n, _)| n).collect();
    // always keep at least one range partition
    if !old.is_empty() && old.len() < monthly.len() {
        let names = old.iter().map(|n| n.as_str()).collect::<Vec<_>>().join(", ");
        sqlx::query(&format!("ALTER TABLE activity_events DROP PARTITION {names}")).execute(&state.db).await?;
        tracing::info!("activity_events: dropped partition(s) {names} (older than retention)");
    }
    Ok(())
}

async fn retention_cleanup(state: &AppState) -> anyhow::Result<()> {
    let days = setting_f64(&state.db, "retention_days", 180.0).await.max(7.0) as i64;
    let cutoff = Utc::now() - Duration::days(days);
    if let Err(e) = maintain_partitions(state, cutoff).await {
        tracing::warn!("partition maintenance failed: {e:#}");
    }
    let ev = sqlx::query("DELETE FROM activity_events WHERE started_at < ? LIMIT 50000")
        .bind(cutoff.naive_utc())
        .execute(&state.db)
        .await?
        .rows_affected();
    let nt = sqlx::query("DELETE FROM notifications WHERE created_at < ? LIMIT 50000")
        .bind(cutoff)
        .execute(&state.db)
        .await?
        .rows_affected();
    // corrections whose event is gone (needed once the table is partitioned: no foreign key)
    sqlx::query(
        "DELETE c FROM data_corrections c LEFT JOIN activity_events e ON e.id = c.activity_event_id WHERE e.id IS NULL",
    )
    .execute(&state.db)
    .await?;
    if ev + nt > 0 {
        tracing::info!("retention: removed {ev} activity events and {nt} notifications older than {days} days");
    }
    Ok(())
}

pub async fn run(state: AppState) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
    let mut n: u64 = 0;
    loop {
        tick.tick().await;
        let off = state.cfg.tz_offset_min;
        let today = local_today(off);
        let mut dates: Vec<NaiveDate> = state.dirty_dates.lock().await.drain().collect();
        dates.push(today);
        // finish yesterday properly during the first hour after midnight
        if to_local(Utc::now(), off).hour() == 0 {
            dates.push(today - Duration::days(1));
        }
        dates.sort();
        dates.dedup();
        for d in dates {
            if let Err(e) = recompute_day(&state, d).await {
                tracing::warn!("summary for {d} failed: {e:#}");
                state.mark_dirty([d]).await;
            }
        }
        if n % 5 == 0 {
            crate::routes::cameras::sync_all_streams(&state).await;
            crate::routes::vms::cleanup_playback(&state).await;
        }
        // AI workers send a heartbeat every few seconds; silence means the camera is down
        let _ = sqlx::query(
            "UPDATE cameras SET status = 'offline' WHERE status = 'online' AND last_seen_at < UTC_TIMESTAMP() - INTERVAL 90 SECOND",
        )
        .execute(&state.db)
        .await;
        state.live.write().await.retain(|_, d| (Utc::now() - d.updated_at).num_seconds() < 120);
        state.camera_health.write().await.retain(|_, h| {
            h["updated_at"]
                .as_str()
                .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
                .map(|t| (Utc::now() - t.with_timezone(&Utc)).num_seconds() < 120)
                .unwrap_or(false)
        });
        if n % 60 == 0 {
            if let Err(e) = retention_cleanup(&state).await {
                tracing::warn!("retention cleanup failed: {e:#}");
            }
        }
        n += 1;
    }
}
