//! Login, sessions and request guards.
//!
//! Every request passes [`guard`], which:
//! 1. rejects unknown `Host` headers (defeats DNS rebinding: a hostile page can't read the API
//!    through a domain that resolves to this machine),
//! 2. rejects cross-site writes (`Origin` / `Sec-Fetch-Site` must match),
//! 3. requires a session for the API once an admin password is set. Viewer sessions are
//!    read-only.
//!
//! Sessions are random 256-bit tokens in an `HttpOnly; SameSite=Strict` cookie; only their
//! SHA-256 is stored (in `data_dir/sessions.json`).

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use axum::Json;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use photolake_core::Config;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::state::Shared;

pub const COOKIE: &str = "photolake_session";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Viewer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Session {
    role: Role,
    expires: DateTime<Utc>,
}

/// Session store plus login throttling.
pub struct Auth {
    path: PathBuf,
    sessions: Mutex<HashMap<String, Session>>,
    failures: Mutex<HashMap<IpAddr, (u32, Instant)>>,
}

/// Hash a password for `[server.auth]`.
pub fn hash_password(password: &str) -> Result<String> {
    if password.chars().count() < 8 {
        bail!("use at least 8 characters");
    }
    let salt = SaltString::generate(&mut argon2::password_hash::rand_core::OsRng);
    Ok(argon2::Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("hashing password: {e}"))?
        .to_string())
}

fn verify(password: &str, hash: Option<&str>) -> bool {
    let Some(parsed) = hash.and_then(|h| PasswordHash::new(h).ok()) else {
        return false;
    };
    argon2::Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

fn digest(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

impl Auth {
    pub fn new(cfg: &Config) -> Self {
        let path = cfg.data_dir().join("sessions.json");
        let mut sessions: HashMap<String, Session> = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        sessions.retain(|_, s| s.expires > Utc::now());
        Self {
            path,
            sessions: Mutex::new(sessions),
            failures: Mutex::new(HashMap::new()),
        }
    }

    fn persist(&self, sessions: &HashMap<String, Session>) {
        let write = || -> Result<()> {
            let tmp = self.path.with_extension("json.tmp");
            std::fs::write(&tmp, serde_json::to_vec(sessions)?)?;
            photolake_core::config::restrict(&tmp);
            std::fs::rename(&tmp, &self.path)?;
            Ok(())
        };
        if let Err(e) = write() {
            tracing::warn!(error = %e, "saving sessions");
        }
    }

    fn create(&self, role: Role, days: u32) -> String {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).expect("OS random number generator");
        let token = hex::encode(bytes);
        let mut s = self.sessions.lock().unwrap();
        s.retain(|_, v| v.expires > Utc::now());
        s.insert(
            digest(&token),
            Session {
                role,
                expires: Utc::now() + chrono::Duration::days(days.max(1) as i64),
            },
        );
        self.persist(&s);
        token
    }

    fn lookup(&self, token: &str) -> Option<Role> {
        let s = self.sessions.lock().unwrap();
        s.get(&digest(token))
            .filter(|v| v.expires > Utc::now())
            .map(|v| v.role)
    }

    fn remove(&self, token: &str) {
        let mut s = self.sessions.lock().unwrap();
        if s.remove(&digest(token)).is_some() {
            self.persist(&s);
        }
    }

    /// Sign every session out (after a password change).
    pub fn clear(&self) {
        let mut s = self.sessions.lock().unwrap();
        s.clear();
        self.persist(&s);
    }

    /// Seconds this client must wait before trying again, if throttled.
    fn throttled(&self, ip: IpAddr) -> Option<u64> {
        let f = self.failures.lock().unwrap();
        let (n, last) = f.get(&ip)?;
        let wait = Duration::from_secs(2u64.saturating_pow(n.saturating_sub(2)).min(60));
        let elapsed = last.elapsed();
        (*n >= 3 && elapsed < wait).then(|| (wait - elapsed).as_secs().max(1))
    }

    fn record_failure(&self, ip: IpAddr) {
        let mut f = self.failures.lock().unwrap();
        let e = f.entry(ip).or_insert((0, Instant::now()));
        e.0 += 1;
        e.1 = Instant::now();
    }

    fn record_success(&self, ip: IpAddr) {
        self.failures.lock().unwrap().remove(&ip);
    }
}

/// Whether the server may run without a password: only when nothing but this machine can
/// reach it.
pub fn open_access_allowed(cfg: &Config) -> bool {
    cfg.server
        .bind
        .parse::<SocketAddr>()
        .map(|a| a.ip().is_loopback())
        .unwrap_or_else(|_| cfg.server.bind.starts_with("localhost:"))
}

pub fn password_required(cfg: &Config) -> bool {
    cfg.server
        .auth
        .admin_password_hash
        .as_deref()
        .is_some_and(|h| !h.is_empty())
}

fn cookie_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == COOKIE)
        .map(|(_, v)| v.to_string())
}

