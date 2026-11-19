//! Lab admin API + embedded console (gated by `auth` — password + TOTP / WebAuthn).

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::{Html, IntoResponse},
};
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use crate::AppState;
use crate::settings::SettingsPatch;

const ADMIN_HTML: &str = include_str!("../static/admin.html");

pub async fn console() -> Html<&'static str> {
    Html(ADMIN_HTML)
}

#[derive(Debug, Serialize)]
pub struct Overview {
    pub ok: bool,
    pub version: String,
    pub started_at: u64,
    pub uptime_secs: u64,
    pub settings: crate::settings::RuntimeSettings,
    pub devices: usize,
    pub uploads: usize,
    pub uploads_complete: usize,
    pub jobs_pending: usize,
    pub analysis_dirs: usize,
    pub blobs: usize,
    pub enrollment_tokens: usize,
    pub companies: usize,
    pub lab_keys: usize,
    pub enrollment_events: usize,
    pub banned_keys: usize,
    pub access_total: u64,
    pub blob_downloads: u64,
    pub case_public_key: String,
    pub recent_enrollment: Vec<crate::otp::EnrollmentLogEvent>,
}

fn build_overview(state: &AppState) -> Overview {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let settings = state.settings.lock().unwrap().clone();
    let devices = state.enrollment.list_devices().len();
    let tokens = state.enrollment.list_tokens().len();
    let (uploads_total, uploads_complete) = state.store.counts();
    let jobs_pending = state.db.job_count_pending().unwrap_or(0);
    let analysis_dirs = count_dirs(&state.data_dir.join("analysis"));
    let blobs = state.blobs.count();
    let companies = state.otp.list_companies().len();
    let lab_keys = state.otp.list_lab_keys().len();
    let recent_enrollment = state.otp.list_log(8);
    let enrollment_events = state
        .db
        .enrollment_list(10_000)
        .map(|v| v.len())
        .unwrap_or(recent_enrollment.len());
    let banned_keys = state.enrollment.list_banned_keys().len();
    let access = state.access.stats(0);
    let blob_downloads = access
        .by_kind
        .iter()
        .find(|k| k.kind == "blob_download")
        .map(|k| k.count)
        .unwrap_or(0);

    Overview {
        ok: true,
        version: env!("CARGO_PKG_VERSION").to_string(),
        started_at: state.started_at,
        uptime_secs: now.saturating_sub(state.started_at),
        settings,
        devices,
        uploads: uploads_total,
        uploads_complete,
        jobs_pending,
        analysis_dirs,
        blobs,
        enrollment_tokens: tokens,
        companies,
        lab_keys,
        enrollment_events,
        banned_keys,
        access_total: access.total,
        blob_downloads,
        case_public_key: state.enrollment.case_public_key_hex().to_string(),
        recent_enrollment,
    }
}

pub async fn overview(State(state): State<AppState>) -> Json<Overview> {
    Json(build_overview(&state))
}

/// Rich process status (same payload as overview) for ctl / ops.
pub async fn status(State(state): State<AppState>) -> Json<Overview> {
    Json(build_overview(&state))
}

pub async fn get_settings(State(state): State<AppState>) -> Json<crate::settings::RuntimeSettings> {
    Json(state.settings.lock().unwrap().clone())
}

pub async fn patch_settings(
    State(state): State<AppState>,
    Json(patch): Json<SettingsPatch>,
) -> Result<Json<crate::settings::RuntimeSettings>, (StatusCode, Json<serde_json::Value>)> {
    let mut settings = state.settings.lock().unwrap();
    settings.apply_patch(patch);
    state.enrollment.set_display_name(&settings.display_name);
    state.enrollment.set_public_url(&settings.public_url);
    if let Err(e) = crate::settings::save_persisted(&state.db, &settings) {
        tracing::warn!(error = %e, "failed to persist runtime settings");
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("settings saved in memory but not to disk: {e}") })),
        ));
    }
    Ok(Json(settings.clone()))
}

pub async fn list_devices(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({ "devices": state.enrollment.list_devices() }))
}

pub async fn list_client_keys(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({
        "keys": state.enrollment.list_client_keys(),
        "banned": state.enrollment.list_banned_keys(),
    }))
}

