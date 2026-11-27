use std::env;

#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub db_max_connections: u32,
    pub bind: String,
    pub jwt_secret: String,
    pub jwt_ttl_hours: i64,
    pub admin_username: String,
    pub admin_password: String,
    /// Offset of the office timezone from UTC, in minutes (Asia/Bangkok = 420).
    pub tz_offset_min: i32,
    pub web_dir: String,
    /// go2rtc API reachable from this server, e.g. http://go2rtc:1984 (optional).
    pub go2rtc_api: Option<String>,
    /// RTSP base the AI worker should read from, e.g. rtsp://go2rtc:8554 (optional).
    pub go2rtc_rtsp: Option<String>,
}

fn var(name: &str, default: &str) -> String {
    env::var(name).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| default.to_string())
}

fn opt(name: &str) -> Option<String> {
    env::var(name).ok().filter(|v| !v.is_empty())
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let jwt_secret = var("JWT_SECRET", "");
        if jwt_secret.len() < 32 {
            anyhow::bail!("JWT_SECRET must be set and at least 32 characters long");
        }
        if jwt_secret.to_ascii_lowercase().contains("change-me") {
            anyhow::bail!("JWT_SECRET still has the example value; generate one with `openssl rand -hex 32`");
        }
        let admin_password = var("ADMIN_PASSWORD", "");
        Ok(Self {
            database_url: var("DATABASE_URL", "mysql://smartoffice:smartoffice@127.0.0.1:3306/smartoffice"),
            db_max_connections: var("DB_MAX_CONNECTIONS", "5").parse()?,
            bind: var("BIND_ADDR", "0.0.0.0:8080"),
            jwt_secret,
            jwt_ttl_hours: var("JWT_TTL_HOURS", "12").parse()?,
            admin_username: var("ADMIN_USERNAME", "admin"),
            admin_password,
            tz_offset_min: var("TZ_OFFSET_MINUTES", "420").parse()?,
            web_dir: var("WEB_DIR", "../web"),
            go2rtc_api: opt("GO2RTC_API"),
            go2rtc_rtsp: opt("GO2RTC_RTSP"),
        })
    }
}
