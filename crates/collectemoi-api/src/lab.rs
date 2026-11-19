//! Lab-only helpers: seal CM01 on the server when the Flutter web client
//! cannot load the native dylib. Enabled with `--lab-seal` (default on).

use axum::{Json, extract::State, http::StatusCode};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use collectemoi_envelope::{ArtefactKind, DeviceKeyPair, seal};
use serde::{Deserialize, Serialize};

use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct LabSealRequest {
    pub plaintext_b64: String,
    pub artefact: String,
    pub case_public_key: String,
    pub device_private_key: String,
}

#[derive(Debug, Serialize)]
pub struct LabSealResponse {
    pub envelope_b64: String,
}

pub async fn lab_seal(
    State(state): State<AppState>,
    Json(req): Json<LabSealRequest>,
) -> Result<Json<LabSealResponse>, (StatusCode, String)> {
    if !state.settings.lock().unwrap().lab_seal {
        return Err((
            StatusCode::FORBIDDEN,
            "lab seal disabled (enable via --lab-seal true or admin settings)".into(),
        ));
    }

    let plaintext = B64
        .decode(req.plaintext_b64.trim())
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("plaintext_b64: {e}")))?;
    let case_pk = hex::decode(req.case_public_key.trim())
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("case_public_key: {e}")))?;
    let device_sk = hex::decode(req.device_private_key.trim())
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("device_private_key: {e}")))?;

    let kind = match req.artefact.as_str() {
        "android_bugreport" => ArtefactKind::AndroidBugreport,
        "ios_sysdiagnose" => ArtefactKind::IosSysdiagnose,
        "message_report" => ArtefactKind::MessageReport,
        other => {
            return Err((StatusCode::BAD_REQUEST, format!("unknown artefact {other}")));
        }
    };

    let device = DeviceKeyPair::from_private_bytes(&device_sk)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("device key: {e}")))?;

    let envelope = seal(&plaintext, kind, &case_pk, &device)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("seal: {e}")))?;

    Ok(Json(LabSealResponse {
        envelope_b64: B64.encode(envelope),
    }))
}
