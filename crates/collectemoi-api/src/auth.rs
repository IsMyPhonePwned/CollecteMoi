//! Admin console authentication: Argon2id password, TOTP MFA, WebAuthn/YubiKey.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::{
    Json,
    extract::{ConnectInfo, Path, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::Engine;
use collectemoi_db::{AdminAccountRow, AdminSessionRow, AdminWebauthnRow};
use qrcode::QrCode;
use qrcode::render::svg;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use totp_rs::{Algorithm, Secret, TOTP};
use url::Url;
use uuid::Uuid;
use webauthn_rs::prelude::*;

use crate::AppState;

pub const COOKIE_NAME: &str = "cm_admin_session";
const STAGE_MFA: &str = "mfa";
const STAGE_FULL: &str = "full";
const MFA_TTL_SECS: u64 = 10 * 60;
const FULL_TTL_SECS: u64 = 12 * 60 * 60;
const STATE_TTL_SECS: u64 = 5 * 60;
/// No minimum policy length — admin chooses any non-empty password (confirmed in the UI).
const MAX_PASSWORD_CHARS: usize = 128;

#[derive(Clone)]
pub struct AuthState {
    pub webauthn: Arc<Webauthn>,
    pub fail_guard: Arc<std::sync::Mutex<FailGuard>>,
}

#[derive(Default)]
struct FailBucket {
    fails: u32,
    locked_until: Option<Instant>,
    last_seen: Option<Instant>,
}

/// Per-IP auth failure lockout (global lockout would let one attacker block everyone).
#[derive(Default)]
pub struct FailGuard {
    by_ip: std::collections::HashMap<String, FailBucket>,
}

impl FailGuard {
    fn bucket(&mut self, ip: &str) -> &mut FailBucket {
        let now = Instant::now();
        // Drop idle entries occasionally.
        if self.by_ip.len() > 2_000 {
            self.by_ip.retain(|_, b| {
                b.last_seen
                    .map(|t| now.duration_since(t) < Duration::from_secs(600))
                    .unwrap_or(false)
            });
        }
        let b = self.by_ip.entry(ip.to_string()).or_default();
        b.last_seen = Some(now);
        b
    }

    fn check(&mut self, ip: &str) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
        let b = self.bucket(ip);
        if let Some(until) = b.locked_until {
            if Instant::now() < until {
                return Err(err(
                    StatusCode::TOO_MANY_REQUESTS,
                    "too many failed attempts — try again shortly",
                ));
            }
            b.locked_until = None;
            b.fails = 0;
        }
        Ok(())
    }

    fn record_fail(&mut self, ip: &str) {
        let b = self.bucket(ip);
        b.fails = b.fails.saturating_add(1);
        if b.fails >= 8 {
            b.locked_until = Some(Instant::now() + Duration::from_secs(60));
        }
    }

    fn clear(&mut self, ip: &str) {
        if let Some(b) = self.by_ip.get_mut(ip) {
            b.fails = 0;
            b.locked_until = None;
        }
    }
}

fn client_ip_from_headers(headers: &HeaderMap, peer: Option<std::net::SocketAddr>) -> String {
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        let first = xff.split(',').next().unwrap_or(xff).trim();
        if !first.is_empty() {
            return first.to_string();
        }
    }
    if let Some(real) = headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return real.to_string();
    }
    peer.map(|a| a.ip().to_string())
        .unwrap_or_else(|| "unknown".into())
}

pub fn build_webauthn(public_url: &str, http_bind: &str) -> anyhow::Result<Webauthn> {
    let mut origin = Url::parse(public_url.trim().trim_end_matches('/'))
        .map_err(|e| anyhow::anyhow!("invalid public_url for WebAuthn: {e}"))?;
    let host = origin
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("public_url has no host"))?
        .to_string();
    // webauthn-rs requires Url::domain() which is None for bare IPs. Loopback
    // is remapped to `localhost` so YubiKey/WebAuthn works; open admin via
    // https://localhost:<port>/admin (not 127.0.0.1) when registering keys.
    let (rp_id, primary) = if host == "127.0.0.1" || host == "::1" {
        tracing::warn!(
            "WebAuthn RP ID remapped {host} → localhost — use https://localhost for YubiKey registration"
        );
        origin
            .set_host(Some("localhost"))
            .map_err(|e| anyhow::anyhow!("rewrite origin host: {e}"))?;
        ("localhost".to_string(), origin)
    } else {
        (host, origin)
    };
    let mut builder = WebauthnBuilder::new(&rp_id, &primary)?
        .rp_name("CollecteMoi Lab")
        .allow_any_port(true);
    for extra in http_admin_origins(http_bind) {
        builder = builder.append_allowed_origin(&extra);
    }
    // Also accept the literal loopback HTTPS origin if we remapped to localhost.
    if rp_id == "localhost" {
        if let Ok(u) = Url::parse("https://127.0.0.1:8080") {
            builder = builder.append_allowed_origin(&u);
        }
        if let Ok(u) = Url::parse("https://localhost:8080") {
            builder = builder.append_allowed_origin(&u);
        }
    }
    Ok(builder.build()?)
}

