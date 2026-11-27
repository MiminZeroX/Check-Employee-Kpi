mod auth;
mod config;
mod error;
mod routes;
mod state;
mod summary;
mod util;

use std::{net::SocketAddr, path::PathBuf, time::Duration};

use sqlx::mysql::MySqlPoolOptions;
use axum::http::{header, HeaderName, HeaderValue};
use tower_http::{
    compression::CompressionLayer,
    limit::RequestBodyLimitLayer,
    set_header::SetResponseHeaderLayer,
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use tracing_subscriber::EnvFilter;

use crate::{config::Config, state::AppState};

async fn ensure_admin(state: &AppState) -> anyhow::Result<()> {
    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users").fetch_one(&state.db).await?;
    if count > 0 {
        return Ok(());
    }
    let mut password = state.cfg.admin_password.clone();
    if password.is_empty() {
        password = util::random_token(8);
        tracing::warn!(
            "no ADMIN_PASSWORD set - created user '{}' with generated password: {password}  (change it after login)",
            state.cfg.admin_username
        );
    }
    let hash = auth::hash_password(&password).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    sqlx::query("INSERT INTO users (username, display_name, password_hash, role) VALUES (?, 'Administrator', ?, 'admin')")
        .bind(&state.cfg.admin_username)
        .bind(hash)
        .execute(&state.db)
        .await?;
    tracing::info!("created initial admin user '{}'", state.cfg.admin_username);
    Ok(())
}

fn main() -> anyhow::Result<()> {
    // a small office needs very few threads; fewer threads = less memory (default 2, WORKER_THREADS to change)
    let threads = std::env::var("WORKER_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(2);
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(threads)
        .max_blocking_threads(4)
        .thread_stack_size(512 * 1024)
        .enable_all()
        .build()?
        .block_on(run())
}

async fn run() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn,tower_http=warn")),
        )
        .init();

    let cfg = Config::from_env()?;

    // the database may still be starting when running under docker compose
    let mut attempt = 0;
    let pool = loop {
        match MySqlPoolOptions::new()
            .max_connections(cfg.db_max_connections)
            .acquire_timeout(Duration::from_secs(10))
            .connect(&cfg.database_url)
            .await
        {
            Ok(p) => break p,
            Err(e) if attempt < 30 => {
                attempt += 1;
                tracing::warn!("database not ready ({e}); retrying in 2s");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(e) => return Err(e.into()),
        }
    };
    sqlx::migrate!("./migrations").run(&pool).await?;

    let state = AppState::new(pool, cfg.clone());
    ensure_admin(&state).await?;
    auth::load_permissions(&state).await?;
    routes::cameras::sync_all_streams(&state).await;

    tokio::spawn(summary::run(state.clone()));

    let web = PathBuf::from(&cfg.web_dir);
    let static_files = ServeDir::new(&web).fallback(ServeFile::new(web.join("index.html")));

    // Content-Security-Policy: everything from this server only (live video is proxied under /video),
    // plus the one inline print button of the A4 report, allowed by hash. The report is opened as a
    // blob and inherits this policy.
    let csp = "default-src 'self'; script-src 'self' 'unsafe-hashes' 'sha256-MguIPR6qNR8D3B+eAlK+bIRTZe8t3wkOY4B/56Me9FU='; \
               style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; font-src 'self' https://fonts.gstatic.com; \
               img-src 'self' data: blob:; frame-src 'self'; connect-src 'self' ws: wss:; \
               object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'";
    let app = routes::router(state)
        .fallback_service(static_files)
        .layer(SetResponseHeaderLayer::if_not_present(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(csp)))
        .layer(SetResponseHeaderLayer::overriding(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")))
        .layer(SetResponseHeaderLayer::if_not_present(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY")))
        .layer(SetResponseHeaderLayer::overriding(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer")))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("permissions-policy"),
            HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
        .layer(RequestBodyLimitLayer::new(8 * 1024 * 1024))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&cfg.bind).await?;
    tracing::info!("listening on http://{}", cfg.bind);
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