#[derive(Debug, serde::Deserialize)]
pub struct BanKeyRequest {
    pub public_key: String,
    #[serde(default)]
    pub reason: String,
}

pub async fn ban_public_key(
    State(state): State<AppState>,
    Json(req): Json<BanKeyRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let rec = state
        .enrollment
        .ban_public_key(&req.public_key, &req.reason)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))))?;
    Ok(Json(json!({ "banned": rec })))
}

pub async fn unban_public_key(
    State(state): State<AppState>,
    Json(req): Json<BanKeyRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    state
        .enrollment
        .unban_public_key(&req.public_key)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))))?;
    Ok(Json(json!({ "ok": true, "public_key": req.public_key })))
}

#[derive(Debug, serde::Deserialize)]
pub struct DeviceBanRequest {
    pub banned: bool,
}

pub async fn set_device_ban(
    State(state): State<AppState>,
    axum::extract::Path(device_id): axum::extract::Path<Uuid>,
    Json(req): Json<DeviceBanRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let view = state
        .enrollment
        .set_device_banned(device_id, req.banned)
        .map_err(|e| {
            let status = if e.contains("not found") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::BAD_REQUEST
            };
            (status, Json(json!({ "error": e })))
        })?;
    Ok(Json(json!({ "device": view })))
}

#[derive(Debug, serde::Deserialize)]
pub struct ListUploadsQuery {
    #[serde(default = "default_upload_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

fn default_upload_limit() -> usize {
    200
}

pub async fn list_uploads(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<ListUploadsQuery>,
) -> Json<serde_json::Value> {
    let (list, total) = state.store.list_page(q.limit, q.offset).unwrap_or_default();
    Json(json!({
        "uploads": list,
        "total": total,
        "limit": q.limit.clamp(1, 1000),
        "offset": q.offset,
    }))
}

pub async fn list_jobs(State(state): State<AppState>) -> Json<serde_json::Value> {
    let jobs: Vec<_> = state
        .db
        .job_list_all(500)
        .unwrap_or_default()
        .into_iter()
        .map(|j| {
            json!({
                "upload_id": j.upload_id,
                "case_id": j.case_id,
                "artefact": j.artefact,
                "envelope_path": j.envelope_path,
                "device_public_key": j.device_public_key,
                "queued_at": j.queued_at,
                "status": j.status,
                "claimed_at": j.claimed_at,
                "finished_at": j.finished_at,
                "error": j.error,
            })
        })
        .collect();
    Json(json!({ "jobs": jobs }))
}

pub async fn list_analysis(State(state): State<AppState>) -> Json<serde_json::Value> {
    let dir = state.data_dir.join("analysis");
    let mut items = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let id = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let modified_at = file_mtime_secs(&path).unwrap_or_default();
            let mut files = Vec::new();
            if let Ok(inner) = fs::read_dir(&path) {
                for f in inner.flatten() {
                    let meta = f.metadata().ok();
                    let mtime = meta
                        .as_ref()
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| {
                            t.duration_since(std::time::UNIX_EPOCH)
                                .ok()
                                .map(|d| d.as_secs().to_string())
                        })
                        .unwrap_or_default();
                    files.push(json!({
                        "name": f.file_name().to_string_lossy(),
                        "size": meta.map(|m| m.len()).unwrap_or(0),
                        "modified_at": mtime,
                    }));
                }
            }
            items.push(json!({
                "upload_id": id,
                "modified_at": modified_at,
                "files": files,
            }));
        }
    }
    items.sort_by(|a, b| {
        let ta = a.get("modified_at").and_then(|x| x.as_str()).unwrap_or("");
        let tb = b.get("modified_at").and_then(|x| x.as_str()).unwrap_or("");
        tb.cmp(ta)
    });
    Json(json!({ "analysis": items }))
}

fn file_mtime_secs(path: &Path) -> Option<String> {
    let meta = fs::metadata(path).ok()?;
    let t = meta.modified().ok()?;
    Some(
        t.duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs()
            .to_string(),
    )
}

pub async fn list_tokens(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({ "tokens": state.enrollment.list_tokens() }))
}

