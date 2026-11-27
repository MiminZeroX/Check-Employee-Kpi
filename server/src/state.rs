use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Instant,
};

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use sqlx::MySqlPool;
use tokio::sync::{broadcast, Mutex, RwLock};

use crate::config::Config;

/// Current, in-memory state of one workstation as last reported by an AI worker.
#[derive(Clone, Debug, Serialize)]
pub struct LiveDesk {
    pub workstation_id: i64,
    pub camera_id: i64,
    pub employee_id: Option<i64>,
    pub state: String,
    pub confidence: f64,
    pub since: DateTime<Utc>,
    pub sitting_since: Option<DateTime<Utc>>,
    /// Last measured head yaw, used to calibrate the "looking at screen" direction.
    pub yaw: Option<f64>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip)]
    pub sitting_alerted: bool,
}

#[derive(Clone)]
pub struct AppState(pub Arc<Inner>);

pub struct Inner {
    pub db: MySqlPool,
    pub cfg: Config,
    pub crypto: crate::crypto::Crypto,
    pub events: broadcast::Sender<String>,
    pub live: RwLock<HashMap<i64, LiveDesk>>,
    /// Latest stream health per camera from the AI worker (fps, latency, reconnects, error).
    pub camera_health: RwLock<HashMap<i64, serde_json::Value>>,
    /// Local dates whose daily summaries must be recomputed.
    pub dirty_dates: Mutex<HashSet<NaiveDate>>,
    pub http: reqwest::Client,
    pub started: Instant,
    pub sys: Mutex<sysinfo::System>,
    /// role -> permissions (cache of role_permissions)
    pub role_perms: RwLock<HashMap<String, Arc<HashSet<String>>>>,
    /// Failed login attempts per "u:<username>" / "ip:<address>" key: (count, first failure).
    pub login_failures: Mutex<HashMap<String, (u32, Instant)>>,
}

impl std::ops::Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl AppState {
    pub fn new(db: MySqlPool, cfg: Config, crypto: crate::crypto::Crypto) -> Self {
        let (events, _) = broadcast::channel(512);
        Self(Arc::new(Inner {
            db,
            cfg,
            crypto,
            events,
            live: RwLock::new(HashMap::new()),
            camera_health: RwLock::new(HashMap::new()),
            dirty_dates: Mutex::new(HashSet::new()),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .expect("http client"),
            started: Instant::now(),
            sys: Mutex::new(sysinfo::System::new()),
            login_failures: Mutex::new(HashMap::new()),
            role_perms: RwLock::new(HashMap::new()),
        }))
    }

    /// Push a JSON message to every connected dashboard.
    pub fn broadcast(&self, msg: serde_json::Value) {
        let _ = self.events.send(msg.to_string());
    }

    pub async fn mark_dirty(&self, dates: impl IntoIterator<Item = NaiveDate>) {
        let mut d = self.dirty_dates.lock().await;
        d.extend(dates);
    }
}
