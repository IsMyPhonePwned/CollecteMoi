//! API access audit log — who hit which endpoint (blob downloads, uploads, etc.).

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::{
    Json,
    body::HttpBody,
    extract::{ConnectInfo, Query, Request, State},
    http::HeaderMap,
    middleware::Next,
    response::Response,
};
use collectemoi_db::{AccessEventRow, Db};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::net::SocketAddr;

use crate::AppState;

/// Rolling window used for live RX/TX rates on the admin overview.
const BANDWIDTH_WINDOW_SECS: u64 = 10;
/// Seconds of per-bucket history exposed for the dashboard network graph.
const BANDWIDTH_HISTORY_SECS: usize = 90;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessEvent {
    pub ts: String,
    pub method: String,
    pub path: String,
    pub status: u16,
    pub duration_ms: u64,
    pub remote: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub user_agent: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub device_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub resource_id: String,
    /// True when the request carried a valid full admin session cookie.
    #[serde(default)]
    pub is_admin: bool,
    /// Admin username when `is_admin` (empty for clients / anonymous).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub admin_user: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub app_version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub platform: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub os_version: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AccessStats {
    pub total: u64,
    pub since_boot: u64,
    pub by_kind: Vec<KindCount>,
    pub by_path: Vec<PathCount>,
    pub by_remote: Vec<RemoteCount>,
    pub blob_access: Vec<BlobAccessStat>,
    pub recent: Vec<AccessEvent>,
}