pub async fn list_companies(State(state): State<AppState>) -> Json<serde_json::Value> {
    let companies = state.otp.list_companies();
    let devices = state.enrollment.list_devices();
    let enriched: Vec<serde_json::Value> = companies
        .into_iter()
        .map(|c| {
            let device_count = devices.iter().filter(|d| d.company_id == c.id).count();
            let branding_root = state.data_dir.join("branding").join(&c.id);
            let versions = list_branding_versions_on_disk(&branding_root);
            let builds = count_mobile_builds(&state.data_dir, &c.id);
            json!({
                "id": c.id,
                "name": c.name,
                "domains": c.domains,
                "case_id": c.case_id,
                "enabled": c.enabled,
                "updated_at": c.updated_at,
                "color": c.color,
                "device_count": device_count,
                "branding_versions": versions.len(),
                "build_count": builds,
            })
        })
        .collect();
    Json(json!({ "companies": enriched }))
}

pub async fn get_company(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let company = state.otp.get_company(&id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("unknown company '{id}'") })),
        )
    })?;
    let devices: Vec<_> = state
        .enrollment
        .list_devices()
        .into_iter()
        .filter(|d| d.company_id == company.id)
        .collect();
    let lab_keys = state.otp.list_lab_keys_for_company(&company.id);
    let case_id = company.case_id;
    let tokens: Vec<_> = state
        .enrollment
        .list_tokens()
        .into_iter()
        .filter(|t| t.case_id == case_id)
        .collect();
    let banned = state.enrollment.list_banned_keys();
    let device_keys: Vec<_> = devices
        .iter()
        .filter(|d| !d.public_key.is_empty())
        .map(|d| {
            let is_banned = banned.iter().any(|b| b.public_key == d.public_key)
                || d.banned
                || d.public_key_banned;
            json!({
                "device_id": d.device_id,
                "email": d.email,
                "public_key": d.public_key,
                "noise_key_fingerprint": d.noise_key_fingerprint,
                "banned": is_banned,
            })
        })
        .collect();
    let events: Vec<_> = state
        .otp
        .list_log(200)
        .into_iter()
        .filter(|e| e.company_id.as_deref() == Some(company.id.as_str()))
        .take(50)
        .collect();
    let branding_root = state.data_dir.join("branding").join(&company.id);
    let versions = list_branding_versions_on_disk(&branding_root);
    let builds = list_mobile_builds_for_company(&state.data_dir, &company.id);
    Ok(Json(json!({
        "ok": true,
        "company": company,
        "devices": devices,
        "lab_keys": lab_keys,
        "tokens": tokens,
        "keys": device_keys,
        "enrollment_events": events,
        "branding": {
            "path": format!("data/branding/{}/", company.id),
            "versions": versions,
        },
        "builds": builds,
        "hint": "Pause blocks new enrollments for this company's domains. Lab keys and tokens minted here are scoped to this company.",
    })))
}

pub async fn upsert_company(
    State(state): State<AppState>,
    Json(body): Json<crate::otp::Company>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let c = state
        .otp
        .upsert_company(body)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))))?;
    Ok(Json(json!({ "company": c })))
}

#[derive(Debug, serde::Deserialize)]
pub struct CompanyEnabledBody {
    pub enabled: bool,
}

pub async fn set_company_enabled(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(body): Json<CompanyEnabledBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let c = state
        .otp
        .set_company_enabled(&id, body.enabled)
        .map_err(|e| {
            let status = if e.starts_with("unknown") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::BAD_REQUEST
            };
            (status, Json(json!({ "error": e })))
        })?;
    Ok(Json(json!({
        "ok": true,
        "company": c,
        "message": if c.enabled { "Company resumed" } else { "Company paused" },
    })))
}

pub async fn delete_company(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    state.otp.delete_company(&id).map_err(|e| {
        let status = if e.starts_with("unknown") {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::BAD_REQUEST
        };
        (status, Json(json!({ "error": e })))
    })?;
    Ok(Json(json!({
        "ok": true,
        "deleted": id,
        "hint": "Company record removed. Branding under data/branding/<id>/ was left on disk.",
    })))
}

