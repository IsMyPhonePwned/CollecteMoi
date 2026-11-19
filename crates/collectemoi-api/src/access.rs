//! API access audit log — who hit which endpoint (blob downloads, uploads, etc.).

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::{
    Json,
    extract::{ConnectInfo, Query, Request, State},
    middleware::Next,
    response::Response,
};
use collectemoi_db::{AccessEventRow, Db};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::net::SocketAddr;

use crate::AppState;

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

pub struct AccessLog {
    db: Arc<Db>,
    inner: Mutex<Counters>,
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
    let p = path.split('?').next().unwrap_or(path);
    let parts: Vec<&str> = p.split('/').filter(|x| !x.is_empty()).collect();

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
        .to_string();
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

    let (kind, resource_id) = classify(&method, &path_only);
    let started = Instant::now();
    let response = next.run(req).await;
    let status = response.status().as_u16();
    let duration_ms = started.elapsed().as_millis() as u64;

    // Always emit a structured connection line into process logs (api.log),
    // except for the log-panel self-pollers (avoid feedback when viewing Logs).
    if !should_skip_persist(&path_only) {
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
        });
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
