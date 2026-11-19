//! Opaque blob catalogue — decrypted artefacts stored by the worker.
//!
//! Layout (sharded for large trees):
//!   <data-dir>/blobs/<aa>/<bb>/<blob_id>/meta.json
//!   <data-dir>/blobs/<aa>/<bb>/<blob_id>/blob.bin
//!   <data-dir>/blobs/index.jsonl

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::{
    Json,
    body::Body,
    extract::{Path as AxumPath, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use collectemoi_db::{BlobRow, Db, parse_uuid_id, safe_filename};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobMeta {
    pub blob_id: String,
    pub upload_id: String,
    #[serde(default)]
    pub case_id: String,
    pub artefact: String,
    pub size: u64,
    pub sha256: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub queued_at: String,
    #[serde(default)]
    pub device_public_key: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default = "default_ct")]
    pub content_type: String,
}

fn default_ct() -> String {
    "application/octet-stream".into()
}

#[derive(Debug, Deserialize)]
pub struct ListBlobsQuery {
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
    pub case_id: Option<String>,
    pub artefact: Option<String>,
}

fn default_limit() -> usize {
    100
}

pub struct BlobStore {
    root: PathBuf,
    db: Arc<Db>,
}

impl BlobStore {
    pub fn open(data_dir: &Path, db: Arc<Db>) -> anyhow::Result<Self> {
        let root = data_dir.join("blobs");
        fs::create_dir_all(&root)?;
        Ok(Self { root, db })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn shard_dir(&self, id: &Uuid) -> PathBuf {
        let s = id.to_string();
        self.root.join(&s[0..2]).join(&s[2..4]).join(&s)
    }

    /// Resolve blob directory only for validated UUID ids (blocks path traversal).
    pub fn blob_dir(&self, id: &str) -> Option<PathBuf> {
        let uuid = parse_uuid_id(id).ok()?;
        let sharded = self.shard_dir(&uuid);
        if sharded.join("meta.json").is_file() || sharded.join("blob.bin").is_file() {
            return Some(sharded);
        }
        let flat = self.root.join(uuid.to_string());
        if flat.join("meta.json").is_file() || flat.join("blob.bin").is_file() {
            return Some(flat);
        }
        // Catalogue may exist before bytes are checked.
        if self.db.blob_get(&uuid.to_string()).ok().flatten().is_some() {
            return Some(sharded);
        }
        None
    }

    pub fn get_meta(&self, id: &str) -> Option<BlobMeta> {
        let uuid = parse_uuid_id(id).ok()?;
        if let Ok(Some(row)) = self.db.blob_get(&uuid.to_string()) {
            return Some(row_to_meta(row));
        }
        let dir = self.blob_dir(&uuid.to_string())?;
        let bytes = fs::read(dir.join("meta.json")).ok()?;
        let meta: BlobMeta = serde_json::from_slice(&bytes).ok()?;
        let _ = self.db.blob_upsert(&meta_to_row(&meta));
        Some(meta)
    }

    pub fn content_path(&self, id: &str) -> Option<PathBuf> {
        let dir = self.blob_dir(id)?;
        let p = dir.join("blob.bin");
        if p.is_file() { Some(p) } else { None }
    }

    pub fn delete(&self, id: &str) -> bool {
        let Ok(uuid) = parse_uuid_id(id) else {
            return false;
        };
        let id = uuid.to_string();
        let removed_db = self.db.blob_delete(&id).unwrap_or(false);
        if let Some(dir) = self.blob_dir(&id) {
            let _ = fs::remove_dir_all(&dir);
            return true;
        }
        removed_db
    }

    /// Newest-first listing with optional filters (parameterized SQL).
    pub fn list(&self, q: &ListBlobsQuery) -> (Vec<BlobMeta>, usize) {
        match self.db.blob_list(
            q.limit,
            q.offset,
            q.case_id.as_deref(),
            q.artefact.as_deref(),
        ) {
            Ok((rows, total)) => (rows.into_iter().map(row_to_meta).collect(), total),
            Err(_) => (Vec::new(), 0),
        }
    }

    pub fn count(&self) -> usize {
        self.db.blob_count().unwrap_or(0)
    }
}

fn meta_to_row(m: &BlobMeta) -> BlobRow {
    BlobRow {
        blob_id: m.blob_id.clone(),
        upload_id: m.upload_id.clone(),
        case_id: m.case_id.clone(),
        artefact: m.artefact.clone(),
        size: m.size,
        sha256: m.sha256.clone(),
        created_at: m.created_at.clone(),
        queued_at: m.queued_at.clone(),
        device_public_key: m.device_public_key.clone(),
        filename: m.filename.clone(),
        content_type: if m.content_type.is_empty() {
            default_ct()
        } else {
            m.content_type.clone()
        },
    }
}

fn row_to_meta(r: BlobRow) -> BlobMeta {
    BlobMeta {
        blob_id: r.blob_id,
        upload_id: r.upload_id,
        case_id: r.case_id,
        artefact: r.artefact,
        size: r.size,
        sha256: r.sha256,
        created_at: r.created_at,
        queued_at: r.queued_at,
        device_public_key: r.device_public_key,
        filename: r.filename,
        content_type: r.content_type,
    }
}

#[derive(Debug, Serialize)]
pub struct BlobUploader {
    pub device_id: Option<String>,
    pub email: String,
    pub company_id: String,
    pub platform: String,
    pub model: String,
    pub public_key: String,
    pub enrolled_at: String,
}

#[derive(Debug, Serialize)]
pub struct BlobView {
    #[serde(flatten)]
    pub meta: BlobMeta,
    pub uploader: Option<BlobUploader>,
    pub download_count: u64,
    pub last_download_at: String,
    pub last_download_remote: String,
    pub last_download_device_id: String,
    /// Recent blob access events (downloads first, then meta/delete), newest first.
    pub access: Vec<crate::access::AccessEvent>,
}

fn resolve_uploader(state: &AppState, device_public_key: &str) -> Option<BlobUploader> {
    if device_public_key.is_empty() {
        return None;
    }
    state
        .enrollment
        .find_device_by_pubkey(device_public_key)
        .map(|d| BlobUploader {
            device_id: Some(d.device_id.to_string()),
            email: d.email,
            company_id: d.company_id,
            platform: d.platform,
            model: d.model,
            public_key: d.public_key,
            enrolled_at: d.enrolled_at,
        })
        .or_else(|| {
            Some(BlobUploader {
                device_id: None,
                email: String::new(),
                company_id: String::new(),
                platform: String::new(),
                model: String::new(),
                public_key: device_public_key.to_string(),
                enrolled_at: String::new(),
            })
        })
}

fn enrich_blob(
    state: &AppState,
    meta: BlobMeta,
    history: Vec<crate::access::AccessEvent>,
) -> BlobView {
    let uploader = resolve_uploader(state, &meta.device_public_key);
    let download_events: Vec<_> = history
        .iter()
        .filter(|e| e.kind == "blob_download")
        .cloned()
        .collect();
    let download_count = download_events.len() as u64;
    let (last_download_at, last_download_remote, last_download_device_id) = download_events
        .first()
        .map(|e| (e.ts.clone(), e.remote.clone(), e.device_id.clone()))
        .unwrap_or_default();

    BlobView {
        meta,
        uploader,
        download_count,
        last_download_at,
        last_download_remote,
        last_download_device_id,
        access: history,
    }
}

pub async fn list_blobs(
    State(state): State<AppState>,
    Query(q): Query<ListBlobsQuery>,
) -> Json<serde_json::Value> {
    let (blobs, total) = state.blobs.list(&q);
    let ids: Vec<_> = blobs.iter().map(|b| b.blob_id.clone()).collect();
    let mut histories = state.access.blob_histories(&ids, 40);
    let views: Vec<_> = blobs
        .into_iter()
        .map(|m| {
            let hist = histories.remove(&m.blob_id).unwrap_or_default();
            enrich_blob(&state, m, hist)
        })
        .collect();
    Json(json!({
        "blobs": views,
        "total": total,
        "limit": q.limit.clamp(1, 1000),
        "offset": q.offset,
    }))
}

pub async fn get_blob(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<BlobView>, (StatusCode, Json<serde_json::Value>)> {
    let id = parse_uuid_id(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "invalid blob id" })),
        )
    })?;
    let meta = state.blobs.get_meta(&id.to_string()).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "blob not found" })),
        )
    })?;
    let hist = state.access.blob_history(&meta.blob_id, 50);
    Ok(Json(enrich_blob(&state, meta, hist)))
}

pub async fn get_blob_content(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    let id = parse_uuid_id(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "invalid blob id" })),
        )
    })?;
    let id = id.to_string();
    let meta = state.blobs.get_meta(&id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "blob not found" })),
        )
    })?;
    let path = state.blobs.content_path(&id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "blob content missing" })),
        )
    })?;

    let file = tokio::fs::File::open(&path).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
    })?;
    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);

    let raw_name = if meta.filename.is_empty() {
        format!("{id}.bin")
    } else {
        meta.filename.clone()
    };
    let filename = safe_filename(&raw_name);
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        meta.content_type
            .parse()
            .unwrap_or(header::HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"{filename}\"")
            .parse()
            .unwrap_or(header::HeaderValue::from_static("attachment")),
    );
    if meta.size > 0 {
        headers.insert(header::CONTENT_LENGTH, meta.size.into());
    }

    Ok((StatusCode::OK, headers, body).into_response())
}

pub async fn delete_blob(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let id = id.to_string();
    if state.blobs.delete(&id) {
        Ok(Json(json!({ "ok": true, "blob_id": id })))
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "blob not found" })),
        ))
    }
}