fn list_branding_versions_on_disk(company_root: &Path) -> Vec<String> {
    let idx_path = company_root.join("versions.json");
    if let Ok(raw) = fs::read_to_string(&idx_path) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            if let Some(arr) = v.get("versions").and_then(|x| x.as_array()) {
                let mut out: Vec<String> = arr
                    .iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect();
                if !out.is_empty() {
                    out.sort();
                    out.dedup();
                    return out;
                }
            }
        }
    }
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(company_root) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                    if name != "assets" && !name.starts_with('.') {
                        out.push(name.to_string());
                    }
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn count_mobile_builds(data_dir: &Path, company_id: &str) -> usize {
    list_mobile_builds_for_company(data_dir, company_id).len()
}

fn list_mobile_builds_for_company(data_dir: &Path, company_id: &str) -> Vec<serde_json::Value> {
    let root = data_dir.join("mobile").join(company_id);
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let lower = name.to_ascii_lowercase();
            if name.ends_with(".meta.json") {
                continue;
            }
            if !(lower.ends_with(".apk")
                || lower.ends_with(".ipa")
                || (lower.ends_with(".zip") && lower.contains("ios")))
            {
                continue;
            }
            let meta = fs::metadata(&p).ok();
            let version_dir = p
                .parent()
                .and_then(|parent| parent.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            // If parent is the company folder itself, version is empty.
            let branding_version = if version_dir == company_id {
                String::new()
            } else {
                version_dir
            };
            let modified_at = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs().to_string())
                .unwrap_or_default();
            out.push(json!({
                "name": name,
                "path": p.strip_prefix(data_dir).map(|r| r.display().to_string()).unwrap_or_else(|_| p.display().to_string()),
                "company_id": company_id,
                "version": branding_version,
                "size": meta.as_ref().map(|m| m.len()).unwrap_or(0),
                "modified_at": modified_at,
                "platform": if lower.ends_with(".apk") { "android" } else { "ios" },
            }));
        }
    }
    out.sort_by(|a, b| {
        let av = a.get("version").and_then(|v| v.as_str()).unwrap_or("");
        let bv = b.get("version").and_then(|v| v.as_str()).unwrap_or("");
        bv.cmp(av).then_with(|| {
            a.get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .cmp(b.get("name").and_then(|v| v.as_str()).unwrap_or(""))
        })
    });
    out
}

pub async fn list_lab_keys(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({ "lab_keys": state.otp.list_lab_keys() }))
}

pub async fn upsert_lab_key(
    State(state): State<AppState>,
    Json(body): Json<crate::otp::LabKey>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let k = state
        .otp
        .upsert_lab_key(body)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))))?;
    Ok(Json(json!({ "lab_key": k })))
}

pub async fn delete_lab_key(
    State(state): State<AppState>,
    axum::extract::Path(key): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    state.otp.delete_lab_key(&key).map_err(|e| {
        let status = if e.starts_with("unknown") {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::BAD_REQUEST
        };
        (status, Json(json!({ "error": e })))
    })?;
    Ok(Json(json!({ "ok": true, "deleted": key })))
}

pub async fn enrollment_log(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({ "events": state.otp.list_log(200) }))
}

#[derive(Debug, serde::Deserialize)]
pub struct LogsQuery {
    /// `all` | `api` | `worker` | `access` (default: all)
    #[serde(default = "default_log_source")]
    pub source: String,
    /// Max lines to return (default 800, capped at 5000).
    #[serde(default = "default_log_lines")]
    pub lines: usize,
    /// Optional level filter: error|warn|info|debug|trace (case-insensitive).
    #[serde(default)]
    pub level: String,
    /// Substring filter applied server-side (case-insensitive).
    #[serde(default)]
    pub q: String,
}

fn default_log_source() -> String {
    "all".into()
}

fn default_log_lines() -> usize {
    800
}

#[derive(Debug, Serialize)]
pub struct LogLine {
    pub source: String,
    pub ts: Option<String>,
    pub level: Option<String>,
    pub target: Option<String>,
    pub message: String,
    pub raw: String,
}