fn http_admin_origins(http_bind: &str) -> Vec<Url> {
    let bind = http_bind.trim();
    if bind.is_empty() || bind.eq_ignore_ascii_case("off") {
        return Vec::new();
    }
    let (host, port) = if let Some((h, p)) = bind.rsplit_once(':') {
        (h, p)
    } else {
        return Vec::new();
    };
    let mut hosts = vec![];
    if host == "0.0.0.0" || host == "127.0.0.1" || host == "::" || host == "::1" {
        hosts.push("localhost");
        hosts.push("127.0.0.1");
    } else {
        hosts.push(host);
    }
    hosts
        .into_iter()
        .filter_map(|h| Url::parse(&format!("http://{h}:{port}")).ok())
        .collect()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn now_secs_str() -> String {
    now_secs().to_string()
}

fn err(status: StatusCode, msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    (status, Json(json!({ "error": msg })))
}

fn hash_token(token: &str) -> String {
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    hex::encode(h.finalize())
}

fn new_token() -> String {
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    hex::encode(buf)
}

fn hash_password(password: &str) -> Result<String, (StatusCode, Json<serde_json::Value>)> {
    use argon2::{
        Argon2,
        password_hash::{PasswordHasher, SaltString, rand_core::OsRng},
    };
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "password hash failed"))
}

fn verify_password(password: &str, hash: &str) -> bool {
    use argon2::{
        Argon2,
        password_hash::{PasswordHash, PasswordVerifier},
    };
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

fn make_totp(
    secret_b32: &str,
    account: &str,
) -> Result<TOTP, (StatusCode, Json<serde_json::Value>)> {
    let secret = Secret::Encoded(secret_b32.to_string())
        .to_bytes()
        .map_err(|_| err(StatusCode::BAD_REQUEST, "invalid totp secret"))?;
    TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        secret,
        Some("CollecteMoi Lab".into()),
        account.to_string(),
    )
    .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "totp init failed"))
}

fn verify_totp(
    secret_b32: &str,
    account: &str,
    code: &str,
) -> Result<bool, (StatusCode, Json<serde_json::Value>)> {
    let totp = make_totp(secret_b32, account)?;
    Ok(totp.check_current(code.trim()).unwrap_or(false))
}

fn cookie_header(token: &str, max_age: u64, secure: bool) -> HeaderValue {
    let mut v = format!("{COOKIE_NAME}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}");
    if secure {
        v.push_str("; Secure");
    }
    HeaderValue::from_str(&v).unwrap_or_else(|_| HeaderValue::from_static(COOKIE_NAME))
}

fn clear_cookie_header(secure: bool) -> HeaderValue {
    let mut v = format!("{COOKIE_NAME}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0");
    if secure {
        v.push_str("; Secure");
    }
    HeaderValue::from_str(&v).unwrap_or_else(|_| HeaderValue::from_static(COOKIE_NAME))
}

fn request_is_https(headers: &HeaderMap, uri_scheme_hint: bool) -> bool {
    if uri_scheme_hint {
        return true;
    }
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.eq_ignore_ascii_case("https"))
        .unwrap_or(false)
}

/// Whether Set-Cookie should include `Secure`.
///
/// Lab deployments often enable TLS *and* `--http-bind` for local admin. A
/// `Secure` cookie set while the operator is on `http://127.0.0.1:…` is dropped
/// by the browser — login returns 200 then every follow-up call is 401. Prefer
/// the actual request scheme; when a plain HTTP bind is configured, never force
/// Secure from the TLS flag alone.
fn cookie_should_be_secure(state: &AppState, headers: &HeaderMap) -> bool {
    if request_is_https(headers, false) {
        return true;
    }
    let Ok(s) = state.settings.lock() else {
        return true;
    };
    if !s.http_bind.is_empty() {
        return false;
    }
    s.tls
}

fn extract_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in raw.split(';') {
        let part = part.trim();
        if let Some((k, v)) = part.split_once('=') {
            if k.trim() == name {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

fn client_meta(headers: &HeaderMap) -> (String, String) {
    let ua = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(180)
        .collect();
    let remote = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').next().unwrap_or(s).trim().to_string())
        .unwrap_or_default();
    (remote, ua)
}

fn validate_password(password: &str) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    if password.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "password is required"));
    }
    if password.chars().count() > MAX_PASSWORD_CHARS {
        return Err(err(StatusCode::BAD_REQUEST, "password too long"));
    }
    Ok(())
}

