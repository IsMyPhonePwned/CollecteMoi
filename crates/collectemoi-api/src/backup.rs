//! Admin export / import of SQLite catalogue (+ optional keys & blob bytes).

use axum::{
    Json,
    body::Bytes,
    extract::{Multipart, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use collectemoi_db::{ExportOptions, ImportOptions, export_zip, import_zip};
use serde::Deserialize;
use serde_json::json;

use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct ExportQuery {
    #[serde(default)]
    pub keys: Option<bool>,
    #[serde(default)]
    pub blobs: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ImportQuery {
    /// Must be true to apply the import (safety latch).
    #[serde(default)]
    pub confirm: bool,
    #[serde(default)]
    pub keys: Option<bool>,
    #[serde(default)]
    pub blobs: Option<bool>,
}

fn flag(v: Option<bool>) -> bool {
    v.unwrap_or(false)
}

pub async fn export_backup(
    State(state): State<AppState>,
    Query(q): Query<ExportQuery>,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    let include_keys = flag(q.keys);
    let include_blobs = flag(q.blobs);
    let settings_json = serde_json::to_string_pretty(&*state.settings.lock().unwrap())
        .unwrap_or_else(|_| "{}".into());

    let opts = ExportOptions {
        include_keys,
        include_blobs,
        settings_json: Some(settings_json),
    };

    let data_dir = state.data_dir.clone();
    let db = state.db.clone();
    let bytes = tokio::task::spawn_blocking(move || export_zip(&data_dir, &db, &opts))
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
        })?
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
        })?;

    let filename = format!(
        "collectemoi-backup-{}.zip",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    );
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/zip"),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"{filename}\"")
            .parse()
            .unwrap_or(header::HeaderValue::from_static(
                "attachment; filename=\"collectemoi-backup.zip\"",
            )),
    );
    headers.insert(header::CONTENT_LENGTH, bytes.len().into());

    Ok((StatusCode::OK, headers, bytes).into_response())
}

pub async fn import_backup(
    State(state): State<AppState>,
    Query(q): Query<ImportQuery>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    if !q.confirm {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "pass confirm=true to apply import (this replaces the SQLite catalogue)"
            })),
        ));
    }

    let mut zip_bytes: Option<Bytes> = None;
    while let Some(field) = multipart.next_field().await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": e.to_string() })),
        )
    })? {
        let name = field.name().unwrap_or("").to_string();
        if name == "file" || name == "backup" || name.is_empty() {
            let data = field.bytes().await.map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": e.to_string() })),
                )
            })?;
            if !data.is_empty() {
                zip_bytes = Some(data);
                break;
            }
        }
    }

    let zip_bytes = zip_bytes.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "multipart field 'file' (zip) required" })),
        )
    })?;

    if zip_bytes.len() > 512 * 1024 * 1024 {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(json!({ "error": "backup too large (max 512 MiB)" })),
        ));
    }

    let opts = ImportOptions {
        include_keys: flag(q.keys),
        include_blobs: flag(q.blobs),
        replace_db: true,
    };

    let data_dir = state.data_dir.clone();
    let db = state.db.clone();
    let report = tokio::task::spawn_blocking(move || import_zip(&data_dir, &db, &zip_bytes, &opts))
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
        })?
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": e.to_string() })),
            )
        })?;

    // Reload in-memory caches from SQLite.
    let _ = state.enrollment.reload_from_db();
    let _ = state.otp.reload_from_db();

    if let Some(ref settings_json) = report.settings_json {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(settings_json) {
            let mut s = state.settings.lock().unwrap();
            if let Some(name) = v.get("display_name").and_then(|x| x.as_str()) {
                s.display_name = name.to_string();
                state.enrollment.set_display_name(name);
            }
            if let Some(url) = v.get("public_url").and_then(|x| x.as_str()) {
                s.public_url = url.trim_end_matches('/').to_string();
                state.enrollment.set_public_url(url);
            }
            if let Some(b) = v.get("lab_seal").and_then(|x| x.as_bool()) {
                s.lab_seal = b;
            }
            if let Some(b) = v.get("allow_dev_envelope").and_then(|x| x.as_bool()) {
                s.allow_dev_envelope = b;
            }
            if let Some(n) = v
                .get("healthz_interval_secs")
                .and_then(|x| x.as_u64())
            {
                s.healthz_interval_secs =
                    crate::settings::clamp_healthz_interval_secs(n);
            }
            if let Err(e) = crate::settings::save_persisted(&state.db, &s) {
                tracing::warn!(error = %e, "failed to persist imported settings");
            }
        }
    }

    Ok(Json(json!({
        "ok": true,
        "restored_db": report.restored_db,
        "restored_keys": report.restored_keys,
        "restored_blobs": report.restored_blobs,
        "has_settings": report.has_settings,
        "manifest": report.manifest,
        "note": "Catalogue restored. Restart api/worker if blob files or keys were included and workers look stale.",
    })))
}
