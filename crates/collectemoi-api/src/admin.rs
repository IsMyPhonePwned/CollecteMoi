//! Lab admin API + embedded console (gated by `auth` — password + TOTP / WebAuthn).

use std::collections::HashSet;
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

#[derive(Debug, Clone, Serialize)]
pub struct ActivityPoint {
    /// Bucket start (unix seconds).
    pub t: u64,
    pub enrollments: u64,
    pub submissions: u64,
    pub downloads: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActivitySeries {
    pub window_secs: u64,
    pub bucket_secs: u64,
    pub enrollments_total: u64,
    pub submissions_total: u64,
    pub downloads_total: u64,
    pub history: Vec<ActivityPoint>,
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
    pub bandwidth: crate::access::BandwidthStats,
    pub activity: ActivitySeries,
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
    let bandwidth = state.access.bandwidth();
    let activity = build_activity_series(state, now);

    Overview {
        ok: true,
        version: crate::version::displayed(),
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
        bandwidth,
        activity,
        case_public_key: state.enrollment.case_public_key_hex().to_string(),
        recent_enrollment,
    }
}

/// Last 24h of enrollments / submissions / downloads in hourly buckets.
fn build_activity_series(state: &AppState, now: u64) -> ActivitySeries {
    const WINDOW_SECS: u64 = 24 * 3600;
    const BUCKET_SECS: u64 = 3600;

    let since = now.saturating_sub(WINDOW_SECS);
    let bucket_start = |ts: u64| -> u64 { ts - (ts % BUCKET_SECS) };
    let first_bucket = bucket_start(since);
    let last_bucket = bucket_start(now);
    let mut buckets: std::collections::BTreeMap<u64, (u64, u64, u64)> =
        std::collections::BTreeMap::new();
    let mut t = first_bucket;
    while t <= last_bucket {
        buckets.insert(t, (0, 0, 0));
        t = t.saturating_add(BUCKET_SECS);
    }

    // Successful device enrollments.
    if let Ok(events) = state.db.enrollment_list(5_000) {
        for e in events {
            if e.ts < since || !e.event.eq_ignore_ascii_case("enroll_ok") {
                continue;
            }
            let b = bucket_start(e.ts);
            if let Some(slot) = buckets.get_mut(&b) {
                slot.0 = slot.0.saturating_add(1);
            }
        }
    }

    // Artefact submissions + analyst downloads.
    if let Ok(rows) = state.db.access_since(&since.to_string(), 10_000) {
        for row in rows {
            let ts = row.ts.parse::<u64>().unwrap_or(0);
            if ts < since {
                continue;
            }
            let b = bucket_start(ts);
            let Some(slot) = buckets.get_mut(&b) else {
                continue;
            };
            match row.kind.as_str() {
                "upload_complete" if row.status < 400 => {
                    slot.1 = slot.1.saturating_add(1);
                }
                "blob_download" if row.status < 400 => {
                    slot.2 = slot.2.saturating_add(1);
                }
                _ => {}
            }
        }
    }

    let history: Vec<ActivityPoint> = buckets
        .into_iter()
        .map(|(t, (enrollments, submissions, downloads))| ActivityPoint {
            t,
            enrollments,
            submissions,
            downloads,
        })
        .collect();
    let enrollments_total = history.iter().map(|p| p.enrollments).sum();
    let submissions_total = history.iter().map(|p| p.submissions).sum();
    let downloads_total = history.iter().map(|p| p.downloads).sum();

    ActivitySeries {
        window_secs: WINDOW_SECS,
        bucket_secs: BUCKET_SECS,
        enrollments_total,
        submissions_total,
        downloads_total,
        history,
    }
}

pub async fn overview(State(state): State<AppState>) -> Json<Overview> {
    Json(build_overview(&state))
}

/// Rich process status (same payload as overview) for ctl / ops.
pub async fn status(State(state): State<AppState>) -> Json<Overview> {
    Json(build_overview(&state))
}

#[derive(Debug, Serialize)]
pub struct PublicStatus {
    pub ok: bool,
    pub version: String,
    pub started_at: u64,
    pub uptime_secs: u64,
    pub settings: PublicStatusSettings,
    pub devices: usize,
    pub uploads: usize,
    pub uploads_complete: usize,
    pub jobs_pending: usize,
    pub blobs: usize,
}

#[derive(Debug, Serialize)]
pub struct PublicStatusSettings {
    pub display_name: String,
}

pub async fn public_status(State(state): State<AppState>) -> Json<PublicStatus> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let display_name = state.settings.lock().unwrap().display_name.clone();
    let devices = state.enrollment.list_devices().len();
    let (uploads, uploads_complete) = state.store.counts();
    Json(PublicStatus {
        ok: true,
        version: crate::version::displayed(),
        started_at: state.started_at,
        uptime_secs: now.saturating_sub(state.started_at),
        settings: PublicStatusSettings { display_name },
        devices,
        uploads,
        uploads_complete,
        jobs_pending: state.db.job_count_pending().unwrap_or(0),
        blobs: state.blobs.count(),
    })
}

