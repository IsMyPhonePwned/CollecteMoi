use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use collectemoi_db::JobRow;
use collectemoi_envelope::{ArtefactKind as EnvKind, parse_and_verify};
use collectemoi_proto::{
    ArtefactKind, CompleteUploadRequest, CompleteUploadResponse, CreateUploadRequest,
    CreateUploadResponse, ErrorBody, UploadStatus,
};
use uuid::Uuid;

use crate::AppState;
use crate::store::UploadMeta;

type ApiError = (StatusCode, Json<ErrorBody>);

/// Canonical client-facing code for a banned device (app matches on this string).
const DEVICE_BANNED: &str = "device_banned";

fn err(status: StatusCode, msg: impl Into<String>) -> ApiError {
    (status, Json(ErrorBody { error: msg.into() }))
}

fn to_env_kind(k: ArtefactKind) -> EnvKind {
    match k {
        ArtefactKind::AndroidBugreport => EnvKind::AndroidBugreport,
        ArtefactKind::IosSysdiagnose => EnvKind::IosSysdiagnose,
        ArtefactKind::MessageReport => EnvKind::MessageReport,
    }
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

/// Reject create / chunk / complete when the device or its public key is banned.
/// Checks `x-device-id` and optional body/session public key before any disk work.
fn reject_if_banned(
    state: &AppState,
    headers: &HeaderMap,
    device_public_key: Option<&str>,
) -> Result<(), ApiError> {
    if let Some(device_id) = header_str(headers, "x-device-id") {
        if state.enrollment.is_device_id_banned(device_id) {
            return Err(err(StatusCode::FORBIDDEN, DEVICE_BANNED));
        }
    }
    if let Some(pk) = device_public_key {
        if state.enrollment.is_public_key_banned(pk) {
            return Err(err(StatusCode::FORBIDDEN, DEVICE_BANNED));
        }
    }
    Ok(())
}

pub async fn create_upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateUploadRequest>,
) -> Result<(StatusCode, Json<CreateUploadResponse>), ApiError> {
    reject_if_banned(&state, &headers, None)?;

    // Per-company report quota (per device), with optional per-device bypass.
    // Independent of per-IP chunk PUT budgets. Enforced only on the server.
    if let Some(device_id) = header_str(&headers, "x-device-id") {
        let (company_id, bypass) = state
            .enrollment
            .device_rate_limit_context(device_id)
            .unwrap_or_default();
        if !bypass {
            let limit = if company_id.is_empty() {
                state.rate_limiter.config().device_uploads_per_hour
            } else {
                state
                    .otp
                    .get_company(&company_id)
                    .map(|c| c.device_uploads_per_hour)
                    .unwrap_or_else(|| state.rate_limiter.config().device_uploads_per_hour)
            };
            if let Err(retry) = state.rate_limiter.check_device_upload(device_id, limit) {
                tracing::warn!(
                    device_id = %device_id,
                    company_id = %company_id,
                    limit,
                    retry_after = retry.as_secs().max(1),
                    "device upload rate limit exceeded"
                );
                return Err(err(StatusCode::TOO_MANY_REQUESTS, "device_rate_limited"));
            }
        }
    }

    if req.chunk_size == 0 || req.chunk_count == 0 {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "chunk_size and chunk_count must be > 0",
        ));
    }
    let expected_chunks = req.total_size.div_ceil(req.chunk_size).max(1) as u32;
    if req.chunk_count != expected_chunks {
        return Err(err(
            StatusCode::BAD_REQUEST,
            format!(
                "chunk_count {0} does not match total_size/chunk_size ({expected_chunks})",
                req.chunk_count
            ),
        ));
    }
    if req.ciphertext_sha256.len() != 64 || hex::decode(&req.ciphertext_sha256).is_err() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "ciphertext_sha256 must be 64 hex chars",
        ));
    }

    let upload_id = Uuid::new_v4();
    let meta = UploadMeta {
        upload_id,
        case_id: req.case_id,
        artefact: req.artefact,
        total_size: req.total_size,
        chunk_size: req.chunk_size,
        chunk_count: req.chunk_count,
        ciphertext_sha256: req.ciphertext_sha256.to_lowercase(),
        received: vec![false; req.chunk_count as usize],
        complete: false,
        created_at: chrono_like_now(),
        completed_at: String::new(),
    };
    state
        .store
        .create(meta)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tracing::info!(%upload_id, case = %req.case_id, "upload created");
    Ok((
        StatusCode::CREATED,
        Json(CreateUploadResponse { upload_id }),
    ))
}

