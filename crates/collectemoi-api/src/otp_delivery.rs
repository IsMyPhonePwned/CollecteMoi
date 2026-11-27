//! Pluggable OTP delivery. Deployments configure a webhook that sends the email.
//!
//! `POST <otp_delivery_webhook_url>` with JSON:
//! ```json
//! {
//!   "event": "enroll_otp",
//!   "email": "user@corp.example",
//!   "email_masked": "u***@corp.example",
//!   "code": "123456",
//!   "challenge_id": "…",
//!   "company_id": "acme",
//!   "company_name": "Acme",
//!   "expires_in_secs": 600,
//!   "expires_at": 1730000000,
//!   "subject": "Acme Lab: your verification code",
//!   "body": "Your verification code for Acme is 123456.\n\nIt expires in 10 minutes."
//! }
//! ```
//! `subject` / `body` come from Client apps branding (OTP notification templates).
//! Optional `Authorization: Bearer <token>` when `otp_delivery_webhook_token` is set.
//! Expect HTTP 2xx; anything else fails enroll/start.

use serde::Serialize;
use uuid::Uuid;

use crate::settings::RuntimeSettings;

#[derive(Debug, Clone, Serialize)]
pub struct OtpWebhookPayload {
    pub event: &'static str,
    pub email: String,
    pub email_masked: String,
    pub code: String,
    pub challenge_id: Uuid,
    pub company_id: String,
    pub company_name: String,
    pub expires_in_secs: u64,
    pub expires_at: u64,
    /// Rendered notification subject from company branding.
    #[serde(default)]
    pub subject: String,
    /// Rendered notification body from company branding.
    #[serde(default)]
    pub body: String,
}

/// Deliver an enroll OTP via the configured webhook.
/// Returns `Ok(true)` when a webhook was called, `Ok(false)` when none is configured.
pub async fn deliver_enroll_otp(
    settings: &RuntimeSettings,
    payload: &OtpWebhookPayload,
) -> Result<bool, String> {
    let url = settings.otp_delivery_webhook_url.trim();
    if url.is_empty() {
        return Ok(false);
    }
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("otp delivery webhook URL must start with http:// or https://".into());
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("CollecteMoi-OTP-Delivery/0.1")
        .build()
        .map_err(|e| format!("otp webhook client: {e}"))?;

    let mut req = client
        .post(url)
        .header("Content-Type", "application/json")
        .header("X-CollecteMoi-Event", "enroll_otp")
        .json(payload);

    let token = settings.otp_delivery_webhook_token.trim();
    if !token.is_empty() {
        req = req.header("Authorization", format!("Bearer {token}"));
    }

    let res = req
        .send()
        .await
        .map_err(|e| format!("otp webhook request failed: {e}"))?;
    let status = res.status();
    if !status.is_success() {
        let body = res.text().await.unwrap_or_default();
        let snippet: String = body.chars().take(200).collect();
        return Err(format!(
            "otp webhook returned {status}{}",
            if snippet.is_empty() {
                String::new()
            } else {
                format!(": {snippet}")
            }
        ));
    }
    tracing::info!(
        email = %payload.email_masked,
        company = %payload.company_id,
        challenge = %payload.challenge_id,
        "otp delivered via webhook"
    );
    Ok(true)
}
