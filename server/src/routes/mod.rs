pub mod accounting;
pub mod activity;
pub mod business;
pub mod documents;
pub mod expenses;
pub mod integrations;
pub mod attendance;
pub mod cameras;
pub mod crm;
pub mod devices;
pub mod hr;
pub mod inventory;
pub mod ingest;
pub mod kpi;
pub mod org;
pub mod payroll;
pub mod reports;
pub mod roles;
pub mod sales;
pub mod security;
pub mod system;
pub mod users;
pub mod video;
pub mod vms;

use axum::Router;
use chrono::NaiveDate;
use serde::Deserialize;

use crate::state::AppState;

/// Common `?from=YYYY-MM-DD&to=YYYY-MM-DD` filter.
#[derive(Debug, Deserialize, Default)]
pub struct Range {
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    pub employee_id: Option<i64>,
    pub department_id: Option<i64>,
    pub branch_id: Option<i64>,
}

/// Employees whose department belongs to `branch` (None = no branch filter).
pub async fn branch_employees(st: &AppState, branch: Option<i64>) -> crate::error::AppResult<Option<std::collections::HashSet<i64>>> {
    let Some(b) = branch else { return Ok(None) };
    let ids: Vec<(i64,)> = sqlx::query_as(
        "SELECT e.id FROM employees e JOIN departments d ON d.id = e.department_id WHERE d.branch_id = ?",
    )
    .bind(b)
    .fetch_all(&st.db)
    .await?;
    Ok(Some(ids.into_iter().map(|(i,)| i).collect()))
}

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .merge(users::routes())
        .merge(org::routes())
        .merge(cameras::routes())
        .merge(devices::routes())
        .merge(ingest::routes())
        .merge(activity::routes())
        .merge(attendance::routes())
        .merge(kpi::routes())
        .merge(crm::routes())
        .merge(reports::routes())
        .merge(system::routes())
        .merge(video::api_routes())
        .merge(roles::routes())
        .merge(hr::routes())
        .merge(business::routes())
        .merge(documents::routes())
        .merge(integrations::routes())
        .merge(vms::routes())
        .merge(security::routes())
        .merge(accounting::routes())
        .merge(payroll::routes())
        .merge(expenses::routes())
        .merge(inventory::routes())
        .merge(sales::routes())
        .layer(axum::middleware::from_fn_with_state(state.clone(), security::rate_limit));

    Router::new()
        .nest("/api", api)
        // ZKTeco ADMS / "push SDK" protocol must live at the root path
        .merge(ingest::iclock_routes())
        // camera viewing goes through the authenticated proxy, never directly to go2rtc
        .merge(video::proxy_routes())
        .with_state(state)
}
