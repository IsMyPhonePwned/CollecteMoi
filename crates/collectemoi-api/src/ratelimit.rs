//! Basic in-process DDoS / abuse controls: per-IP rate limits, class budgets,
//! short automatic IP cool-downs after sustained 429 pressure, and per-device
//! upload (report) quotas over a rolling hour.
//!
//! This is application-layer protection for a lab API. It does not replace
//! network-edge DDoS mitigation (CDN, reverse proxy, firewalls).

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::{
    Json,
    extract::{ConnectInfo, Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde_json::json;

use crate::AppState;

/// Tunables for per-IP sliding windows (requests allowed per window).
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    pub enabled: bool,
    /// General API traffic.
    pub general_per_min: u32,
    /// Admin login / setup / MFA.
    pub auth_per_min: u32,
    /// Enrollment start/confirm / legacy enroll.
    pub enroll_per_min: u32,
    /// CPU-heavy lab seal.
    pub seal_per_min: u32,
    /// Chunk uploads (many small requests).
    pub upload_per_min: u32,
    /// New upload sessions (`POST /v1/uploads`) allowed per device id per window.
    pub device_uploads_per_hour: u32,
    /// Rolling window length for the per-device upload quota (seconds).
    pub device_upload_window_secs: u64,
    /// After this many rate-limit hits in the ban window, cool the IP down.
    pub abuse_hits: u32,
    pub abuse_ban_secs: u64,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            general_per_min: 180,
            auth_per_min: 20,
            enroll_per_min: 30,
            seal_per_min: 12,
            upload_per_min: 3000,
            device_uploads_per_hour: 10,
            device_upload_window_secs: 3600,
            abuse_hits: 40,
            abuse_ban_secs: 120,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrafficClass {
    Exempt,
    General,
    Auth,
    Enroll,
    Seal,
    Upload,
}

impl TrafficClass {
    fn budget(self, cfg: &RateLimitConfig) -> Option<u32> {
        match self {
            TrafficClass::Exempt => None,
            TrafficClass::General => Some(cfg.general_per_min),
            TrafficClass::Auth => Some(cfg.auth_per_min),
            TrafficClass::Enroll => Some(cfg.enroll_per_min),
            TrafficClass::Seal => Some(cfg.seal_per_min),
            TrafficClass::Upload => Some(cfg.upload_per_min),
        }
    }
}

#[derive(Debug)]
struct Window {
    /// Unix-ish second buckets within the last minute (count per second).
    counts: [u16; 60],
    /// Which absolute minute-second the slot 0 maps to (monotonic secs % huge).
    epoch_sec: u64,
    total: u32,
}

impl Default for Window {
    fn default() -> Self {
        Self {
            counts: [0; 60],
            epoch_sec: 0,
            total: 0,
        }
    }
}

impl Window {
    fn add(&mut self, now: Instant, start: Instant, limit: u32) -> bool {
        let secs = now.duration_since(start).as_secs();
        // Rotate / expire slots older than 60s.
        if self.epoch_sec == 0 {
            self.epoch_sec = secs;
        }
        let advance = secs.saturating_sub(self.epoch_sec);
        if advance >= 60 {
            self.counts = [0; 60];
            self.total = 0;
            self.epoch_sec = secs;
        } else if advance > 0 {
            for _ in 0..advance {
                let idx = (self.epoch_sec % 60) as usize;
                self.total = self.total.saturating_sub(u32::from(self.counts[idx]));
                self.counts[idx] = 0;
                self.epoch_sec += 1;
            }
        }
        if self.total >= limit {
            return false;
        }
        let idx = (secs % 60) as usize;
        self.counts[idx] = self.counts[idx].saturating_add(1);
        self.total = self.total.saturating_add(1);
        true
    }
}

#[derive(Debug)]
struct IpState {
    general: Window,
    auth: Window,
    enroll: Window,
    seal: Window,
    upload: Window,
    abuse_hits: u32,
    banned_until: Option<Instant>,
    last_seen: Instant,
}

impl IpState {
    fn new(now: Instant) -> Self {
        Self {
            general: Window::default(),
            auth: Window::default(),
            enroll: Window::default(),
            seal: Window::default(),
            upload: Window::default(),
            abuse_hits: 0,
            banned_until: None,
            last_seen: now,
        }
    }
}

#[derive(Debug, Default)]
struct DeviceUploadWindow {
    /// Timestamps of accepted `POST /v1/uploads` creates.
    events: VecDeque<Instant>,
    last_seen: Option<Instant>,
}

impl DeviceUploadWindow {
    fn prune(&mut self, now: Instant, window: Duration) {
        while let Some(front) = self.events.front() {
            if now.duration_since(*front) > window {
                self.events.pop_front();
            } else {
                break;
            }
        }
    }

    /// Returns Ok(()) and records the event, or Err(retry_after).
    fn try_add(&mut self, now: Instant, limit: u32, window: Duration) -> Result<(), Duration> {
        self.prune(now, window);
        self.last_seen = Some(now);
        if limit == 0 {
            return Err(window);
        }
        if self.events.len() as u32 >= limit {
            let oldest = self.events.front().copied().unwrap_or(now);
            let retry = window.saturating_sub(now.duration_since(oldest));
            return Err(retry.max(Duration::from_secs(1)));
        }
        self.events.push_back(now);
        Ok(())
    }
}

#[derive(Debug)]
struct Inner {
    start: Instant,
    by_ip: HashMap<String, IpState>,
    by_device: HashMap<String, DeviceUploadWindow>,
    last_gc: Instant,
}

#[derive(Clone)]
pub struct RateLimiter {
    cfg: RateLimitConfig,
    inner: Arc<Mutex<Inner>>,
}

impl RateLimiter {
    pub fn new(cfg: RateLimitConfig) -> Self {
        let now = Instant::now();
        Self {
            cfg,
            inner: Arc::new(Mutex::new(Inner {
                start: now,
                by_ip: HashMap::new(),
                by_device: HashMap::new(),
                last_gc: now,
            })),
        }
    }

    pub fn config(&self) -> &RateLimitConfig {
        &self.cfg
    }

    /// Rolling per-device quota for new upload sessions (one report / artefact).
    /// `limit == 0` means unlimited. Independent of chunk PUT volume.
    pub fn check_device_upload(&self, device_id: &str, limit: u32) -> Result<(), Duration> {
        if !self.cfg.enabled || limit == 0 {
            return Ok(());
        }
        let window = Duration::from_secs(self.cfg.device_upload_window_secs.max(60));
        let key = device_id.trim();
        if key.is_empty() {
            return Ok(());
        }

        let mut g = self.inner.lock().unwrap();
        let now = Instant::now();
        if now.duration_since(g.last_gc) > Duration::from_secs(120) {
            g.by_ip
                .retain(|_, st| now.duration_since(st.last_seen) < Duration::from_secs(300));
            g.by_device.retain(|_, st| {
                st.last_seen
                    .is_some_and(|t| now.duration_since(t) < window + Duration::from_secs(300))
            });
            g.last_gc = now;
        }

        let st = g.by_device.entry(key.to_string()).or_default();
        st.try_add(now, limit, window)
    }

    fn classify(method: &str, path: &str) -> TrafficClass {
        let p = path.split('?').next().unwrap_or(path);
        if p == "/healthz" || p == "/admin" || p == "/admin/" || p.starts_with("/admin/") {
            return TrafficClass::Exempt;
        }
        // Auth *writes* stay throttled. Other /v1/admin/* is gated by session
        // cookies already — do not rate-limit the console refresh loop or a
        // successful sign-in looks like "login failed" after restart.
        if p.starts_with("/v1/admin/auth/login")
            || p.starts_with("/v1/admin/auth/setup")
            || p.starts_with("/v1/admin/auth/mfa")
            || p.starts_with("/v1/admin/auth/password")
        {
            return TrafficClass::Auth;
        }
        if p.starts_with("/v1/admin/") {
            return TrafficClass::Exempt;
        }
        if p == "/v1/enroll" || p.starts_with("/v1/enroll/") {
            return TrafficClass::Enroll;
        }
        if p == "/v1/lab/seal" {
            return TrafficClass::Seal;
        }
        if p == "/v1/uploads"
            || (p.starts_with("/v1/uploads/")
                && (method.eq_ignore_ascii_case("PUT")
                    || method.eq_ignore_ascii_case("POST")
                    || p.contains("/chunks/")))
        {
            return TrafficClass::Upload;
        }
        TrafficClass::General
    }

    /// Clear counters / cool-down for an IP (call after successful login).
    pub fn clear_ip(&self, ip: &str) {
        let mut g = self.inner.lock().unwrap();
        g.by_ip.remove(ip);
    }

    fn check(&self, ip: &str, class: TrafficClass) -> Result<(), Duration> {
        if !self.cfg.enabled {
            return Ok(());
        }
        let Some(limit) = class.budget(&self.cfg) else {
            return Ok(());
        };

        let mut g = self.inner.lock().unwrap();
        let now = Instant::now();
        // GC idle IPs every ~2 minutes.
        if now.duration_since(g.last_gc) > Duration::from_secs(120) {
            g.by_ip
                .retain(|_, st| now.duration_since(st.last_seen) < Duration::from_secs(300));
            g.last_gc = now;
        }

        let start = g.start;
        let st = g
            .by_ip
            .entry(ip.to_string())
            .or_insert_with(|| IpState::new(now));
        st.last_seen = now;

        if let Some(until) = st.banned_until {
            if now < until {
                return Err(until.saturating_duration_since(now));
            }
            st.banned_until = None;
            st.abuse_hits = 0;
        }

        let window = match class {
            TrafficClass::Exempt => return Ok(()),
            TrafficClass::General => &mut st.general,
            TrafficClass::Auth => &mut st.auth,
            TrafficClass::Enroll => &mut st.enroll,
            TrafficClass::Seal => &mut st.seal,
            TrafficClass::Upload => &mut st.upload,
        };

        if window.add(now, start, limit) {
            return Ok(());
        }

        st.abuse_hits = st.abuse_hits.saturating_add(1);
        if st.abuse_hits >= self.cfg.abuse_hits {
            let ban = Duration::from_secs(self.cfg.abuse_ban_secs.max(30));
            st.banned_until = Some(now + ban);
            st.abuse_hits = 0;
            tracing::warn!(
                remote = %ip,
                class = ?class,
                ban_secs = ban.as_secs(),
                "rate-limit abuse cool-down"
            );
            return Err(ban);
        }

        Err(Duration::from_secs(5))
    }
}

fn client_ip(req: &Request) -> String {
    let peer_ip = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|connect| connect.0.ip());
    // Forwarded headers are only trusted from a loopback reverse proxy;
    // otherwise the remote client could spoof a fresh address on every call.
    if peer_ip.is_some_and(|ip| ip.is_loopback()) {
        if let Some(xff) = req
            .headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
        {
            let first = xff.split(',').next().unwrap_or(xff).trim();
            if !first.is_empty() {
                return first.to_string();
            }
        }
        if let Some(real) = req
            .headers()
            .get("x-real-ip")
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return real.to_string();
        }
    }
    peer_ip
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "unknown".into())
}

