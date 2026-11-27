pub mod activity;
pub mod attendance;
pub mod cameras;
pub mod crm;
pub mod devices;
pub mod hr;
pub mod ingest;
pub mod kpi;
pub mod org;
pub mod reports;
pub mod roles;
pub mod system;
pub mod users;
pub mod video;

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
        .merge(hr::routes());

    Router::new()
        .nest("/api", api)
        // ZKTeco ADMS / "push SDK" protocol must live at the root path
        .merge(ingest::iclock_routes())
        // camera viewing goes through the authenticated proxy, never directly to go2rtc
        .merge(video::proxy_routes())
        .with_state(state)
}