fn validate_username(username: &str) -> Result<String, (StatusCode, Json<serde_json::Value>)> {
    let u = username.trim();
    if u.len() < 3 || u.len() > 64 {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "username must be 3–64 characters",
        ));
    }
    if !u
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "username may only contain letters, digits, . _ -",
        ));
    }
    Ok(u.to_string())
}

fn create_session(
    state: &AppState,
    stage: &str,
    ttl: u64,
    headers: &HeaderMap,
) -> Result<String, (StatusCode, Json<serde_json::Value>)> {
    let token = new_token();
    let now = now_secs();
    let (remote, ua) = client_meta(headers);
    let row = AdminSessionRow {
        token_hash: hash_token(&token),
        stage: stage.to_string(),
        created_at: now.to_string(),
        expires_at: (now + ttl).to_string(),
        last_seen_at: now.to_string(),
        remote,
        user_agent: ua,
    };
    state
        .db
        .admin_session_insert(&row)
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "session create failed"))?;
    Ok(token)
}

fn load_session(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Option<(String, AdminSessionRow)>, (StatusCode, Json<serde_json::Value>)> {
    let Some(token) = extract_cookie(headers, COOKIE_NAME) else {
        return Ok(None);
    };
    let hash = hash_token(&token);
    let now = now_secs_str();
    let _ = state.db.admin_sessions_purge_expired(&now);
    let Some(sess) = state
        .db
        .admin_session_get(&hash)
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "session lookup failed"))?
    else {
        return Ok(None);
    };
    if sess.expires_at < now {
        let _ = state.db.admin_session_delete(&hash);
        return Ok(None);
    }
    let _ = state.db.admin_session_touch(&hash, &now);
    Ok(Some((token, sess)))
}

fn require_stage(
    state: &AppState,
    headers: &HeaderMap,
    stage: &str,
) -> Result<(String, AdminSessionRow), (StatusCode, Json<serde_json::Value>)> {
    let Some((token, sess)) = load_session(state, headers)? else {
        return Err(err(StatusCode::UNAUTHORIZED, "authentication required"));
    };
    if sess.stage != stage {
        return Err(err(StatusCode::UNAUTHORIZED, "authentication required"));
    }
    Ok((token, sess))
}

fn require_full(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(String, AdminSessionRow), (StatusCode, Json<serde_json::Value>)> {
    require_stage(state, headers, STAGE_FULL)
}

fn path_is_public_auth(path: &str) -> bool {
    matches!(
        path,
        "/v1/admin/auth/status"
            | "/v1/admin/auth/setup/start"
            | "/v1/admin/auth/setup/finish"
            | "/v1/admin/auth/login"
            | "/v1/admin/auth/mfa/totp"
            | "/v1/admin/auth/mfa/webauthn/start"
            | "/v1/admin/auth/mfa/webauthn/finish"
            | "/v1/admin/auth/logout"
    )
}

fn path_needs_auth(path: &str) -> bool {
    if path == "/admin" || path == "/admin/" || path.starts_with("/admin/") {
        // HTML shell is public; JS gate uses auth status.
        return false;
    }
    if path.starts_with("/v1/admin/auth/") {
        return !path_is_public_auth(path);
    }
    path.starts_with("/v1/admin/")
}

pub async fn require_admin(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    if !path_needs_auth(&path) {
        return next.run(req).await;
    }
    match require_full(&state, req.headers()) {
        Ok(_) => next.run(req).await,
        Err((status, body)) => (status, body).into_response(),
    }
}

/* ── handlers ─────────────────────────────────────────────────── */

#[derive(Serialize)]
pub struct AuthStatus {
    pub setup_required: bool,
    pub authenticated: bool,
    pub stage: Option<String>,
    pub username: Option<String>,
    pub totp_enabled: bool,
    pub webauthn_count: usize,
    pub methods: Vec<&'static str>,
}

pub async fn auth_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AuthStatus>, (StatusCode, Json<serde_json::Value>)> {
    let account = state
        .db
        .admin_account_get()
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "db error"))?;
    let webauthn_count = state.db.admin_webauthn_list().map(|v| v.len()).unwrap_or(0);
    let sess = load_session(&state, &headers)?;
    let authenticated = sess
        .as_ref()
        .map(|(_, s)| s.stage == STAGE_FULL)
        .unwrap_or(false);
    let stage = sess.as_ref().map(|(_, s)| s.stage.clone());
    let totp_enabled = account.as_ref().map(|a| a.totp_enabled).unwrap_or(false);
    let methods = mfa_methods(totp_enabled, webauthn_count);
    Ok(Json(AuthStatus {
        setup_required: account.is_none(),
        authenticated,
        stage,
        username: account.as_ref().map(|a| a.username.clone()),
        totp_enabled,
        webauthn_count,
        methods,
    }))
}