pub async fn middleware(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let limiter = &state.rate_limiter;
    if !limiter.cfg.enabled {
        return next.run(req).await;
    }

    let method = req.method().as_str().to_string();
    let path = req.uri().path().to_string();
    let class = RateLimiter::classify(&method, &path);
    let ip = client_ip(&req);

    if let Err(retry) = limiter.check(&ip, class) {
        let secs = retry.as_secs().max(1);
        tracing::warn!(
            remote = %ip,
            method = %method,
            path = %path,
            class = ?class,
            retry_after = secs,
            "rate limit exceeded"
        );
        let mut res = (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({
                "error": "too many requests — slow down",
                "retry_after_secs": secs,
            })),
        )
            .into_response();
        if let Ok(v) = HeaderValue::from_str(&secs.to_string()) {
            res.headers_mut().insert(header::RETRY_AFTER, v);
        }
        return res;
    }

    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_class_is_stricter_path() {
        assert_eq!(
            RateLimiter::classify("POST", "/v1/admin/auth/login"),
            TrafficClass::Auth
        );
        assert_eq!(
            RateLimiter::classify("POST", "/v1/enroll/start"),
            TrafficClass::Enroll
        );
        assert_eq!(
            RateLimiter::classify("POST", "/v1/lab/seal"),
            TrafficClass::Seal
        );
        assert_eq!(
            RateLimiter::classify("PUT", "/v1/uploads/abc/chunks/0"),
            TrafficClass::Upload
        );
        assert_eq!(
            RateLimiter::classify("GET", "/healthz"),
            TrafficClass::Exempt
        );
    }

    #[test]
    fn window_enforces_budget() {
        let lim = RateLimiter::new(RateLimitConfig {
            general_per_min: 3,
            abuse_hits: 100,
            ..RateLimitConfig::default()
        });
        assert!(lim.check("1.2.3.4", TrafficClass::General).is_ok());
        assert!(lim.check("1.2.3.4", TrafficClass::General).is_ok());
        assert!(lim.check("1.2.3.4", TrafficClass::General).is_ok());
        assert!(lim.check("1.2.3.4", TrafficClass::General).is_err());
        // Other IP unaffected
        assert!(lim.check("9.9.9.9", TrafficClass::General).is_ok());
    }

    #[test]
    fn device_upload_quota_is_per_device() {
        let lim = RateLimiter::new(RateLimitConfig {
            device_uploads_per_hour: 2,
            device_upload_window_secs: 3600,
            ..RateLimitConfig::default()
        });
        assert!(lim.check_device_upload("dev-a", 2).is_ok());
        assert!(lim.check_device_upload("dev-a", 2).is_ok());
        assert!(lim.check_device_upload("dev-a", 2).is_err());
        assert!(lim.check_device_upload("dev-b", 2).is_ok());
        assert!(lim.check_device_upload("dev-a", 0).is_ok()); // unlimited
    }
}
