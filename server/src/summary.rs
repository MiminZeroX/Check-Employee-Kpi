//! Background jobs: daily summary aggregation and data-retention cleanup.

use std::collections::HashMap;

use chrono::{DateTime, Duration, NaiveDate, Timelike, Utc};

use crate::{
    state::AppState,
    util::{day_range, is_at_desk, local_today, setting_f64, to_local},
};

#[derive(Default, Debug)]
struct Acc {
    secs: HashMap<String, i64>,
    phone_count: i32,
    away_count: i32,
    desk_starts: Vec<DateTime<Utc>>,
    run_start: Option<DateTime<Utc>>,
    run_end: Option<DateTime<Utc>>,
    longest: i64,
    /// lengths (seconds) of every continuous sitting run
    runs: Vec<i64>,
    first_scan: Option<DateTime<Utc>>,
    last_scan: Option<DateTime<Utc>>,
    alerts: i64,
}

/// A break shorter than this does not end a "continuous sitting" run.
const SITTING_GAP_SEC: i64 = 120;

pub async fn recompute_day(state: &AppState, date: NaiveDate) -> anyhow::Result<usize> {
    let off = state.cfg.tz_offset_min;
    let (start, end) = day_range(date, off);

    let events: Vec<(i64, String, DateTime<Utc>, DateTime<Utc>)> = sqlx::query_as(
        "SELECT employee_id, COALESCE(corrected_state, state), started_at, ended_at
         FROM activity_events
         WHERE employee_id IS NOT NULL AND review_status <> 'rejected'
           AND started_at < ? AND ended_at > ?
         ORDER BY employee_id, started_at",
    )
    .bind(end)
    .bind(start)
    .fetch_all(&state.db)
    .await?;

    let scans: Vec<(i64, DateTime<Utc>, DateTime<Utc>)> = sqlx::query_as(
        "SELECT employee_id, MIN(scan_time), MAX(scan_time) FROM attendance_logs
         WHERE employee_id IS NOT NULL AND scan_time >= ? AND scan_time < ?
         GROUP BY employee_id",
    )
    .bind(start)
    .bind(end)
    .fetch_all(&state.db)
    .await?;

    let alerts: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT employee_id, COUNT(*) FROM notifications
         WHERE kind = 'wellness_sitting' AND employee_id IS NOT NULL
           AND created_at >= ? AND created_at < ?
         GROUP BY employee_id",
    )
    .bind(start)
    .bind(end)
    .fetch_all(&state.db)
    .await?;

    let mut acc: HashMap<i64, Acc> = HashMap::new();
    for (emp, first, last) in scans {
        let a = acc.entry(emp).or_default();
        a.first_scan = Some(first);
        a.last_scan = Some(last);
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
        let a = acc.entry(emp).or_default();
        *a.secs.entry(st.clone()).or_default() += dur;
        match st.as_str() {
            "phone" => a.phone_count += 1,
            "away" => a.away_count += 1,
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
    let n = acc.len();
    for (emp, mut a) in acc {
        if let (Some(rs), Some(re)) = (a.run_start, a.run_end) {
            a.runs.push((re - rs).num_seconds());
        }
        // a wellness alert is due for every sitting run longer than the threshold;
        // take the larger of that and the alerts actually pushed live
        let derived = a.runs.iter().filter(|r| **r >= alert_sec).count() as i64;
        a.alerts = a.alerts.max(derived);
        let first_desk = match a.first_scan {
            Some(fs) => a.desk_starts.iter().copied().filter(|d| *d >= fs).min(),
            None => a.desk_starts.iter().copied().min(),
        };
        let scan_to_desk = match (a.first_scan, first_desk) {
            (Some(fs), Some(fd)) => Some((fd - fs).num_seconds() as i32),
            _ => None,
        };
        let g = |k: &str| *a.secs.get(k).unwrap_or(&0) as i32;
        sqlx::query(
            "INSERT INTO daily_summaries
              (employee_id, work_date, first_scan_at, last_scan_at, first_desk_at, scan_to_desk_sec,
               focused_sec, present_sec, phone_sec, looking_away_sec, away_sec, unknown_sec,
               phone_count, away_count, longest_sitting_sec, wellness_alerts, updated_at)
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,UTC_TIMESTAMP())
             ON DUPLICATE KEY UPDATE
               first_scan_at=VALUES(first_scan_at), last_scan_at=VALUES(last_scan_at),
               first_desk_at=VALUES(first_desk_at), scan_to_desk_sec=VALUES(scan_to_desk_sec),
               focused_sec=VALUES(focused_sec), present_sec=VALUES(present_sec),
               phone_sec=VALUES(phone_sec), looking_away_sec=VALUES(looking_away_sec),
               away_sec=VALUES(away_sec), unknown_sec=VALUES(unknown_sec),
               phone_count=VALUES(phone_count), away_count=VALUES(away_count),
               longest_sitting_sec=VALUES(longest_sitting_sec),
               wellness_alerts=VALUES(wellness_alerts), updated_at=UTC_TIMESTAMP()",
        )
        .bind(emp)
        .bind(date)
        .bind(a.first_scan)
        .bind(a.last_scan)
        .bind(first_desk)
        .bind(scan_to_desk)
        .bind(g("focused"))
        .bind(g("present"))
        .bind(g("phone"))
        .bind(g("looking_away"))
        .bind(g("away"))
        .bind(g("unknown"))
        .bind(a.phone_count)
        .bind(a.away_count)
        .bind(a.longest as i32)
        .bind(a.alerts as i32)
        .execute(&state.db)
        .await?;
    }
    Ok(n)
}

async fn retention_cleanup(state: &AppState) -> anyhow::Result<()> {
    let days = setting_f64(&state.db, "retention_days", 180.0).await.max(7.0) as i64;
    let cutoff = Utc::now() - Duration::days(days);
    let ev = sqlx::query("DELETE FROM activity_events WHERE started_at < ? LIMIT 50000")
        .bind(cutoff)
        .execute(&state.db)
        .await?
        .rows_affected();
    let nt = sqlx::query("DELETE FROM notifications WHERE created_at < ? LIMIT 50000")
        .bind(cutoff)
        .execute(&state.db)
        .await?
        .rows_affected();
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
        }
        // AI workers send a heartbeat every few seconds; silence means the camera is down
        let _ = sqlx::query(
            "UPDATE cameras SET status = 'offline' WHERE status = 'online' AND last_seen_at < UTC_TIMESTAMP() - INTERVAL 90 SECOND",
        )
        .execute(&state.db)
        .await;
        state.live.write().await.retain(|_, d| (Utc::now() - d.updated_at).num_seconds() < 120);
        if n % 60 == 0 {
            if let Err(e) = retention_cleanup(&state).await {
                tracing::warn!("retention cleanup failed: {e:#}");
            }
        }
        n += 1;
    }
}