fn mfa_methods(totp_enabled: bool, webauthn_count: usize) -> Vec<&'static str> {
    let mut methods = Vec::new();
    if totp_enabled {
        methods.push("totp");
    }
    if webauthn_count > 0 {
        methods.push("webauthn");
    }
    methods
}

#[derive(Serialize)]
pub struct SetupStartResponse {
    pub setup_id: String,
    /// Password-only bootstrap — TOTP / YubiKey are optional after first login.
    pub password_required: bool,
    pub totp_required: bool,
}

pub async fn setup_start(
    State(state): State<AppState>,
) -> Result<Json<SetupStartResponse>, (StatusCode, Json<serde_json::Value>)> {
    if state
        .db
        .admin_account_get()
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "db error"))?
        .is_some()
    {
        return Err(err(StatusCode::CONFLICT, "admin already configured"));
    }

    let setup_id = Uuid::new_v4().simple().to_string();
    let expires = (now_secs() + STATE_TTL_SECS * 6).to_string();
    state
        .db
        .admin_state_put(
            &setup_id,
            "setup_password",
            &json!({ "v": 1 }).to_string(),
            &expires,
        )
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "setup store failed"))?;

    Ok(Json(SetupStartResponse {
        setup_id,
        password_required: true,
        totp_required: false,
    }))
}

#[derive(Deserialize)]
pub struct SetupFinishRequest {
    pub setup_id: String,
    /// Ignored if present — first account is always `admin`.
    #[serde(default)]
    pub username: Option<String>,
    pub password: String,
}

pub async fn setup_finish(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<SetupFinishRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    if state
        .db
        .admin_account_get()
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "db error"))?
        .is_some()
    {
        return Err(err(StatusCode::CONFLICT, "admin already configured"));
    }
    let _ = req.username; // clients may still send it; ignored
    let username = validate_username("admin")?;
    validate_password(&req.password)?;
    let _ = state
        .db
        .admin_state_take(&req.setup_id)
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "setup lookup failed"))?
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "setup expired — start again"))?;
    let password_hash = hash_password(&req.password)?;
    let now = now_secs_str();
    state
        .db
        .admin_account_upsert(&AdminAccountRow {
            username: username.clone(),
            password_hash,
            totp_secret: String::new(),
            totp_enabled: false,
            created_at: now.clone(),
            updated_at: now,
        })
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "account create failed"))?;

    let token = create_session(&state, STAGE_FULL, FULL_TTL_SECS, &headers)?;
    let secure = cookie_should_be_secure(&state, &headers);
    let mut res = Json(json!({
        "ok": true,
        "authenticated": true,
        "username": username,
        "totp_enabled": false,
    }))
    .into_response();
    res.headers_mut().insert(
        header::SET_COOKIE,
        cookie_header(&token, FULL_TTL_SECS, secure),
    );
    Ok(res)
}

#[derive(Deserialize)]
pub struct LoginRequest {
    /// Optional — defaults to `admin` (only account).
    #[serde(default)]
    pub username: Option<String>,
    pub password: String,
}

pub async fn login(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let ip = client_ip_from_headers(&headers, Some(addr));
    {
        let mut g = state.auth.fail_guard.lock().unwrap();
        g.check(&ip)?;
    }
    let account = state
        .db
        .admin_account_get()
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "db error"))?
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "setup required"))?;

    let username = req
        .username
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("admin");
    // Strip BOM / NUL only — do not trim spaces (passwords may contain them).
    let password = req
        .password
        .trim_matches(|c: char| c == '\0' || c == '\u{feff}');
    let user_ok = username == account.username;
    let pass_ok = verify_password(password, &account.password_hash);
    if !(user_ok && pass_ok) {
        tracing::warn!(
            remote = %ip,
            user_ok,
            pass_len = password.chars().count(),
            "admin login failed"
        );
        state.auth.fail_guard.lock().unwrap().record_fail(&ip);
        // Constant-ish delay
        tokio::time::sleep(Duration::from_millis(250)).await;
        return Err(err(
            StatusCode::UNAUTHORIZED,
            "invalid username or password",
        ));
    }
    state.auth.fail_guard.lock().unwrap().clear(&ip);
    state.rate_limiter.clear_ip(&ip);
    tracing::info!(remote = %ip, "admin login ok");

    let webauthn_count = state.db.admin_webauthn_list().map(|v| v.len()).unwrap_or(0);
    let methods = mfa_methods(account.totp_enabled, webauthn_count);
    let secure = cookie_should_be_secure(&state, &headers);

    // Password is always required. MFA only when TOTP and/or a security key is enabled.
    if methods.is_empty() {
        let token = create_session(&state, STAGE_FULL, FULL_TTL_SECS, &headers)?;
        let mut res = Json(json!({
            "ok": true,
            "stage": STAGE_FULL,
            "authenticated": true,
            "username": account.username,
            "methods": methods,
            "webauthn_count": webauthn_count,
            "totp_enabled": account.totp_enabled,
        }))
        .into_response();
        res.headers_mut().insert(
            header::SET_COOKIE,
            cookie_header(&token, FULL_TTL_SECS, secure),
        );
        return Ok(res);
    }

    let token = create_session(&state, STAGE_MFA, MFA_TTL_SECS, &headers)?;
    let mut res = Json(json!({
        "ok": true,
        "stage": STAGE_MFA,
        "methods": methods,
        "webauthn_count": webauthn_count,
        "totp_enabled": account.totp_enabled,
    }))
    .into_response();
    res.headers_mut().insert(
        header::SET_COOKIE,
        cookie_header(&token, MFA_TTL_SECS, secure),
    );
    Ok(res)
}

