//! Work calendar: which shift (if any) an employee works on a given day.
//!
//! Resolution order: per-day assignment (shift or day off) -> employee's shift ->
//! company default shift -> the employee's own work_start/work_end on Mon-Fri.
//! Public holidays (company-wide or for the employee's branch) turn a workday into a holiday.

use std::collections::HashMap;

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, Utc};

use crate::util::from_local;

#[derive(Clone, Debug)]
pub struct Shift {
    pub id: i64,
    pub name: String,
    pub start: NaiveTime,
    pub end: NaiveTime,
    pub break_start: Option<NaiveTime>,
    pub break_end: Option<NaiveTime>,
    pub work_days: Vec<u32>,
    pub grace_min: i64,
}

#[derive(Clone, Debug, Default)]
pub struct DaySchedule {
    /// workday | holiday | dayoff
    pub day_type: &'static str,
    pub shift_id: Option<i64>,
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
    pub break_start: Option<DateTime<Utc>>,
    pub break_end: Option<DateTime<Utc>>,
    pub grace_min: i64,
    pub holiday: Option<String>,
}

pub struct Calendar {
    off: i32,
    shifts: HashMap<i64, Shift>,
    default_shift: Option<i64>,
    /// employee -> (shift, branch, work_start, work_end)
    emps: HashMap<i64, (Option<i64>, Option<i64>, NaiveTime, NaiveTime)>,
    overrides: HashMap<(i64, NaiveDate), Option<i64>>,
    holidays: Vec<(NaiveDate, Option<i64>, String)>,
}

fn parse_days(s: &str) -> Vec<u32> {
    s.split(',').filter_map(|d| d.trim().parse().ok()).filter(|d| (1..=7).contains(d)).collect()
}

impl Calendar {
    pub async fn load(db: &sqlx::MySqlPool, off: i32, from: NaiveDate, to: NaiveDate) -> anyhow::Result<Self> {
        let rows: Vec<(i64, String, NaiveTime, NaiveTime, Option<NaiveTime>, Option<NaiveTime>, String, i32, bool)> = sqlx::query_as(
            "SELECT id, name, start_time, end_time, break_start, break_end, work_days, grace_min, is_default FROM shifts",
        )
        .fetch_all(db)
        .await?;
        let mut shifts = HashMap::new();
        let mut default_shift = None;
        for (id, name, start, end, bs, be, days, grace, is_default) in rows {
            if is_default {
                default_shift = Some(id);
            }
            shifts.insert(
                id,
                Shift { id, name, start, end, break_start: bs, break_end: be, work_days: parse_days(&days), grace_min: grace as i64 },
            );
        }
        let emps: Vec<(i64, Option<i64>, Option<i64>, NaiveTime, NaiveTime)> = sqlx::query_as(
            "SELECT e.id, e.shift_id, d.branch_id, e.work_start, e.work_end FROM employees e LEFT JOIN departments d ON d.id = e.department_id",
        )
        .fetch_all(db)
        .await?;
        let overrides: Vec<(i64, NaiveDate, Option<i64>)> =
            sqlx::query_as("SELECT employee_id, work_date, shift_id FROM shift_assignments WHERE work_date BETWEEN ? AND ?")
                .bind(from)
                .bind(to)
                .fetch_all(db)
                .await?;
        let holidays: Vec<(NaiveDate, Option<i64>, String)> =
            sqlx::query_as("SELECT holiday_date, branch_id, name FROM holidays WHERE holiday_date BETWEEN ? AND ?")
                .bind(from)
                .bind(to)
                .fetch_all(db)
                .await?;
        Ok(Self {
            off,
            shifts,
            default_shift,
            emps: emps.into_iter().map(|(id, s, b, ws, we)| (id, (s, b, ws, we))).collect(),
            overrides: overrides.into_iter().map(|(e, d, s)| ((e, d), s)).collect(),
            holidays,
        })
    }

    pub fn shift(&self, id: i64) -> Option<&Shift> {
        self.shifts.get(&id)
    }

    pub fn day(&self, emp: i64, date: NaiveDate) -> DaySchedule {
        let (emp_shift, branch, ws, we) = self.emps.get(&emp).cloned().unwrap_or((None, None, NaiveTime::MIN, NaiveTime::MIN));
        let weekday = date.weekday().number_from_monday();
        // which shift applies, and does it work today?
        let shift: Option<Shift> = match self.overrides.get(&(emp, date)) {
            Some(None) => return DaySchedule { day_type: "dayoff", ..Default::default() },
            Some(Some(id)) => self.shifts.get(id).cloned().map(|mut s| {
                s.work_days = (1..=7).collect(); // an explicit assignment works that day
                s
            }),
            None => emp_shift.or(self.default_shift).and_then(|id| self.shifts.get(&id).cloned()),
        };
        let shift = shift.unwrap_or_else(|| Shift {
            id: 0,
            name: "ตามเวลาพนักงาน".into(),
            start: ws,
            end: we,
            break_start: None,
            break_end: None,
            work_days: vec![1, 2, 3, 4, 5],
            grace_min: 0,
        });
        if !shift.work_days.contains(&weekday) {
            return DaySchedule { day_type: "dayoff", ..Default::default() };
        }
        let at = |t: NaiveTime| from_local(date.and_time(t), self.off);
        let start = at(shift.start);
        let mut end = at(shift.end);
        if end <= start {
            end += Duration::days(1); // overnight shift
        }
        let holiday = self
            .holidays
            .iter()
            .find(|(d, b, _)| *d == date && (b.is_none() || *b == branch))
            .map(|(_, _, n)| n.clone());
        DaySchedule {
            day_type: if holiday.is_some() { "holiday" } else { "workday" },
            shift_id: (shift.id > 0).then_some(shift.id),
            start: Some(start),
            end: Some(end),
            break_start: shift.break_start.map(at),
            break_end: shift.break_end.map(at),
            grace_min: shift.grace_min,
            holiday,
        }
    }

    /// Working days of `emp` in [from, to] (used to count leave days).
    pub fn working_days(&self, emp: i64, from: NaiveDate, to: NaiveDate) -> f64 {
        let mut d = from;
        let mut n = 0.0;
        while d <= to {
            if self.day(emp, d).day_type == "workday" {
                n += 1.0;
            }
            d += Duration::days(1);
        }
        n
    }
}

/// Seconds of overlap between [a1, a2) and [b1, b2).
pub fn overlap(a1: DateTime<Utc>, a2: DateTime<Utc>, b1: DateTime<Utc>, b2: DateTime<Utc>) -> i64 {
    let s = a1.max(b1);
    let e = a2.min(b2);
    if e > s {
        (e - s).num_seconds()
    } else {
        0
    }
}