pub async fn get_settings(State(state): State<AppState>) -> Json<crate::settings::RuntimeSettings> {
    Json(state.settings.lock().unwrap().clone())
}

/// Data directory path + on-disk usage (Settings → Storage).
pub async fn storage(State(state): State<AppState>) -> Json<crate::storage::StorageStats> {
    let data_dir = state.data_dir.clone();
    let stats = tokio::task::spawn_blocking(move || crate::storage::collect(&data_dir))
        .await
        .unwrap_or_else(|_| crate::storage::StorageStats {
            data_dir: String::new(),
            data_bytes: 0,
            breakdown: Vec::new(),
            disk_total_bytes: None,
            disk_used_bytes: None,
            disk_free_bytes: None,
        });
    Json(stats)
}

/// Selectively delete lab data under the data directory (password + `DELETE` confirm).
/// Never removes admin auth, TLS certs, settings, companies, lab keys, or case keys.
pub async fn wipe_storage(
    State(state): State<AppState>,
    Json(req): Json<crate::storage::WipeRequest>,
) -> Result<Json<crate::storage::WipeReport>, (StatusCode, Json<serde_json::Value>)> {
    if req.confirm.trim() != "DELETE" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "type DELETE to confirm" })),
        ));
    }
    if !req.any_selected() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "select at least one data category" })),
        ));
    }
    crate::auth::require_admin_password(&state, &req.password)?;

    let data_dir = state.data_dir.clone();
    let bytes_before = tokio::task::spawn_blocking({
        let d = data_dir.clone();
        move || crate::storage::collect(&d).data_bytes
    })
    .await
    .unwrap_or(0);

    let mut cleared = Vec::new();
    let mut details = json!({});

    if req.collected {
        let (blobs, uploads, jobs) = state.db.purge_collected().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("database purge failed: {e}") })),
            )
        })?;
        let dirs = [
            ("blobs", data_dir.join("blobs")),
            ("uploads", data_dir.join("uploads")),
            ("analysis", data_dir.join("analysis")),
            ("jobs", data_dir.join("jobs")),
        ];
        for (name, path) in dirs {
            let _ = crate::storage::reset_dir(&path);
            cleared.push(name.to_string());
        }
        details["collected"] = json!({ "blobs": blobs, "uploads": uploads, "jobs": jobs });
        cleared.push("collected_db".into());
    }

    if req.logs {
        let access_n = state.db.purge_access_events().unwrap_or(0);
        let enroll_n = state.db.purge_enrollment_events().unwrap_or(0);
        state.access.reset_after_purge();
        let _ = crate::storage::remove_file_if_exists(&data_dir.join("enrollment_log.jsonl"));
        let _ = crate::storage::remove_file_if_exists(&data_dir.join("access_log.jsonl"));
        details["logs"] = json!({ "access_events": access_n, "enrollment_events": enroll_n });
        cleared.push("logs".into());
    }

    if req.mobile_builds {
        let _ = crate::storage::reset_dir(&data_dir.join("mobile"));
        cleared.push("mobile_builds".into());
    }

    if req.build_caches {
        let _ = crate::storage::reset_dir(&data_dir.join(".pub-cache"));
        let _ = crate::storage::reset_dir(&data_dir.join(".gradle"));
        cleared.push("build_caches".into());
    }

    if req.devices {
        let n = state.db.purge_devices().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("device purge failed: {e}") })),
            )
        })?;
        let _ = crate::storage::reset_dir(&data_dir.join("devices"));
        let _ = state.enrollment.reload_from_db();
        details["devices"] = json!({ "rows": n });
        cleared.push("devices".into());
    }

    let bytes_after = tokio::task::spawn_blocking({
        let d = data_dir.clone();
        move || crate::storage::collect(&d).data_bytes
    })
    .await
    .unwrap_or(0);

    tracing::warn!(
        target: "collectemoi_api::storage",
        cleared = ?cleared,
        bytes_before,
        bytes_after,
        "admin wiped data from settings"
    );

    Ok(Json(crate::storage::WipeReport {
        ok: true,
        cleared,
        bytes_before,
        bytes_after,
        details,
    }))
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
    let pk_short = short_key(&rec.public_key);
    state.otp.audit(
        "device_ban",
        None,
        None,
        None,
        None,
        None,
        Some(&format!(
            "pubkey={pk_short} reason={}",
            if req.reason.trim().is_empty() {
                "(none)"
            } else {
                req.reason.trim()
            }
        )),
    );
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
    state.otp.audit(
        "device_unban",
        None,
        None,
        None,
        None,
        None,
        Some(&format!("pubkey={}", short_key(&req.public_key))),
    );
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
    state.otp.audit(
        if req.banned {
            "device_ban"
        } else {
            "device_unban"
        },
        Some(view.email.as_str()).filter(|s| !s.is_empty()),
        Some(view.company_id.as_str()).filter(|s| !s.is_empty()),
        None,
        Some(&device_id.to_string()),
        None,
        Some(if req.banned {
            "banned_via_device"
        } else {
            "unbanned_via_device"
        }),
    );
    Ok(Json(json!({ "device": view })))
}