#[derive(Deserialize)]
pub struct TotpMfaRequest {
    pub code: String,
}

pub async fn mfa_totp(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<TotpMfaRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let ip = client_ip_from_headers(&headers, Some(addr));
    {
        let mut g = state.auth.fail_guard.lock().unwrap();
        g.check(&ip)?;
    }
    let (token, sess) = require_stage(&state, &headers, STAGE_MFA)?;
    let account = state
        .db
        .admin_account_get()
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "db error"))?
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "setup required"))?;
    if !account.totp_enabled || account.totp_secret.is_empty() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "TOTP is not enabled — use a security key or enable TOTP in Settings",
        ));
    }
    if !verify_totp(&account.totp_secret, &account.username, &req.code)? {
        state.auth.fail_guard.lock().unwrap().record_fail(&ip);
        return Err(err(StatusCode::UNAUTHORIZED, "invalid authenticator code"));
    }
    state.auth.fail_guard.lock().unwrap().clear(&ip);
    state.rate_limiter.clear_ip(&ip);
    let expires = (now_secs() + FULL_TTL_SECS).to_string();
    state
        .db
        .admin_session_set_stage(&sess.token_hash, STAGE_FULL, &expires)
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "session upgrade failed"))?;
    let secure = cookie_should_be_secure(&state, &headers);
    let mut res = Json(json!({
        "ok": true,
        "authenticated": true,
        "username": account.username,
    }))
    .into_response();
    res.headers_mut().insert(
        header::SET_COOKIE,
        cookie_header(&token, FULL_TTL_SECS, secure),
    );
    Ok(res)
}

pub async fn mfa_webauthn_start(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let (_token, sess) = require_stage(&state, &headers, STAGE_MFA)?;
    let keys = load_passkeys(&state)?;
    if keys.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "no security keys registered"));
    }
    let (rcr, auth_state) = state
        .auth
        .webauthn
        .start_passkey_authentication(&keys)
        .map_err(|e| {
            err(
                StatusCode::BAD_REQUEST,
                &format!("webauthn start failed: {e}"),
            )
        })?;
    let expires = (now_secs() + STATE_TTL_SECS).to_string();
    let state_json = serde_json::to_string(&auth_state)
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "serialize failed"))?;
    state
        .db
        .admin_state_put(
            &format!("authstate:{}", sess.token_hash),
            "webauthn_auth_state",
            &state_json,
            &expires,
        )
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "state store failed"))?;
    Ok(Json(serde_json::to_value(&rcr).unwrap_or(json!({}))))
}

#[derive(Deserialize)]
pub struct WebauthnAuthFinish {
    pub response: serde_json::Value,
}

