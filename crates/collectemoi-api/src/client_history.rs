use axum::{
    Json,
    body::Bytes,
    http::{HeaderMap, StatusCode},
};
use collectemoi_db::DeviceSubmissionRow;
use uuid::Uuid;

use crate::{AppState, noise};

type ApiError = (StatusCode, Json<serde_json::Value>);

fn err(status: StatusCode, message: &str) -> ApiError {
    (status, Json(serde_json::json!({ "error": message })))
}

/// Return only submissions made by the device that proves possession of its
/// enrolled noise key. The encrypted proof is read-only and scoped to this route.
pub async fn list_submissions(
    axum::extract::State(state): axum::extract::State<AppState>,
    headers: HeaderMap,
    proof: Bytes,
) -> Result<Json<Vec<DeviceSubmissionRow>>, ApiError> {
    let raw_device_id = headers
        .get("x-device-id")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "device authentication required"))?;
    let device_id = Uuid::parse_str(raw_device_id)
        .map_err(|_| err(StatusCode::UNAUTHORIZED, "device authentication required"))?;
    let device_id = device_id.to_string();
    let device = state
        .db
        .device_get(&device_id)
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "device lookup failed"))?
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "device authentication required"))?;

    if device.banned {
        return Err(err(StatusCode::FORBIDDEN, "device is disabled"));
    }
    let key = noise::validate_noise_key_hex(&device.noise_key)
        .map_err(|_| err(StatusCode::UNAUTHORIZED, "device authentication required"))?;
    let plaintext = noise::open(&proof, &key)
        .map_err(|_| err(StatusCode::UNAUTHORIZED, "device authentication required"))?;
    let expected = format!("collectemoi-client-submissions-v1:{device_id}");
    if plaintext.as_slice() != expected.as_bytes() {
        return Err(err(
            StatusCode::UNAUTHORIZED,
            "device authentication required",
        ));
    }

    let submissions = state
        .db
        .submissions_for_device(&device.public_key, 1000)
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "history lookup failed"))?;
    Ok(Json(submissions))
}