pub async fn get_upload(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<UploadStatus>, ApiError> {
    let meta = state
        .store
        .get(id)
        .map_err(|_| err(StatusCode::NOT_FOUND, "upload not found"))?;
    Ok(Json(status_from_meta(meta)))
}

pub async fn put_chunk(
    State(state): State<AppState>,
    Path((id, n)): Path<(Uuid, u32)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    reject_if_banned(&state, &headers, None)?;

    let digest = headers
        .get("digest")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // Disk + SQLite work must not block the async runtime under many parallel uploads.
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || store.put_chunk(id, n, &body, digest.as_deref()))
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("not found") {
                err(StatusCode::NOT_FOUND, msg)
            } else if msg.contains("mismatch") || msg.contains("out of range") {
                err(StatusCode::BAD_REQUEST, msg)
            } else if msg.contains("complete") {
                err(StatusCode::CONFLICT, msg)
            } else {
                err(StatusCode::INTERNAL_SERVER_ERROR, msg)
            }
        })?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn complete_upload(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(req): Json<CompleteUploadRequest>,
) -> Result<Json<CompleteUploadResponse>, ApiError> {
    // Ban check *before* assemble — otherwise a banned phone can still mark the
    // upload complete on disk and leave a ciphertext envelope behind.
    reject_if_banned(&state, &headers, Some(req.device_public_key.as_str()))?;

    let (meta, path) = state.store.assemble(id).map_err(|e| {
        let msg = e.to_string();
        if msg.contains("missing") {
            err(StatusCode::CONFLICT, msg)
        } else if msg.contains("not found") {
            err(StatusCode::NOT_FOUND, msg)
        } else if msg.contains("mismatch") {
            err(StatusCode::BAD_REQUEST, msg)
        } else {
            err(StatusCode::INTERNAL_SERVER_ERROR, msg)
        }
    })?;

    // Re-check after assemble in case an admin banned mid-request.
    reject_if_banned(&state, &headers, Some(req.device_public_key.as_str()))?;

    let mut blob =
        std::fs::read(&path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Unwrap per-client ChaCha20 noise layer (CMNK) when present.
    if crate::noise::is_cmnk(&blob) {
        let noise_key = state
            .enrollment
            .noise_key_for_pubkey(&req.device_public_key)
            .ok_or_else(|| {
                err(
                    StatusCode::BAD_REQUEST,
                    "CMNK blob but no noise key registered for this device public key",
                )
            })?;
        blob =
            crate::noise::open(&blob, &noise_key).map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
        // Persist unwrapped CM01 for the worker.
        std::fs::write(&path, &blob)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        tracing::info!(upload = %id, "unwrapped CMNK noise layer");
    }

    if blob.starts_with(b"CMDEV") {
        if !state.settings.lock().unwrap().allow_dev_envelope {
            return Err(err(
                StatusCode::BAD_REQUEST,
                "dev envelope rejected (pass --allow-dev-envelope or enable in admin)",
            ));
        }
        tracing::warn!(upload = %id, "accepted CMDEV bring-up envelope");
    } else {
        let device_pk = hex::decode(req.device_public_key.trim())
            .map_err(|_| err(StatusCode::BAD_REQUEST, "device_public_key must be hex"))?;

        let (header, _ct) = parse_and_verify(&blob, &device_pk)
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("envelope invalid: {e}")))?;

        let expected = to_env_kind(meta.artefact);
        if header.artefact != expected {
            return Err(err(
                StatusCode::BAD_REQUEST,
                format!(
                    "artefact kind mismatch: envelope {:?}, upload {:?}",
                    header.artefact, meta.artefact
                ),
            ));
        }
    }

    // Queue decrypt→blob job in SQLite (authoritative) + legacy JSON drop for ops.
    let artefact = match meta.artefact {
        ArtefactKind::AndroidBugreport => "android_bugreport",
        ArtefactKind::IosSysdiagnose => "ios_sysdiagnose",
        ArtefactKind::MessageReport => "message_report",
    };
    let queued_at = chrono_like_now();
    let envelope_path = path.display().to_string();
    state
        .db
        .job_enqueue(&JobRow {
            upload_id: id.to_string(),
            case_id: meta.case_id.to_string(),
            artefact: artefact.into(),
            envelope_path: envelope_path.clone(),
            device_public_key: req.device_public_key.trim().to_string(),
            queued_at: queued_at.clone(),
            status: "pending".into(),
            claimed_at: None,
            finished_at: None,
            error: None,
        })
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let jobs_dir = state.data_dir.join("jobs");
    let _ = std::fs::create_dir_all(&jobs_dir);
    let job = serde_json::json!({
        "upload_id": id,
        "case_id": meta.case_id,
        "artefact": artefact,
        "envelope_path": envelope_path,
        "device_public_key": req.device_public_key.trim(),
        "queued_at": queued_at,
    });
    let job_path = jobs_dir.join(format!("{id}.json"));
    let job_tmp = jobs_dir.join(format!("{id}.json.tmp"));
    let _ = std::fs::write(
        &job_tmp,
        serde_json::to_vec_pretty(&job).unwrap_or_default(),
    );
    let _ = std::fs::rename(&job_tmp, &job_path);

    tracing::info!(
        upload = %id,
        artefact = ?meta.artefact,
        size = meta.total_size,
        "upload complete — queued for blob store"
    );

    Ok(Json(CompleteUploadResponse {
        upload_id: id,
        status: "queued".into(),
    }))
}

fn chrono_like_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

fn status_from_meta(meta: UploadMeta) -> UploadStatus {
    UploadStatus {
        upload_id: meta.upload_id,
        case_id: meta.case_id,
        artefact: meta.artefact,
        total_size: meta.total_size,
        chunk_size: meta.chunk_size,
        chunk_count: meta.chunk_count,
        received: meta.received,
        complete: meta.complete,
    }
}