#[derive(Debug, Clone, Serialize)]
pub struct KindCount {
    pub kind: String,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PathCount {
    pub path: String,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RemoteCount {
    pub remote: String,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BlobAccessStat {
    pub blob_id: String,
    pub downloads: u64,
    pub meta_gets: u64,
    pub lists: u64,
    pub deletes: u64,
    pub last_at: String,
    pub last_remote: String,
    pub last_device_id: String,
}

#[derive(Default)]
struct Counters {
    total: u64,
    by_kind: HashMap<String, u64>,
    by_path: HashMap<String, u64>,
    by_remote: HashMap<String, u64>,
    /// blob_id → (downloads, meta, lists, deletes, last_at, last_remote, last_device)
    blobs: HashMap<String, (u64, u64, u64, u64, String, String, String)>,
    recent: Vec<AccessEvent>,
}

/// Live HTTP body bandwidth (Content-Length based; chunked bodies without CL under-count).
#[derive(Debug, Clone, Serialize)]
pub struct BandwidthStats {
    /// Inbound bytes/sec over the last `window_secs`.
    pub rx_bps: u64,
    /// Outbound bytes/sec over the last `window_secs`.
    pub tx_bps: u64,
    /// Total inbound bytes since process start.
    pub rx_total: u64,
    /// Total outbound bytes since process start.
    pub tx_total: u64,
    pub window_secs: u64,
    /// Peak combined rate in the history window (for graph scaling).
    pub peak_bps: u64,
    /// Per-second samples (oldest → newest) for the dashboard chart.
    pub history: Vec<BandwidthPoint>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BandwidthPoint {
    /// Unix seconds for this 1s bucket.
    pub t: u64,
    pub rx_bps: u64,
    pub tx_bps: u64,
}

#[derive(Default)]
struct BandwidthMeter {
    rx_total: u64,
    tx_total: u64,
    /// Per-request samples: (when, rx_bytes, tx_bytes).
    samples: VecDeque<(Instant, u64, u64)>,
    /// Closed 1-second buckets: (unix_secs, rx_bytes, tx_bytes).
    history: VecDeque<(u64, u64, u64)>,
    /// Open bucket being filled.
    bucket_secs: u64,
    bucket_rx: u64,
    bucket_tx: u64,
}

fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl BandwidthMeter {
    fn record(&mut self, rx: u64, tx: u64) {
        if rx == 0 && tx == 0 {
            return;
        }
        self.rx_total = self.rx_total.saturating_add(rx);
        self.tx_total = self.tx_total.saturating_add(tx);
        let now = Instant::now();
        self.samples.push_back((now, rx, tx));
        self.prune(now);
        self.roll_buckets(unix_now_secs());
        self.bucket_rx = self.bucket_rx.saturating_add(rx);
        self.bucket_tx = self.bucket_tx.saturating_add(tx);
    }

    fn prune(&mut self, now: Instant) {
        let window = Duration::from_secs(BANDWIDTH_WINDOW_SECS);
        while let Some((at, _, _)) = self.samples.front() {
            if now.duration_since(*at) > window {
                self.samples.pop_front();
            } else {
                break;
            }
        }
    }

    /// Close finished 1s buckets (including idle gaps) into `history`.
    fn roll_buckets(&mut self, now_secs: u64) {
        if now_secs == 0 {
            return;
        }
        if self.bucket_secs == 0 {
            self.bucket_secs = now_secs;
            return;
        }
        while self.bucket_secs < now_secs {
            self.history
                .push_back((self.bucket_secs, self.bucket_rx, self.bucket_tx));
            self.bucket_rx = 0;
            self.bucket_tx = 0;
            self.bucket_secs = self.bucket_secs.saturating_add(1);
            while self.history.len() > BANDWIDTH_HISTORY_SECS {
                self.history.pop_front();
            }
        }
    }

    fn snapshot(&mut self) -> BandwidthStats {
        let now = Instant::now();
        self.prune(now);
        let now_secs = unix_now_secs();
        self.roll_buckets(now_secs);
        let (rx, tx) = self
            .samples
            .iter()
            .fold((0u64, 0u64), |(ar, at), (_, r, t)| {
                (ar.saturating_add(*r), at.saturating_add(*t))
            });
        let secs = BANDWIDTH_WINDOW_SECS.max(1) as f64;
        let mut history: Vec<BandwidthPoint> = self
            .history
            .iter()
            .map(|(t, r, x)| BandwidthPoint {
                t: *t,
                rx_bps: *r,
                tx_bps: *x,
            })
            .collect();
        // Include the open second so the live tip of the graph moves.
        history.push(BandwidthPoint {
            t: now_secs,
            rx_bps: self.bucket_rx,
            tx_bps: self.bucket_tx,
        });
        // Pad leading zeros so a quiet lab still shows a full-width chart.
        if let Some(first) = history.first().map(|p| p.t) {
            let want_start = now_secs.saturating_sub(BANDWIDTH_HISTORY_SECS as u64 - 1);
            if first > want_start {
                let mut pad = Vec::new();
                let mut t = want_start;
                while t < first {
                    pad.push(BandwidthPoint {
                        t,
                        rx_bps: 0,
                        tx_bps: 0,
                    });
                    t += 1;
                }
                pad.extend(history);
                history = pad;
            }
        } else {
            let want_start = now_secs.saturating_sub(BANDWIDTH_HISTORY_SECS as u64 - 1);
            history = (want_start..=now_secs)
                .map(|t| BandwidthPoint {
                    t,
                    rx_bps: 0,
                    tx_bps: 0,
                })
                .collect();
        }
        while history.len() > BANDWIDTH_HISTORY_SECS {
            history.remove(0);
        }
        let peak_bps = history
            .iter()
            .map(|p| p.rx_bps.saturating_add(p.tx_bps))
            .max()
            .unwrap_or(0);
        BandwidthStats {
            rx_bps: (rx as f64 / secs).round() as u64,
            tx_bps: (tx as f64 / secs).round() as u64,
            rx_total: self.rx_total,
            tx_total: self.tx_total,
            window_secs: BANDWIDTH_WINDOW_SECS,
            peak_bps,
            history,
        }
    }
}

pub struct AccessLog {
    db: Arc<Db>,
    inner: Mutex<Counters>,
    bandwidth: Mutex<BandwidthMeter>,
}

impl AccessLog {
    pub fn open(_data_dir: &Path, db: Arc<Db>) -> anyhow::Result<Self> {
        let mut counters = Counters::default();
        // Warm counters from SQLite (newest first, last ~20k).
        if let Ok(rows) = db.access_list(20_000, None, None) {
            for row in rows.into_iter().rev() {
                apply_event(&mut counters, &row_to_event(row), false);
            }
        }
        Ok(Self {
            db,
            inner: Mutex::new(counters),
            bandwidth: Mutex::new(BandwidthMeter::default()),
        })
    }

    /// Drop in-memory counters after a DB purge of `access_events`.
    pub fn reset_after_purge(&self) {
        if let Ok(mut g) = self.inner.lock() {
            *g = Counters::default();
        }
    }

    pub fn record_bandwidth(&self, rx: u64, tx: u64) {
        if let Ok(mut g) = self.bandwidth.lock() {
            g.record(rx, tx);
        }
    }

    pub fn bandwidth(&self) -> BandwidthStats {
        self.bandwidth
            .lock()
            .map(|mut g| g.snapshot())
            .unwrap_or(BandwidthStats {
                rx_bps: 0,
                tx_bps: 0,
                rx_total: 0,
                tx_total: 0,
                window_secs: BANDWIDTH_WINDOW_SECS,
                peak_bps: 0,
                history: Vec::new(),
            })
    }

    pub fn record(&self, event: AccessEvent) {
        let _ = self.db.access_record(&AccessEventRow {
            ts: event.ts.clone(),
            method: event.method.clone(),
            path: event.path.clone(),
            status: event.status as i32,
            duration_ms: event.duration_ms as i64,
            remote: event.remote.clone(),
            user_agent: event.user_agent.clone(),
            device_id: event.device_id.clone(),
            kind: event.kind.clone(),
            resource_id: event.resource_id.clone(),
            is_admin: event.is_admin,
            admin_user: event.admin_user.clone(),
            app_version: event.app_version.clone(),
            platform: event.platform.clone(),
            model: event.model.clone(),
            os_version: event.os_version.clone(),
        });
        let mut g = self.inner.lock().unwrap();
        apply_event(&mut g, &event, true);
    }

    pub fn stats(&self, recent_limit: usize) -> AccessStats {
        let g = self.inner.lock().unwrap();
        let mut by_kind: Vec<_> = g
            .by_kind
            .iter()
            .map(|(k, c)| KindCount {
                kind: k.clone(),
                count: *c,
            })
            .collect();
        by_kind.sort_by(|a, b| b.count.cmp(&a.count).then(a.kind.cmp(&b.kind)));

        let mut by_path: Vec<_> = g
            .by_path
            .iter()
            .map(|(p, c)| PathCount {
                path: p.clone(),
                count: *c,
            })
            .collect();
        by_path.sort_by(|a, b| b.count.cmp(&a.count).then(a.path.cmp(&b.path)));
        by_path.truncate(40);

        let mut by_remote: Vec<_> = g
            .by_remote
            .iter()
            .map(|(r, c)| RemoteCount {
                remote: r.clone(),
                count: *c,
            })
            .collect();
        by_remote.sort_by(|a, b| b.count.cmp(&a.count).then(a.remote.cmp(&b.remote)));
        by_remote.truncate(40);

        let mut blob_access: Vec<_> = g
            .blobs
            .iter()
            .map(|(id, entry)| {
                let (dl, meta, lists, dels, last_at, last_remote, last_device) = entry;
                BlobAccessStat {
                    blob_id: id.clone(),
                    downloads: *dl,
                    meta_gets: *meta,
                    lists: *lists,
                    deletes: *dels,
                    last_at: last_at.clone(),
                    last_remote: last_remote.clone(),
                    last_device_id: last_device.clone(),
                }
            })
            .collect();
        blob_access.sort_by(|a, b| {
            (b.downloads + b.meta_gets)
                .cmp(&(a.downloads + a.meta_gets))
                .then(b.last_at.cmp(&a.last_at))
        });

        let limit = recent_limit.clamp(1, 500);
        let recent: Vec<_> = g.recent.iter().rev().take(limit).cloned().collect();

        AccessStats {
            total: g.total,
            since_boot: g.total,
            by_kind,
            by_path,
            by_remote,
            blob_access,
            recent,
        }
    }

    pub fn list(
        &self,
        limit: usize,
        kind: Option<&str>,
        remote: Option<&str>,
        resource_id: Option<&str>,
    ) -> Vec<AccessEvent> {
        let limit = limit.clamp(1, 2000);
        // Prefer SQL; resource_id filter applied after fetch when needed.
        let mut events: Vec<AccessEvent> = self
            .db
            .access_list(limit.saturating_mul(2).min(2000), kind, remote)
            .unwrap_or_default()
            .into_iter()
            .map(row_to_event)
            .collect();
        if let Some(id) = resource_id.filter(|s| !s.is_empty()) {
            events.retain(|e| e.resource_id == id);
        }
        events.truncate(limit);
        events
    }

    /// Recent access events for one blob (downloads / meta gets / deletes).
    pub fn blob_history(&self, blob_id: &str, limit: usize) -> Vec<AccessEvent> {
        self.db
            .access_for_resource(blob_id, limit)
            .unwrap_or_default()
            .into_iter()
            .map(row_to_event)
            .filter(|e| e.kind.starts_with("blob_"))
            .collect()
    }

    /// Histories for many blobs (parameterized per-id queries).
    pub fn blob_histories(
        &self,
        blob_ids: &[String],
        limit_per: usize,
    ) -> HashMap<String, Vec<AccessEvent>> {
        let mut out: HashMap<String, Vec<AccessEvent>> = HashMap::new();
        for id in blob_ids {
            let hist = self.blob_history(id, limit_per);
            if !hist.is_empty() {
                out.insert(id.clone(), hist);
            }
        }
        out
    }
}

fn row_to_event(r: AccessEventRow) -> AccessEvent {
    AccessEvent {
        ts: r.ts,
        method: r.method,
        path: r.path,
        status: r.status.max(0) as u16,
        duration_ms: r.duration_ms.max(0) as u64,
        remote: r.remote,
        user_agent: r.user_agent,
        device_id: r.device_id,
        kind: r.kind,
        resource_id: r.resource_id,
        is_admin: r.is_admin,
        admin_user: r.admin_user,
        app_version: r.app_version,
        platform: r.platform,
        model: r.model,
        os_version: r.os_version,
    }
}

fn apply_event(c: &mut Counters, ev: &AccessEvent, push_recent: bool) {
    c.total += 1;
    *c.by_kind.entry(ev.kind.clone()).or_default() += 1;
    let path_key = normalize_path_key(&ev.method, &ev.path);
    *c.by_path.entry(path_key).or_default() += 1;
    if !ev.remote.is_empty() {
        *c.by_remote.entry(ev.remote.clone()).or_default() += 1;
    }

    if ev.kind.starts_with("blob_") {
        let id = if ev.resource_id.is_empty() {
            "_list_".to_string()
        } else {
            ev.resource_id.clone()
        };
        let entry =
            c.blobs
                .entry(id)
                .or_insert((0, 0, 0, 0, String::new(), String::new(), String::new()));
        match ev.kind.as_str() {
            "blob_download" => entry.0 += 1,
            "blob_get" => entry.1 += 1,
            "blob_list" => entry.2 += 1,
            "blob_delete" => entry.3 += 1,
            _ => {}
        }
        entry.4 = ev.ts.clone();
        entry.5 = ev.remote.clone();
        entry.6 = ev.device_id.clone();
    }

    if push_recent {
        c.recent.push(ev.clone());
        if c.recent.len() > 1000 {
            let drain = c.recent.len() - 800;
            c.recent.drain(0..drain);
        }
    }
}

fn normalize_path_key(method: &str, path: &str) -> String {
    let base = path.split('?').next().unwrap_or(path);
    let mut parts: Vec<&str> = base.split('/').filter(|p| !p.is_empty()).collect();
    for p in parts.iter_mut() {
        if looks_like_id(p) {
            *p = ":id";
        } else if p.chars().all(|c| c.is_ascii_digit()) && p.len() <= 6 {
            *p = ":n";
        }
    }
    format!("{method} /{}", parts.join("/"))
}

fn looks_like_id(s: &str) -> bool {
    (s.len() == 36 && s.chars().filter(|c| *c == '-').count() == 4)
        || (s.len() >= 32 && s.chars().all(|c| c.is_ascii_hexdigit()))
}

pub fn classify(method: &str, path: &str) -> (String, String) {
    let (p, query) = match path.split_once('?') {
        Some((base, q)) => (base, q),
        None => (path, ""),
    };
    let parts: Vec<&str> = p.split('/').filter(|x| !x.is_empty()).collect();
    let preview_fetch = query.split('&').any(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        k == "preview" && (v.is_empty() || v == "1" || v.eq_ignore_ascii_case("true"))
    });

    if parts.len() == 3 && parts[0] == "v1" && parts[1] == "client" && parts[2] == "submissions" {
        return ("client_history".into(), String::new());
    }

    // /v1/admin/blobs/:id/content  or /v1/blobs/:id/content
    if parts.len() >= 3 && parts[0] == "v1" {
        let is_admin_blobs = parts.get(1) == Some(&"admin") && parts.get(2) == Some(&"blobs");
        let is_blobs = parts.get(1) == Some(&"blobs");
        if is_admin_blobs || is_blobs {
            let offset = if is_admin_blobs { 3 } else { 2 };
            if parts.len() == offset {
                return ("blob_list".into(), String::new());
            }
            let id = parts.get(offset).copied().unwrap_or("").to_string();
            if parts.get(offset + 1) == Some(&"content") {
                // Admin message-report (and similar) inline previews must not inflate DL counts.
                if preview_fetch {
                    return ("blob_preview".into(), id);
                }
                return ("blob_download".into(), id);
            }
            if method.eq_ignore_ascii_case("DELETE") {
                return ("blob_delete".into(), id);
            }
            return ("blob_get".into(), id);
        }
        if parts.get(1) == Some(&"uploads") {
            if parts.len() == 2 && method.eq_ignore_ascii_case("POST") {
                return ("upload_create".into(), String::new());
            }
            if parts.len() >= 4 && parts[3] == "chunks" {
                return (
                    "upload_chunk".into(),
                    parts.get(2).unwrap_or(&"").to_string(),
                );
            }
            if parts.len() >= 4 && parts[3] == "complete" {
                return (
                    "upload_complete".into(),
                    parts.get(2).unwrap_or(&"").to_string(),
                );
            }
            if parts.len() >= 3 {
                return ("upload_get".into(), parts.get(2).unwrap_or(&"").to_string());
            }
        }
        if parts.get(1) == Some(&"enroll") {
            return match parts.get(2).copied() {
                Some("start") => ("enroll_start".into(), String::new()),
                Some("confirm") => ("enroll_confirm".into(), String::new()),
                _ => ("enroll".into(), String::new()),
            };
        }
        if parts.get(1) == Some(&"lab") && parts.get(2) == Some(&"seal") {
            return ("lab_seal".into(), String::new());
        }
        if parts.get(1) == Some(&"admin") && parts.get(2) == Some(&"mobile") {
            if parts.len() == 3 {
                return ("mobile_list".into(), String::new());
            }
            return (
                "mobile_download".into(),
                parts.get(3).unwrap_or(&"").to_string(),
            );
        }
        if parts.get(1) == Some(&"admin") {
            return ("admin".into(), String::new());
        }
    }
    if p == "/healthz" {
        return ("health".into(), String::new());
    }
    if p.starts_with("/admin") {
        return ("admin_ui".into(), String::new());
    }
    ("other".into(), String::new())
}

/// Endpoints polled by the Logs / API-access panels themselves — recording them
/// would create a feedback loop that floods the connection log.
fn should_skip_persist(path: &str) -> bool {
    let p = path.split('?').next().unwrap_or(path);
    matches!(
        p,
        "/v1/admin/logs" | "/v1/admin/access-log" | "/v1/admin/access-stats"
    )
}

/// Admin console + health traffic must not drive the overview bandwidth chart.
fn skip_bandwidth_meter(path: &str, kind: &str, is_admin: bool) -> bool {
    if is_admin {
        return true;
    }
    let p = path.split('?').next().unwrap_or(path);
    if p == "/admin" || p.starts_with("/admin/") || p == "/favicon.png" || p == "/healthz" {
        return true;
    }
    matches!(kind, "admin" | "admin_ui" | "health")
}

fn header_content_length(headers: &HeaderMap) -> u64 {
    headers
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
}

fn body_bytes_estimate(headers: &HeaderMap, exact_hint: Option<u64>) -> u64 {
    header_content_length(headers).max(exact_hint.unwrap_or(0))
}

pub async fn middleware(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let method = req.method().to_string();
    let path = req
        .uri()
        .path_and_query()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| req.uri().path().to_string());
    let path_only = req.uri().path().to_string();

    let user_agent = req
        .headers()
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(180)
        .collect::<String>();
    let device_id = req
        .headers()
        .get("x-device-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(80)
        .collect::<String>();
    let app_version = req
        .headers()
        .get("x-app-version")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(64)
        .collect::<String>();
    let platform = req
        .headers()
        .get("x-device-platform")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(32)
        .collect::<String>();
    let model = req
        .headers()
        .get("x-device-model")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(80)
        .collect::<String>();
    let os_version = req
        .headers()
        .get("x-device-os")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(64)
        .collect::<String>();
    let manufacturer = req
        .headers()
        .get("x-device-manufacturer")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(80)
        .collect::<String>();
    let brand = req
        .headers()
        .get("x-device-brand")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(80)
        .collect::<String>();
    let device_name = req
        .headers()
        .get("x-device-name")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(120)
        .collect::<String>();
    let security_patch = req
        .headers()
        .get("x-device-security-patch")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(32)
        .collect::<String>();
    let api_level = req
        .headers()
        .get("x-device-api-level")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(16)
        .collect::<String>();
    let build_id = req
        .headers()
        .get("x-device-build-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(64)
        .collect::<String>();
    let build_display = req
        .headers()
        .get("x-device-build-display")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(96)
        .collect::<String>();
    let build_incremental = req
        .headers()
        .get("x-device-build-incremental")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(64)
        .collect::<String>();
    let build_type = req
        .headers()
        .get("x-device-build-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(32)
        .collect::<String>();
    let android_release = req
        .headers()
        .get("x-device-android-release")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(32)
        .collect::<String>();
    let physical = req
        .headers()
        .get("x-device-physical")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(16)
        .collect::<String>();
    let low_ram = req
        .headers()
        .get("x-device-low-ram")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(8)
        .collect::<String>();
    let ram_mb = req
        .headers()
        .get("x-device-ram-mb")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(16)
        .collect::<String>();
    let disk_free_mb = req
        .headers()
        .get("x-device-disk-free-mb")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(16)
        .collect::<String>();
    let forwarded = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').next().unwrap_or(s).trim().to_string())
        .filter(|s| !s.is_empty());
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.to_string());
    let remote = forwarded.or(peer).unwrap_or_else(|| "unknown".into());

    let admin_user = crate::auth::admin_identity_for_log(&state, req.headers()).unwrap_or_default();
    let is_admin = !admin_user.is_empty();
    let actor = if is_admin { "admin" } else { "client" };

    // Prefer Content-Length; fall back to body size_hint (covers many JSON Full bodies).
    let rx_bytes = body_bytes_estimate(req.headers(), req.body().size_hint().exact());
    // Use path+query so `?preview=1` classifies as blob_preview (not blob_download).
    let (kind, resource_id) = classify(&method, &path);
    let started = Instant::now();
    let response = next.run(req).await;
    let status = response.status().as_u16();
    let duration_ms = started.elapsed().as_millis() as u64;
    let tx_bytes = body_bytes_estimate(response.headers(), response.body().size_hint().exact());
    // Live bandwidth KPIs are for device / public API traffic only. Counting
    // Admin UI polls (overview refresh, blob lists, logs) made every F5 look
    // like a multi‑MB/s "upload".
    if !skip_bandwidth_meter(&path_only, &kind, is_admin) {
        state.access.record_bandwidth(rx_bytes, tx_bytes);
    }

    // Attribute enrolled clients even on /healthz (Devices → Last seen / check-in).
    // Do this before access-log filtering so pings identify the phone without
    // flooding access_events (health is polled every ~20s).
    if !device_id.is_empty() && !is_admin {
        state.enrollment.touch_device(
            &device_id,
            &app_version,
            &platform,
            &model,
            &os_version,
            &manufacturer,
            &brand,
            &device_name,
            &[
                ("security_patch", security_patch.as_str()),
                ("api_level", api_level.as_str()),
                ("sdk", api_level.as_str()),
                ("build_id", build_id.as_str()),
                ("build_display", build_display.as_str()),
                ("android_build", build_display.as_str()),
                ("build_incremental", build_incremental.as_str()),
                ("build_type", build_type.as_str()),
                ("android_release", android_release.as_str()),
                ("physical", physical.as_str()),
                ("low_ram", low_ram.as_str()),
                ("ram_mb", ram_mb.as_str()),
                ("disk_free_mb", disk_free_mb.as_str()),
            ],
        );
    }

    // Always emit a structured connection line into process logs (api.log),
    // except for the log-panel self-pollers (avoid feedback when viewing Logs)
    // and high-frequency health pings.
    let skip_access_row = should_skip_persist(&path_only) || kind == "health";
    if !skip_access_row {
        tracing::info!(
            target: "collectemoi_api::access",
            method = %method,
            path = %path,
            status,
            duration_ms,
            remote = %remote,
            kind = %kind,
            resource_id = %resource_id,
            device_id = %device_id,
            app_version = %app_version,
            platform = %platform,
            model = %model,
            os_version = %os_version,
            actor = %actor,
            is_admin,
            admin_user = %admin_user,
            rx_bytes,
            tx_bytes,
            "connection"
        );

        state.access.record(AccessEvent {
            ts: now_secs_str(),
            method,
            path,
            status,
            duration_ms,
            remote,
            user_agent,
            device_id,
            kind,
            resource_id,
            is_admin,
            admin_user,
            app_version,
            platform,
            model,
            os_version,
        });
    } else if kind == "health" && !device_id.is_empty() {
        tracing::debug!(
            target: "collectemoi_api::access",
            device_id = %device_id,
            status,
            "health check-in"
        );
    }

    response
}

fn now_secs_str() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_default()
}

#[derive(Debug, Deserialize)]
pub struct AccessQuery {
    #[serde(default = "default_access_limit")]
    pub limit: usize,
    pub kind: Option<String>,
    pub remote: Option<String>,
    pub resource_id: Option<String>,
}

fn default_access_limit() -> usize {
    200
}

pub async fn access_stats(State(state): State<AppState>) -> Json<AccessStats> {
    Json(state.access.stats(80))
}

pub async fn access_log(
    State(state): State<AppState>,
    Query(q): Query<AccessQuery>,
) -> Json<serde_json::Value> {
    let events = state.access.list(
        q.limit,
        q.kind.as_deref(),
        q.remote.as_deref(),
        q.resource_id.as_deref(),
    );
    Json(json!({
        "events": events,
        "count": events.len(),
        "limit": q.limit.clamp(1, 2000),
    }))
}

#[cfg(test)]
mod tests {
    use super::{classify, skip_bandwidth_meter};

    #[test]
    fn content_without_query_is_download() {
        let (kind, id) = classify("GET", "/v1/admin/blobs/6b9daa16-8660-42e6-9434-45fce7ff73ba/content");
        assert_eq!(kind, "blob_download");
        assert_eq!(id, "6b9daa16-8660-42e6-9434-45fce7ff73ba");
    }

    #[test]
    fn content_with_preview_query_is_preview() {
        let (kind, id) = classify(
            "GET",
            "/v1/admin/blobs/6b9daa16-8660-42e6-9434-45fce7ff73ba/content?preview=1",
        );
        assert_eq!(kind, "blob_preview");
        assert_eq!(id, "6b9daa16-8660-42e6-9434-45fce7ff73ba");
    }

    #[test]
    fn bandwidth_meter_skips_admin_and_health() {
        assert!(skip_bandwidth_meter("/v1/admin/overview", "admin", true));
        assert!(skip_bandwidth_meter("/admin", "admin_ui", false));
        assert!(skip_bandwidth_meter("/healthz", "health", false));
        assert!(!skip_bandwidth_meter(
            "/v1/uploads/abc/chunks/0",
            "upload_chunk",
            false
        ));
    }
}
