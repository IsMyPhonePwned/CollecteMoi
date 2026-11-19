//! Shared request/response types for the CollecteMoi device API.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Artefact kinds accepted by the upload API (mirrors the envelope).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ArtefactKind {
    AndroidBugreport,
    IosSysdiagnose,
    MessageReport,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateUploadRequest {
    pub case_id: Uuid,
    pub artefact: ArtefactKind,
    /// Total ciphertext size in bytes (full envelope).
    pub total_size: u64,
    /// Chunk size in bytes (default 8 MiB).
    pub chunk_size: u64,
    pub chunk_count: u32,
    /// Hex-encoded SHA-256 of the full envelope ciphertext.
    pub ciphertext_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateUploadResponse {
    pub upload_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UploadStatus {
    pub upload_id: Uuid,
    pub case_id: Uuid,
    pub artefact: ArtefactKind,
    pub total_size: u64,
    pub chunk_size: u64,
    pub chunk_count: u32,
    /// Bit i is set when chunk i has been received and verified.
    pub received: Vec<bool>,
    pub complete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CompleteUploadRequest {
    /// Optional: device public key (SEC1 uncompressed) used to verify the envelope signature.
    /// Required for `complete` to validate the envelope; hex-encoded.
    pub device_public_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CompleteUploadResponse {
    pub upload_id: Uuid,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ErrorBody {
    pub error: String,
}

/// Default chunk size: 8 MiB.
pub const DEFAULT_CHUNK_SIZE: u64 = 8 * 1024 * 1024;