/// Host name without port (IPv6 brackets removed).
fn host_name(host: &str) -> &str {
    if let Some(rest) = host.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(rest);
    }
    host.rsplit_once(':')
        .filter(|(_, p)| p.chars().all(|c| c.is_ascii_digit()))
        .map_or(host, |(h, _)| h)
}

/// DNS rebinding needs a domain the attacker controls, so IP literals are safe; names must be
/// known.
pub fn host_allowed(host: &str, cfg: &Config) -> bool {
    let name = host_name(host).trim_end_matches('.').to_ascii_lowercase();
    if name.is_empty() {
        return false;
    }
    if name.parse::<IpAddr>().is_ok()
        || name == "localhost"
        || name.ends_with(".localhost")
        || name.ends_with(".ts.net")
    {
        return true;
    }
    if let Some(me) = hostname()
        && (name == me || name == format!("{me}.local") || name == format!("{me}.lan"))
    {
        return true;
    }
    cfg.server.allowed_hosts.iter().any(|h| {
        let h = h.trim().trim_end_matches('.').to_ascii_lowercase();
        h == name
            || h.strip_prefix("*.")
                .is_some_and(|d| name.ends_with(&format!(".{d}")))
    })
}

fn hostname() -> Option<String> {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
}

/// A browser-initiated write must come from the page this server served.
fn same_origin(headers: &HeaderMap) -> bool {
    if headers
        .get("sec-fetch-site")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == "cross-site")
    {
        return false;
    }
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        // Non-browser clients (curl, scripts) send no Origin.
        return true;
    };
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    origin
        .split_once("://")
        .is_some_and(|(_, rest)| rest.eq_ignore_ascii_case(host))
}

fn deny(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

/// Routes reachable without a session.
fn public(path: &str) -> bool {
    !path.starts_with("/api/") || matches!(path, "/api/auth/login" | "/api/auth/me")
}

/// Admin-only even for reads (configuration, drives, raw SQL).
fn admin_only(path: &str) -> bool {
    path.starts_with("/api/settings")
        || path.starts_with("/api/auth/password")
        || path.starts_with("/api/drives")
        || path.starts_with("/api/sql")
}

/// Role for this request (None = not signed in).
fn role_for(app: &Shared, headers: &HeaderMap) -> Option<Role> {
    if !password_required(&app.config()) {
        return Some(Role::Admin);
    }
    cookie_token(headers).and_then(|t| app.auth.lookup(&t))
}

pub async fn guard(State(app): State<Shared>, req: Request, next: Next) -> Response {
    let headers = req.headers();
    let cfg = app.config();
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !host_allowed(host, &cfg) {
        tracing::warn!(host, "rejected request with unknown Host header");
        return deny(
            StatusCode::FORBIDDEN,
            "unknown host name; add it to server.allowed_hosts",
        );
    }
    let method = req.method().clone();
    let safe = matches!(method, Method::GET | Method::HEAD | Method::OPTIONS);
    if !safe && !same_origin(headers) {
        return deny(StatusCode::FORBIDDEN, "cross-site request blocked");
    }
    let path = req.uri().path().to_string();
    if public(&path) {
        return next.run(req).await;
    }
    match role_for(&app, headers) {
        None => deny(StatusCode::UNAUTHORIZED, "sign in required"),
        Some(Role::Viewer) if !safe || admin_only(&path) => {
            deny(StatusCode::FORBIDDEN, "this login is read-only")
        }
        Some(_) => next.run(req).await,
    }
}

/// Security headers for every response.
pub async fn headers(req: Request, next: Next) -> Response {
    let mut resp = next.run(req).await;
    let h = resp.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; \
             img-src 'self' data: blob: https://tiles.openfreemap.org; \
             connect-src 'self' https://tiles.openfreemap.org https://nominatim.openstreetmap.org; \
             worker-src 'self' blob:; child-src 'self' blob:; \
             style-src 'self' 'unsafe-inline'; font-src 'self' data: https://tiles.openfreemap.org; \
             object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'none'",
        ),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("same-origin"),
    );
    resp
}