pub async fn mfa_webauthn_finish(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<WebauthnAuthFinish>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let ip = client_ip_from_headers(&headers, Some(addr));
    {
        let mut g = state.auth.fail_guard.lock().unwrap();
        g.check(&ip)?;
    }
    let (token, sess) = require_stage(&state, &headers, STAGE_MFA)?;
    let (_kind, state_json) = state
        .db
        .admin_state_take(&format!("authstate:{}", sess.token_hash))
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "state lookup failed"))?
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "webauthn challenge expired"))?;
    let auth_state: PasskeyAuthentication = serde_json::from_str(&state_json)
        .map_err(|_| err(StatusCode::BAD_REQUEST, "invalid webauthn state"))?;
    let public_cred: PublicKeyCredential = serde_json::from_value(req.response)
        .map_err(|_| err(StatusCode::BAD_REQUEST, "invalid webauthn response"))?;
    let result = match state
        .auth
        .webauthn
        .finish_passkey_authentication(&public_cred, &auth_state)
    {
        Ok(r) => r,
        Err(_) => {
            state.auth.fail_guard.lock().unwrap().record_fail(&ip);
            return Err(err(
                StatusCode::UNAUTHORIZED,
                "webauthn verification failed",
            ));
        }
    };
    let cred_id = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(result.cred_id());
    let _ = state.db.admin_webauthn_touch(&cred_id, &now_secs_str());
    // Also try hex form if stored differently
    let hex_id = hex::encode(result.cred_id());
    let _ = state.db.admin_webauthn_touch(&hex_id, &now_secs_str());

    state.auth.fail_guard.lock().unwrap().clear(&ip);
    state.rate_limiter.clear_ip(&ip);
    let expires = (now_secs() + FULL_TTL_SECS).to_string();
    state
        .db
        .admin_session_set_stage(&sess.token_hash, STAGE_FULL, &expires)
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "session upgrade failed"))?;
    let account = state.db.admin_account_get().ok().flatten();
    let secure = cookie_should_be_secure(&state, &headers);
    let mut res = Json(json!({
        "ok": true,
        "authenticated": true,
        "username": account.map(|a| a.username),
    }))
    .into_response();
    res.headers_mut().insert(
        header::SET_COOKIE,
        cookie_header(&token, FULL_TTL_SECS, secure),
    );
    Ok(res)
}

pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if let Ok(Some((_, sess))) = load_session(&state, &headers) {
        let _ = state.db.admin_session_delete(&sess.token_hash);
    }
    let secure = cookie_should_be_secure(&state, &headers);
    let mut res = Json(json!({ "ok": true })).into_response();
    res.headers_mut()
        .insert(header::SET_COOKIE, clear_cookie_header(secure));
    res
}

pub async fn me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let _ = require_full(&state, &headers)?;
    let account = state
        .db
        .admin_account_get()
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "db error"))?
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "setup required"))?;
    let keys = state
        .db
        .admin_webauthn_list()
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "db error"))?;
    Ok(Json(json!({
        "username": account.username,
        "totp_enabled": account.totp_enabled,
        "webauthn": keys.iter().map(|k| json!({
            "cred_id": k.cred_id,
            "label": k.label,
            "created_at": k.created_at,
            "last_used_at": k.last_used_at,
        })).collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

pub async fn change_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let _ = require_full(&state, &headers)?;
    validate_password(&req.new_password)?;
    let mut account = state
        .db
        .admin_account_get()
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "db error"))?
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "setup required"))?;
    if !verify_password(&req.current_password, &account.password_hash) {
        return Err(err(StatusCode::UNAUTHORIZED, "current password incorrect"));
    }
    account.password_hash = hash_password(&req.new_password)?;
    account.updated_at = now_secs_str();
    state
        .db
        .admin_account_upsert(&account)
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "update failed"))?;
    Ok(Json(json!({ "ok": true })))
}

/// Offline / host-local recovery: set admin password without knowing the old one.
/// Clears all sessions. Optionally clears TOTP + security keys.
pub fn offline_reset_password(
    db: &collectemoi_db::Db,
    password: &str,
    clear_mfa: bool,
) -> anyhow::Result<OfflineResetReport> {
    if password.is_empty() {
        anyhow::bail!("password is required");
    }
    if password.chars().count() > MAX_PASSWORD_CHARS {
        anyhow::bail!("password too long (max {MAX_PASSWORD_CHARS} characters)");
    }
    let password_hash =
        hash_password(password).map_err(|_| anyhow::anyhow!("password hash failed"))?;
    let now = now_secs_str();
    let existing = db.admin_account_get()?;
    let created = existing.is_none();
    let mut account = existing.unwrap_or(AdminAccountRow {
        username: "admin".into(),
        password_hash: String::new(),
        totp_secret: String::new(),
        totp_enabled: false,
        created_at: now.clone(),
        updated_at: now.clone(),
    });
    account.username = "admin".into();
    account.password_hash = password_hash;
    account.updated_at = now;
    let mut cleared_keys = 0usize;
    if clear_mfa {
        account.totp_secret.clear();
        account.totp_enabled = false;
        cleared_keys = db.admin_webauthn_clear_all()?;
    }
    db.admin_account_upsert(&account)?;
    let cleared_sessions = db.admin_sessions_clear_all()?;
    Ok(OfflineResetReport {
        username: account.username,
        created,
        cleared_sessions,
        cleared_keys,
        mfa_cleared: clear_mfa,
    })
}

/// Wipe admin auth entirely so `/admin` shows first-run setup again.
pub fn offline_force_setup(db: &collectemoi_db::Db) -> anyhow::Result<OfflineResetReport> {
    let had = db.admin_account_delete()?;
    let cleared_sessions = db.admin_sessions_clear_all()?;
    let cleared_keys = db.admin_webauthn_clear_all()?;
    Ok(OfflineResetReport {
        username: "admin".into(),
        created: !had,
        cleared_sessions,
        cleared_keys,
        mfa_cleared: true,
    })
}