pub async fn process_logs(
    State(state): State<AppState>,
    Query(q): Query<LogsQuery>,
) -> Json<serde_json::Value> {
    let limit = q.lines.clamp(1, 5000);
    let run_dir = state.data_dir.join("run");
    let source = q.source.to_lowercase();
    let level_filter = q.level.trim().to_ascii_uppercase();
    let text_filter = q.q.trim().to_ascii_lowercase();
    let fetch = if level_filter.is_empty() && text_filter.is_empty() {
        limit
    } else {
        (limit.saturating_mul(4)).clamp(limit, 5000)
    };
    let mut entries: Vec<LogLine> = Vec::new();

    let want_api = matches!(source.as_str(), "all" | "api" | "");
    let want_worker = matches!(source.as_str(), "all" | "worker" | "");
    let want_access = matches!(
        source.as_str(),
        "all" | "access" | "conn" | "connections" | ""
    );

    if want_api {
        let mut api_lines = read_log_file(&run_dir.join("api.log"), "api", fetch);
        // When connections are also pulled from SQLite, drop the duplicate
        // tracing lines emitted by access middleware into api.log.
        if want_access {
            api_lines.retain(|e| {
                !e.target
                    .as_deref()
                    .map(|t| t.ends_with("::access") || t == "access")
                    .unwrap_or(false)
            });
        }
        entries.extend(api_lines);
    }
    if want_worker {
        entries.extend(read_log_file(&run_dir.join("worker.log"), "worker", fetch));
    }
    if want_access {
        entries.extend(access_events_as_log_lines(&state, fetch));
    }

    // When mixing sources, keep chronological order by timestamp when present.
    let mixed = [want_api, want_worker, want_access]
        .into_iter()
        .filter(|v| *v)
        .count()
        > 1;
    if mixed {
        entries.sort_by(|a, b| match (&a.ts, &b.ts) {
            (Some(ta), Some(tb)) => ta.cmp(tb),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });
    }

    let mut counts = json!({
        "ERROR": 0u64, "WARN": 0u64, "INFO": 0u64, "DEBUG": 0u64, "TRACE": 0u64, "OTHER": 0u64
    });
    for e in &entries {
        let lvl = e.level.as_deref().unwrap_or("OTHER");
        let key = match lvl {
            "ERROR" | "WARN" | "INFO" | "DEBUG" | "TRACE" => lvl,
            _ => "OTHER",
        };
        if let Some(v) = counts.get_mut(key) {
            *v = json!(v.as_u64().unwrap_or(0) + 1);
        }
    }

    if !level_filter.is_empty() {
        entries.retain(|e| {
            e.level
                .as_deref()
                .map(|l| l.eq_ignore_ascii_case(&level_filter))
                .unwrap_or(false)
        });
    }
    if !text_filter.is_empty() {
        entries.retain(|e| {
            format!(
                "{} {} {} {} {}",
                e.ts.as_deref().unwrap_or(""),
                e.level.as_deref().unwrap_or(""),
                e.source,
                e.target.as_deref().unwrap_or(""),
                e.message
            )
            .to_ascii_lowercase()
            .contains(&text_filter)
        });
    }

    if entries.len() > limit {
        let skip = entries.len() - limit;
        entries = entries.into_iter().skip(skip).collect();
    }

    Json(json!({
        "lines": entries,
        "sources": {
            "api": run_dir.join("api.log").display().to_string(),
            "worker": run_dir.join("worker.log").display().to_string(),
            "access": "sqlite:access_events",
        },
        "count": entries.len(),
        "counts": counts,
    }))
}

fn access_events_as_log_lines(state: &AppState, limit: usize) -> Vec<LogLine> {
    state
        .access
        .list(limit, None, None, None)
        .into_iter()
        // list() returns newest-first; reverse so chronological merge works.
        .rev()
        .map(|e| {
            let level = if e.status >= 500 {
                "ERROR"
            } else if e.status >= 400 {
                "WARN"
            } else {
                "INFO"
            };
            let ts = unix_secs_to_iso(&e.ts);
            let mut parts = vec![
                format!("{} {}", e.method, e.path),
                format!("status={}", e.status),
                format!("{}ms", e.duration_ms),
                format!("remote={}", e.remote),
            ];
            if !e.kind.is_empty() {
                parts.push(format!("kind={}", e.kind));
            }
            if !e.resource_id.is_empty() {
                parts.push(format!("resource={}", e.resource_id));
            }
            if !e.device_id.is_empty() {
                parts.push(format!("device={}", e.device_id));
            }
            if !e.user_agent.is_empty() {
                let ua: String = e.user_agent.chars().take(80).collect();
                parts.push(format!("ua={ua}"));
            }
            let message = parts.join(" ");
            let raw = match &ts {
                Some(t) => format!("{t}  {level} access: {message}"),
                None => format!("{level} access: {message}"),
            };
            LogLine {
                source: "access".into(),
                ts,
                level: Some(level.into()),
                target: Some("connection".into()),
                message,
                raw,
            }
        })
        .collect()
}

