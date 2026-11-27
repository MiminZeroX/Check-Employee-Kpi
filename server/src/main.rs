mod auth;
mod config;
mod error;
mod routes;
mod state;
mod summary;
mod util;

use std::{net::SocketAddr, path::PathBuf, time::Duration};

use sqlx::mysql::MySqlPoolOptions;
use tower_http::{
    compression::CompressionLayer,
    limit::RequestBodyLimitLayer,
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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
    routes::cameras::sync_all_streams(&state).await;

    tokio::spawn(summary::run(state.clone()));

    let web = PathBuf::from(&cfg.web_dir);
    let static_files = ServeDir::new(&web).fallback(ServeFile::new(web.join("index.html")));

    let app = routes::router(state)
        .fallback_service(static_files)
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