#[derive(Deserialize)]
pub struct LoginBody {
    pub password: String,
}

fn is_https(headers: &HeaderMap) -> bool {
    let fwd = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("https"));
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("https://"));
    fwd || origin
}

fn session_cookie(value: &str, max_age: i64, secure: bool) -> HeaderValue {
    let secure = if secure { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{COOKIE}={value}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}{secure}"
    ))
    .expect("valid cookie")
}

pub async fn login(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<LoginBody>,
) -> Response {
    let cfg = app.config();
    if !password_required(&cfg) {
        return Json(json!({ "role": Role::Admin })).into_response();
    }
    let ip = peer.ip();
    if let Some(wait) = app.auth.throttled(ip) {
        return deny(
            StatusCode::TOO_MANY_REQUESTS,
            &format!("too many attempts; try again in {wait} s"),
        );
    }
    let a = &cfg.server.auth;
    // Argon2 is deliberately slow; keep it off the async workers.
    let pw = body.password.clone();
    let (admin_hash, viewer_hash) = (
        a.admin_password_hash.clone(),
        a.viewer_password_hash.clone(),
    );
    let role = tokio::task::spawn_blocking(move || {
        if verify(&pw, admin_hash.as_deref()) {
            Some(Role::Admin)
        } else if verify(&pw, viewer_hash.as_deref()) {
            Some(Role::Viewer)
        } else {
            None
        }
    })
    .await
    .ok()
    .flatten();
    let Some(role) = role else {
        app.auth.record_failure(ip);
        tracing::warn!(%ip, "failed sign-in");
        return deny(StatusCode::UNAUTHORIZED, "wrong password");
    };
    app.auth.record_success(ip);
    let token = app.auth.create(role, a.session_days);
    let mut resp = Json(json!({ "role": role })).into_response();
    resp.headers_mut().insert(
        header::SET_COOKIE,
        session_cookie(&token, a.session_days as i64 * 86_400, is_https(&headers)),
    );
    resp
}

pub async fn logout(State(app): State<Shared>, headers: HeaderMap) -> Response {
    if let Some(t) = cookie_token(&headers) {
        app.auth.remove(&t);
    }
    let mut resp = StatusCode::NO_CONTENT.into_response();
    resp.headers_mut().insert(
        header::SET_COOKIE,
        session_cookie("", 0, is_https(&headers)),
    );
    resp
}

#[derive(Deserialize)]
pub struct PasswordBody {
    /// Empty with `clear` removes the password.
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub viewer: bool,
    #[serde(default)]
    pub clear: bool,
}