#[derive(Debug, Clone)]
pub struct OfflineResetReport {
    pub username: String,
    pub created: bool,
    pub cleared_sessions: usize,
    pub cleared_keys: usize,
    pub mfa_cleared: bool,
}

#[derive(Serialize)]
pub struct TotpEnableStartResponse {
    pub enroll_id: String,
    pub totp_secret: String,
    pub otpauth_url: String,
    pub qr_svg: String,
}

/// Begin optional TOTP enrollment (same pattern as YubiKey registration).
pub async fn totp_enable_start(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<TotpEnableStartResponse>, (StatusCode, Json<serde_json::Value>)> {
    let (_token, sess) = require_full(&state, &headers)?;
    let account = state
        .db
        .admin_account_get()
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "db error"))?
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "setup required"))?;
    if account.totp_enabled {
        return Err(err(StatusCode::CONFLICT, "TOTP already enabled"));
    }
    let secret_b32 = Secret::generate_secret().to_encoded().to_string();
    let totp = make_totp(&secret_b32, &account.username)?;
    let otpauth_url = totp.get_url();
    let qr_svg = QrCode::new(otpauth_url.as_bytes())
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "qr failed"))?
        .render::<svg::Color>()
        .min_dimensions(180, 180)
        .dark_color(svg::Color("#0f1412"))
        .light_color(svg::Color("#e8efeb"))
        .build();
    let enroll_id = Uuid::new_v4().simple().to_string();
    let expires = (now_secs() + STATE_TTL_SECS * 6).to_string();
    state
        .db
        .admin_state_put(
            &format!("totp-enroll:{}:{enroll_id}", sess.token_hash),
            "totp_enroll",
            &json!({ "totp_secret": secret_b32 }).to_string(),
            &expires,
        )
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "enroll store failed"))?;
    Ok(Json(TotpEnableStartResponse {
        enroll_id,
        totp_secret: secret_b32,
        otpauth_url,
        qr_svg,
    }))
}

#[derive(Deserialize)]
pub struct TotpEnableFinishRequest {
    pub enroll_id: String,
    pub code: String,
}

pub async fn totp_enable_finish(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<TotpEnableFinishRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let (_token, sess) = require_full(&state, &headers)?;
    let mut account = state
        .db
        .admin_account_get()
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "db error"))?
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "setup required"))?;
    if account.totp_enabled {
        return Err(err(StatusCode::CONFLICT, "TOTP already enabled"));
    }
    let enroll_id = req.enroll_id.trim();
    if enroll_id.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "enroll_id required"));
    }
    let (_kind, state_json) = state
        .db
        .admin_state_take(&format!("totp-enroll:{}:{enroll_id}", sess.token_hash))
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "enroll lookup failed"))?
        .ok_or_else(|| {
            err(
                StatusCode::BAD_REQUEST,
                "TOTP enrollment expired — start again",
            )
        })?;
    let parsed: serde_json::Value = serde_json::from_str(&state_json)
        .map_err(|_| err(StatusCode::BAD_REQUEST, "invalid enroll state"))?;
    let secret = parsed
        .get("totp_secret")
        .and_then(|v| v.as_str())
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "invalid enroll state"))?;
    if !verify_totp(secret, &account.username, &req.code)? {
        return Err(err(StatusCode::UNAUTHORIZED, "invalid authenticator code"));
    }
    account.totp_secret = secret.to_string();
    account.totp_enabled = true;
    account.updated_at = now_secs_str();
    state
        .db
        .admin_account_upsert(&account)
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "update failed"))?;
    Ok(Json(json!({ "ok": true, "totp_enabled": true })))
}

#[derive(Deserialize)]
pub struct TotpDisableRequest {
    pub password: String,
    #[serde(default)]
    pub code: String,
}

pub async fn totp_disable(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<TotpDisableRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let _ = require_full(&state, &headers)?;
    let mut account = state
        .db
        .admin_account_get()
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "db error"))?
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "setup required"))?;
    if !account.totp_enabled {
        return Ok(Json(json!({ "ok": true, "totp_enabled": false })));
    }
    if !verify_password(&req.password, &account.password_hash) {
        return Err(err(StatusCode::UNAUTHORIZED, "password incorrect"));
    }
    if !verify_totp(&account.totp_secret, &account.username, &req.code)? {
        return Err(err(StatusCode::UNAUTHORIZED, "invalid authenticator code"));
    }
    account.totp_secret.clear();
    account.totp_enabled = false;
    account.updated_at = now_secs_str();
    state
        .db
        .admin_account_upsert(&account)
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "update failed"))?;
    Ok(Json(json!({ "ok": true, "totp_enabled": false })))
}