fn unix_secs_to_iso(ts: &str) -> Option<String> {
    let secs: i64 = ts.parse().ok()?;
    let dt = time::OffsetDateTime::from_unix_timestamp(secs).ok()?;
    dt.format(&time::format_description::well_known::Rfc3339)
        .ok()
}

fn read_log_file(path: &Path, source: &str, limit: usize) -> Vec<LogLine> {
    let text = read_file_tail(path, 1024 * 1024).unwrap_or_default();
    if text.is_empty() {
        return Vec::new();
    }
    let stripped = strip_ansi(&text);
    let mut out: Vec<LogLine> = stripped
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| parse_log_line(source, line))
        .collect();
    if out.len() > limit {
        let skip = out.len() - limit;
        out = out.into_iter().skip(skip).collect();
    }
    out
}

fn read_file_tail(path: &Path, max_bytes: u64) -> std::io::Result<String> {
    let mut f = fs::File::open(path)?;
    let len = f.metadata()?.len();
    if len > max_bytes {
        f.seek(SeekFrom::Start(len - max_bytes))?;
    }
    let mut buf = String::new();
    f.read_to_string(&mut buf)?;
    // If we started mid-line, drop the partial first line.
    if len > max_bytes {
        if let Some(pos) = buf.find('\n') {
            buf = buf[pos + 1..].to_string();
        }
    }
    Ok(buf)
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next(); // '['
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        out.push(ch);
    }
    out
}

fn parse_log_line(source: &str, line: &str) -> LogLine {
    // tracing default: `2026-11-02T14:45:15.357931Z  INFO collectemoi_api: message …`
    let raw = line.to_string();
    let mut rest = line;
    let mut ts = None;
    let mut level = None;
    let mut target = None;

    if let Some((first, after)) = rest.split_once(' ') {
        if first.len() >= 20 && first.contains('T') && (first.ends_with('Z') || first.contains('+'))
        {
            ts = Some(first.to_string());
            rest = after.trim_start();
        }
    }

    if let Some((lvl, after)) = rest.split_once(' ') {
        let up = lvl.trim();
        if matches!(
            up,
            "TRACE"
                | "DEBUG"
                | "INFO"
                | "WARN"
                | "ERROR"
                | "trace"
                | "debug"
                | "info"
                | "warn"
                | "error"
        ) {
            level = Some(up.to_ascii_uppercase());
            rest = after.trim_start();
        }
    }

    // tracing uses `target: message` (colon + space). Module paths may contain `::`.
    if let Some((tgt, after)) = rest.split_once(": ") {
        let t = tgt.trim();
        if !t.is_empty() && !t.contains(' ') {
            target = Some(t.to_string());
            rest = after;
        }
    }

    LogLine {
        source: source.to_string(),
        ts,
        level,
        target,
        message: rest.to_string(),
        raw,
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct CreateTokenRequest {
    pub token: Option<String>,
    pub case_id: Option<Uuid>,
}

pub async fn create_token(
    State(state): State<AppState>,
    Json(req): Json<CreateTokenRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let token = req
        .token
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| Uuid::new_v4().simple().to_string()[..12].to_string());
    let case_id = req
        .case_id
        .unwrap_or_else(|| Uuid::parse_str("00000000-0000-4000-8000-000000000001").unwrap());
    state
        .enrollment
        .add_token(&token, case_id)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))))?;
    Ok(Json(json!({ "token": token, "case_id": case_id })))
}

pub async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

fn count_dirs(dir: &PathBuf) -> usize {
    fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .count()
        })
        .unwrap_or(0)
}