/// Set or remove the admin/viewer password from the UI. Signs every other session out and
/// keeps the caller signed in as admin.
pub async fn set_password(
    State(app): State<Shared>,
    headers: HeaderMap,
    Json(b): Json<PasswordBody>,
) -> Response {
    let Some(path) = app.cfg_path.clone() else {
        return deny(StatusCode::BAD_REQUEST, "no config file location");
    };
    let hash = if b.clear {
        None
    } else {
        let pw = b.password.clone();
        match tokio::task::spawn_blocking(move || hash_password(&pw)).await {
            Ok(Ok(h)) => Some(h),
            Ok(Err(e)) => return deny(StatusCode::BAD_REQUEST, &e.to_string()),
            Err(_) => return deny(StatusCode::INTERNAL_SERVER_ERROR, "hashing failed"),
        }
    };
    let days = {
        let mut c = app.cfg.write().unwrap();
        if b.viewer {
            if hash.is_some() && !password_required(&c) {
                return deny(StatusCode::BAD_REQUEST, "set an admin password first");
            }
            c.server.auth.viewer_password_hash = hash;
        } else {
            if hash.is_none() {
                if !open_access_allowed(&c) {
                    return deny(
                        StatusCode::BAD_REQUEST,
                        "other devices can reach this server, so it must keep a password",
                    );
                }
                c.server.auth.viewer_password_hash = None;
            }
            c.server.auth.admin_password_hash = hash;
        }
        if let Err(e) = c.save(&path) {
            return deny(StatusCode::INTERNAL_SERVER_ERROR, &format!("{e:#}"));
        }
        c.server.auth.session_days
    };
    app.auth.clear();
    let mut resp = Json(json!({ "saved": true })).into_response();
    if password_required(&app.config()) {
        let token = app.auth.create(Role::Admin, days);
        resp.headers_mut().insert(
            header::SET_COOKIE,
            session_cookie(&token, days as i64 * 86_400, is_https(&headers)),
        );
    }
    resp
}

/// Who am I? `{ auth_required, role }` (role is null when signed out).
pub async fn me(State(app): State<Shared>, headers: HeaderMap) -> Json<serde_json::Value> {
    let cfg = app.config();
    Json(json!({
        "auth_required": password_required(&cfg),
        "has_viewer": cfg.server.auth.viewer_password_hash.is_some(),
        "role": role_for(&app, &headers),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_names() {
        let mut cfg = Config::default();
        cfg.server.allowed_hosts = vec!["photos.example.com".into(), "*.home.arpa".into()];
        for ok in [
            "127.0.0.1:7878",
            "localhost:7878",
            "[::1]:7878",
            "192.168.1.20:7878",
            "photolake.tail1234.ts.net",
            "photos.example.com",
            "nas.home.arpa:7878",
        ] {
            assert!(host_allowed(ok, &cfg), "{ok}");
        }
        for bad in [
            "evil.example:7878",
            "",
            "localhost.evil.com",
            "home.arpa.evil.com",
        ] {
            assert!(!host_allowed(bad, &cfg), "{bad}");
        }
    }

    #[test]
    fn origin_checks() {
        let mut h = HeaderMap::new();
        h.insert(header::HOST, HeaderValue::from_static("127.0.0.1:7878"));
        assert!(same_origin(&h));
        h.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://127.0.0.1:7878"),
        );
        assert!(same_origin(&h));
        h.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://evil.example"),
        );
        assert!(!same_origin(&h));
        let mut h2 = HeaderMap::new();
        h2.insert("sec-fetch-site", HeaderValue::from_static("cross-site"));
        assert!(!same_origin(&h2));
    }

    #[test]
    fn passwords_verify() {
        let h = hash_password("correct horse").unwrap();
        assert!(verify("correct horse", Some(&h)));
        assert!(!verify("wrong", Some(&h)));
        assert!(!verify("x", None));
        assert!(hash_password("short").is_err());
    }

    #[test]
    fn cookie_parsing() {
        let mut h = HeaderMap::new();
        h.insert(
            header::COOKIE,
            HeaderValue::from_static("a=1; photolake_session=abc; b=2"),
        );
        assert_eq!(cookie_token(&h).as_deref(), Some("abc"));
    }
}