pub async fn webauthn_register_start(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let (_token, sess) = require_full(&state, &headers)?;
    let account = state
        .db
        .admin_account_get()
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "db error"))?
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "setup required"))?;
    let existing = load_passkeys(&state)?;
    let exclude: Option<Vec<CredentialID>> = if existing.is_empty() {
        None
    } else {
        Some(existing.iter().map(|p| p.cred_id().clone()).collect())
    };
    // Stable per-username UUID (v4 feature only in workspace): hash into UUID bytes.
    let user_unique = {
        let mut h = Sha256::new();
        h.update(b"collectemoi-admin-webauthn:");
        h.update(account.username.as_bytes());
        let dig = h.finalize();
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&dig[..16]);
        // RFC 4122 variant/version bits for a synthetic UUID.
        bytes[6] = (bytes[6] & 0x0f) | 0x50;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        Uuid::from_bytes(bytes)
    };
    let (ccr, reg_state) = state
        .auth
        .webauthn
        .start_passkey_registration(user_unique, &account.username, &account.username, exclude)
        .map_err(|e| {
            err(
                StatusCode::BAD_REQUEST,
                &format!("webauthn start failed: {e}"),
            )
        })?;
    let state_json = serde_json::to_string(&reg_state)
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "serialize failed"))?;
    let expires = (now_secs() + STATE_TTL_SECS).to_string();
    state
        .db
        .admin_state_put(
            &format!("regstate:{}", sess.token_hash),
            "webauthn_reg_state",
            &state_json,
            &expires,
        )
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "state store failed"))?;
    Ok(Json(serde_json::to_value(&ccr).unwrap_or(json!({}))))
}

#[derive(Deserialize)]
pub struct WebauthnRegisterFinish {
    pub response: serde_json::Value,
    #[serde(default)]
    pub label: String,
}

pub async fn webauthn_register_finish(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<WebauthnRegisterFinish>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let (_token, sess) = require_full(&state, &headers)?;
    let (_kind, state_json) = state
        .db
        .admin_state_take(&format!("regstate:{}", sess.token_hash))
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "state lookup failed"))?
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "registration challenge expired"))?;
    let reg_state: PasskeyRegistration = serde_json::from_str(&state_json)
        .map_err(|_| err(StatusCode::BAD_REQUEST, "invalid registration state"))?;
    let reg: RegisterPublicKeyCredential = serde_json::from_value(req.response)
        .map_err(|_| err(StatusCode::BAD_REQUEST, "invalid registration response"))?;
    let passkey = state
        .auth
        .webauthn
        .finish_passkey_registration(&reg, &reg_state)
        .map_err(|e| {
            err(
                StatusCode::BAD_REQUEST,
                &format!("registration failed: {e}"),
            )
        })?;
    let cred_id = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(passkey.cred_id());
    let passkey_json = serde_json::to_string(&passkey)
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "serialize failed"))?;
    let label = {
        let l = req.label.trim();
        if l.is_empty() {
            "YubiKey".to_string()
        } else {
            l.chars().take(64).collect()
        }
    };
    let now = now_secs_str();
    state
        .db
        .admin_webauthn_upsert(&AdminWebauthnRow {
            cred_id: cred_id.clone(),
            passkey_json,
            label: label.clone(),
            created_at: now,
            last_used_at: None,
        })
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "store failed"))?;
    Ok(Json(
        json!({ "ok": true, "cred_id": cred_id, "label": label }),
    ))
}

pub async fn webauthn_list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let _ = require_full(&state, &headers)?;
    let keys = state
        .db
        .admin_webauthn_list()
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "db error"))?;
    Ok(Json(json!({
        "keys": keys.iter().map(|k| json!({
            "cred_id": k.cred_id,
            "label": k.label,
            "created_at": k.created_at,
            "last_used_at": k.last_used_at,
        })).collect::<Vec<_>>(),
    })))
}

pub async fn webauthn_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(cred_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let _ = require_full(&state, &headers)?;
    let deleted = state
        .db
        .admin_webauthn_delete(&cred_id)
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "db error"))?;
    if !deleted {
        return Err(err(StatusCode::NOT_FOUND, "security key not found"));
    }
    Ok(Json(json!({ "ok": true })))
}

fn load_passkeys(state: &AppState) -> Result<Vec<Passkey>, (StatusCode, Json<serde_json::Value>)> {
    let rows = state
        .db
        .admin_webauthn_list()
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "db error"))?;
    let mut out = Vec::new();
    for row in rows {
        match serde_json::from_str::<Passkey>(&row.passkey_json) {
            Ok(pk) => out.push(pk),
            Err(_) => tracing::warn!(cred_id = %row.cred_id, "skip unreadable webauthn credential"),
        }
    }
    Ok(out)
}