#[derive(Debug, serde::Deserialize)]
pub struct DeviceRateLimitBypassRequest {
    pub bypass: bool,
}

pub async fn set_device_rate_limit_bypass(
    State(state): State<AppState>,
    axum::extract::Path(device_id): axum::extract::Path<Uuid>,
    Json(req): Json<DeviceRateLimitBypassRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let view = state
        .enrollment
        .set_device_rate_limit_bypass(device_id, req.bypass)
        .map_err(|e| {
            let status = if e.contains("not found") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::BAD_REQUEST
            };
            (status, Json(json!({ "error": e })))
        })?;
    state.otp.audit(
        if req.bypass {
            "device_rate_limit_bypass_on"
        } else {
            "device_rate_limit_bypass_off"
        },
        Some(view.email.as_str()).filter(|s| !s.is_empty()),
        Some(view.company_id.as_str()).filter(|s| !s.is_empty()),
        None,
        Some(&device_id.to_string()),
        None,
        Some(if req.bypass {
            "bypass_company_upload_quota"
        } else {
            "enforce_company_upload_quota"
        }),
    );
    Ok(Json(json!({ "device": view })))
}

fn short_key(pk: &str) -> String {
    let t = pk.trim();
    if t.len() <= 12 {
        return t.to_string();
    }
    format!("{}…{}", &t[..6], &t[t.len() - 4..])
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

pub async fn delete_upload(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    match state.store.delete_incomplete(id) {
        Ok(()) => Ok(Json(json!({ "ok": true, "upload_id": id }))),
        Err(e) => {
            let msg = e.to_string();
            let status = if msg.contains("not found") {
                StatusCode::NOT_FOUND
            } else if msg.contains("complete") {
                StatusCode::CONFLICT
            } else {
                StatusCode::BAD_REQUEST
            };
            Err((status, Json(json!({ "error": msg }))))
        }
    }
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
            let builds = list_mobile_builds_for_company(&state.data_dir, &c.id);
            let last_build_at = builds
                .iter()
                .filter_map(|b| {
                    b.get("generated_at")
                        .or_else(|| b.get("modified_at"))
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                })
                .max();
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
                "build_count": builds.len(),
                "last_build_at": last_build_at,
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
    let enroll_events: Vec<_> = state
        .otp
        .list_log(200)
        .into_iter()
        .filter(|e| e.company_id.as_deref() == Some(company.id.as_str()))
        .take(50)
        .collect();

    // Upload / blob activity for this company's devices (and blobs under its case).
    let device_ids: HashSet<String> = devices
        .iter()
        .map(|d| d.device_id.to_string())
        .collect();
    let device_by_id: std::collections::HashMap<String, &crate::enroll::DevicePublicView> = devices
        .iter()
        .map(|d| (d.device_id.to_string(), d))
        .collect();
    let case_id = company.case_id.to_string();
    let (case_blobs, _) = state.blobs.list(&crate::blobs::ListBlobsQuery {
        limit: 300,
        offset: 0,
        case_id: Some(case_id.clone()),
        artefact: None,
    });
    let company_blob_ids: HashSet<String> = case_blobs
        .iter()
        .map(|b| b.blob_id.clone())
        .collect();
    let (case_uploads, _) = state.store.list_page(300, 0).unwrap_or_default();
    let company_uploads: Vec<_> = case_uploads
        .into_iter()
        .filter(|u| u.case_id == company.case_id)
        .collect();

    let upload_access: Vec<_> = state
        .access
        .list(1500, None, None, None)
        .into_iter()
        .filter(|e| {
            // Skip chunk/get noise — create/complete + blob ops are the audit signal.
            let keep = matches!(
                e.kind.as_str(),
                "upload_create"
                    | "upload_complete"
                    | "blob_download"
                    | "blob_meta"
                    | "blob_delete"
                    | "blob_list"
            );
            if !keep {
                return false;
            }
            if !e.device_id.is_empty() && device_ids.contains(&e.device_id) {
                return true;
            }
            // Admin / anonymous blob ops still count when the artefact belongs here.
            if e.kind.starts_with("blob_")
                && !e.resource_id.is_empty()
                && company_blob_ids.contains(&e.resource_id)
            {
                return true;
            }
            false
        })
        .take(80)
        .collect();

    // Unified audit timeline: enrollment + uploads/blobs (newest first).
    let mut audit: Vec<serde_json::Value> = Vec::new();
    for e in &enroll_events {
        audit.push(json!({
            "source": "enrollment",
            "ts": e.ts,
            "event": e.event,
            "email": e.email,
            "device_id": e.device_id,
            "lab_key": e.lab_key,
            "detail": e.detail,
            "remote": serde_json::Value::Null,
            "status": serde_json::Value::Null,
            "resource_id": serde_json::Value::Null,
        }));
    }
    for e in &upload_access {
        let email = device_by_id
            .get(&e.device_id)
            .map(|d| d.email.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("");
        let detail = {
            let mut bits = Vec::new();
            if !e.resource_id.is_empty() && e.resource_id != "_list_" {
                bits.push(format!("id={}", e.resource_id));
            }
            if e.is_admin && !e.admin_user.is_empty() {
                bits.push(format!("admin={}", e.admin_user));
            }
            if !e.model.is_empty() || !e.platform.is_empty() {
                bits.push(format!(
                    "{}{}",
                    if e.platform.is_empty() {
                        String::new()
                    } else {
                        format!("{} ", e.platform)
                    },
                    e.model
                ));
            }
            if e.status != 0 {
                bits.push(format!("HTTP {}", e.status));
            }
            bits.push(format!("{}ms", e.duration_ms));
            bits.join(" · ")
        };
        audit.push(json!({
            "source": "access",
            "ts": e.ts,
            "event": e.kind,
            "email": if email.is_empty() { serde_json::Value::Null } else { json!(email) },
            "device_id": if e.device_id.is_empty() { serde_json::Value::Null } else { json!(e.device_id) },
            "lab_key": serde_json::Value::Null,
            "detail": detail,
            "remote": e.remote,
            "status": e.status,
            "resource_id": e.resource_id,
            "path": e.path,
            "method": e.method,
        }));
    }
    // Ensure completed uploads appear even if access rows were pruned.
    for u in &company_uploads {
        if !u.complete {
            continue;
        }
        let ts = if !u.completed_at.is_empty() {
            &u.completed_at
        } else {
            &u.created_at
        };
        if ts.is_empty() {
            continue;
        }
        let already = upload_access.iter().any(|e| {
            e.kind == "upload_complete" && e.resource_id == u.upload_id.to_string()
        });
        if already {
            continue;
        }
        let artefact = serde_json::to_value(u.artefact)
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| format!("{:?}", u.artefact));
        audit.push(json!({
            "source": "upload",
            "ts": ts,
            "event": "upload_complete",
            "email": serde_json::Value::Null,
            "device_id": serde_json::Value::Null,
            "lab_key": serde_json::Value::Null,
            "detail": format!(
                "{artefact} · {} bytes · upload={}",
                u.total_size,
                u.upload_id
            ),
            "remote": serde_json::Value::Null,
            "status": 200,
            "resource_id": u.upload_id.to_string(),
        }));
    }
    audit.sort_by(|a, b| {
        let ta = a.get("ts").and_then(|v| v.as_u64().or_else(|| {
            v.as_str().and_then(|s| s.parse::<u64>().ok())
        })).unwrap_or(0);
        let tb = b.get("ts").and_then(|v| v.as_u64().or_else(|| {
            v.as_str().and_then(|s| s.parse::<u64>().ok())
        })).unwrap_or(0);
        tb.cmp(&ta)
    });
    audit.truncate(100);

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
        "enrollment_events": enroll_events,
        "upload_events": upload_access,
        "uploads": company_uploads,
        "blobs": case_blobs,
        "audit": audit,
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
            // If parent is the company folder itself, branding version is empty.
            let branding_version = if version_dir == company_id {
                String::new()
            } else {
                version_dir
            };
            let mtime_secs = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let generated_at = unix_secs_to_rfc3339(mtime_secs);
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let sidecar = read_build_sidecar(&p, size, mtime_secs);
            let app_version = sidecar
                .as_ref()
                .and_then(|s| s.get("version"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let sha256 = sidecar
                .as_ref()
                .and_then(|s| s.get("sha256"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            out.push(json!({
                "name": name,
                "path": p.strip_prefix(data_dir).map(|r| r.display().to_string()).unwrap_or_else(|_| p.display().to_string()),
                "company_id": company_id,
                "branding_version": branding_version,
                // App version from .meta.json when present (Flutter --build-name).
                "version": app_version,
                "sha256": sha256,
                "size": size,
                "generated_at": generated_at,
                "modified_at": generated_at,
                "platform": if lower.ends_with(".apk") { "android" } else { "ios" },
            }));
        }
    }
    out.sort_by(|a, b| {
        let ta = a
            .get("generated_at")
            .or_else(|| a.get("modified_at"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let tb = b
            .get("generated_at")
            .or_else(|| b.get("modified_at"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        tb.cmp(ta).then_with(|| {
            a.get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .cmp(b.get("name").and_then(|v| v.as_str()).unwrap_or(""))
        })
    });
    out
}

fn unix_secs_to_rfc3339(secs: u64) -> String {
    if secs == 0 {
        return String::new();
    }
    time::OffsetDateTime::from_unix_timestamp(secs as i64)
        .ok()
        .and_then(|odt| {
            odt.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .unwrap_or_default()
}

fn read_build_sidecar(path: &Path, size: u64, mtime_secs: u64) -> Option<serde_json::Value> {
    let mut meta_path = path.as_os_str().to_owned();
    meta_path.push(".meta.json");
    let raw = fs::read_to_string(PathBuf::from(meta_path)).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    if value.get("size").and_then(|v| v.as_u64()) != Some(size) {
        return None;
    }
    if let Some(stored) = value.get("mtime_secs").and_then(|v| v.as_u64()) {
        if stored != 0 && stored != mtime_secs {
            return None;
        }
    }
    Some(value)
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
    // Resolve enrolled devices so log lines can show model / email next to IP
    // even when a given event only carried `x-device-id`.
    let known: std::collections::HashMap<String, crate::enroll::DevicePublicView> = state
        .enrollment
        .list_devices()
        .into_iter()
        .map(|d| (d.device_id.to_string(), d))
        .collect();

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
            let known_dev = if e.device_id.is_empty() {
                None
            } else {
                known.get(&e.device_id)
            };
            let platform = if !e.platform.is_empty() {
                e.platform.as_str()
            } else {
                known_dev.map(|d| d.platform.as_str()).unwrap_or("")
            };
            let model = if !e.model.is_empty() {
                e.model.as_str()
            } else {
                known_dev.map(|d| d.model.as_str()).unwrap_or("")
            };
            let os_version = if !e.os_version.is_empty() {
                e.os_version.as_str()
            } else {
                known_dev.map(|d| d.os_version.as_str()).unwrap_or("")
            };
            let app_version = if !e.app_version.is_empty() {
                e.app_version.as_str()
            } else {
                known_dev.map(|d| d.app_version.as_str()).unwrap_or("")
            };
            // Lead with remote IP so Logs / Connections make the client address obvious.
            let mut parts = vec![
                format!("remote={}", if e.remote.is_empty() { "-" } else { &e.remote }),
                format!("{} {}", e.method, e.path),
                format!("status={}", e.status),
                format!("{}ms", e.duration_ms),
            ];
            if !e.kind.is_empty() {
                parts.push(format!("kind={}", e.kind));
            }
            if !e.resource_id.is_empty() {
                parts.push(format!("resource={}", e.resource_id));
            }
            if !e.device_id.is_empty() {
                parts.push(format!("device={}", e.device_id));
                if let Some(d) = known_dev {
                    let mut who = Vec::new();
                    if !d.device_name.is_empty() {
                        who.push(d.device_name.as_str());
                    } else if !model.is_empty() {
                        who.push(model);
                    }
                    if !d.email.is_empty() {
                        who.push(d.email.as_str());
                    } else if !platform.is_empty() && (d.device_name.is_empty() || model.is_empty())
                    {
                        who.push(platform);
                    }
                    if !who.is_empty() {
                        // Spaces → underscores so the Logs UI can parse who= as one token.
                        let label = who.join("/").replace(' ', "_");
                        parts.push(format!("who={label}"));
                    }
                }
            }
            if !app_version.is_empty() {
                parts.push(format!("app={app_version}"));
            }
            if !platform.is_empty() || !model.is_empty() {
                parts.push(format!(
                    "device_info={}/{}",
                    if platform.is_empty() { "-" } else { platform },
                    if model.is_empty() { "-" } else { model }
                ));
            }
            if !os_version.is_empty() {
                parts.push(format!("os={os_version}"));
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
