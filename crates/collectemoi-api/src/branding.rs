//! Lab white-label branding + admin-triggered mobile builds.
//!
//! Branding is per company under `<data-dir>/branding/<company_id>/` and is
//! synced into the Flutter tenant bundle when `collectemoi-app` runs with
//! `--tenant-dir`. Legacy flat `branding/tenant.json` migrates to `_default`.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::{
    Json,
    extract::{Multipart, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::AppState;

const BRANDING_DIR: &str = "branding";
const TENANT_FILE: &str = "tenant.json";
const THEME_FILE: &str = "theme.json";
const ASSETS_DIR: &str = "assets";
const BUILD_STATE: &str = "build_job.json";
const VERSIONS_FILE: &str = "versions.json";
/// Shared branding when no enrollment company is selected.
pub const DEFAULT_COMPANY_ID: &str = "_default";
/// First branding version created for a company.
pub const DEFAULT_VERSION: &str = "1.0.0";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrandingTenant {
    #[serde(default = "default_id")]
    pub id: String,
    #[serde(default = "default_display_name")]
    pub display_name: String,
    #[serde(default)]
    pub tagline: String,
    #[serde(default)]
    pub enroll_lead: String,
    #[serde(default)]
    pub server_base_url: String,
    #[serde(default)]
    pub support_contact: String,
    #[serde(default)]
    pub privacy_policy_url: String,
    #[serde(default = "default_locale")]
    pub default_locale: String,
    /// Locales shipped in the generated client (subset of en/fr/es).
    #[serde(default = "default_enabled_locales")]
    pub enabled_locales: Vec<String>,
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default = "default_collectors")]
    pub enabled_collectors: Vec<String>,
    #[serde(default)]
    pub tls_pins_spki_sha256: Vec<String>,
    #[serde(default)]
    pub case_encryption_public_key_hex: String,
    /// Subject line for the enroll OTP email/SMS notification (webhook payload).
    /// Placeholders: `{code}`, `{company_name}`, `{display_name}`, `{email}`,
    /// `{expires_in_mins}`, `{expires_in_secs}`.
    #[serde(default = "default_otp_notification_subject")]
    pub otp_notification_subject: String,
    /// Body for the enroll OTP notification delivered via webhook.
    #[serde(default = "default_otp_notification_body")]
    pub otp_notification_body: String,
    /// Per-screen user-facing copy (home, consent, collectors, …). Base language = English.
    #[serde(default)]
    pub copy: BrandingCopy,
    /// Optional per-locale overrides for [`copy`] (keys: `fr`, `es`, …).
    /// Empty / missing fields fall back to dictionary lookup of the English base, then English.
    #[serde(default)]
    pub copy_i18n: std::collections::BTreeMap<String, serde_json::Value>,
    /// Optional per-locale overrides for tagline / enroll / OTP notification text.
    #[serde(default)]
    pub strings_i18n: std::collections::BTreeMap<String, TenantStringsI18n>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TenantStringsI18n {
    #[serde(default)]
    pub tagline: String,
    #[serde(default)]
    pub enroll_lead: String,
    #[serde(default)]
    pub otp_notification_subject: String,
    #[serde(default)]
    pub otp_notification_body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrandingCopy {
    #[serde(default)]
    pub home: HomeCopy,
    #[serde(default)]
    pub consent: ConsentCopy,
    #[serde(default)]
    pub my_data: MyDataCopy,
    #[serde(default = "GuideCopy::android_default")]
    pub android_bugreport: GuideCopy,
    #[serde(default = "GuideCopy::ios_default")]
    pub ios_sysdiagnose: GuideCopy,
    #[serde(default)]
    pub message_report: MessageCopy,
    /// System notification strings shown while encrypting / uploading artefacts.
    #[serde(default)]
    pub notifications: NotificationsCopy,
}

impl Default for BrandingCopy {
    fn default() -> Self {
        default_branding_copy()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationsCopy {
    #[serde(default = "default_notif_encrypting_title")]
    pub encrypting_title: String,
    #[serde(default = "default_notif_uploading_title")]
    pub uploading_title: String,
    /// Progress title; `{pct}` is replaced with 0–100.
    #[serde(default = "default_notif_uploading_progress_title")]
    pub uploading_progress_title: String,
    #[serde(default = "default_notif_complete_title")]
    pub complete_title: String,
    #[serde(default = "default_notif_failed_title")]
    pub failed_title: String,
    /// Body after success; `{artefact}` is the collector label.
    #[serde(default = "default_notif_complete_body")]
    pub complete_body: String,
    /// Body after failure; `{artefact}` is the collector label.
    #[serde(default = "default_notif_failed_body")]
    pub failed_body: String,
}

impl Default for NotificationsCopy {
    fn default() -> Self {
        Self {
            encrypting_title: default_notif_encrypting_title(),
            uploading_title: default_notif_uploading_title(),
            uploading_progress_title: default_notif_uploading_progress_title(),
            complete_title: default_notif_complete_title(),
            failed_title: default_notif_failed_title(),
            complete_body: default_notif_complete_body(),
            failed_body: default_notif_failed_body(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HomeCopy {
    #[serde(default = "default_home_headline")]
    pub headline: String,
    #[serde(default = "default_home_body")]
    pub body: String,
    #[serde(default = "default_android_title")]
    pub android_title: String,
    #[serde(default = "default_android_subtitle")]
    pub android_subtitle: String,
    #[serde(default = "default_ios_title")]
    pub ios_title: String,
    #[serde(default = "default_ios_subtitle")]
    pub ios_subtitle: String,
    #[serde(default = "default_message_title")]
    pub message_title: String,
    #[serde(default = "default_message_subtitle")]
    pub message_subtitle: String,
}

impl Default for HomeCopy {
    fn default() -> Self {
        Self {
            headline: default_home_headline(),
            body: default_home_body(),
            android_title: default_android_title(),
            android_subtitle: default_android_subtitle(),
            ios_title: default_ios_title(),
            ios_subtitle: default_ios_subtitle(),
            message_title: default_message_title(),
            message_subtitle: default_message_subtitle(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsentCopy {
    #[serde(default = "default_consent_title")]
    pub title: String,
    #[serde(default = "default_consent_footer")]
    pub footer: String,
    #[serde(default = "default_consent_checkbox")]
    pub checkbox: String,
    #[serde(default = "default_consent_button")]
    pub button: String,
    #[serde(default = "default_disclosure_android")]
    pub disclosure_android: String,
    #[serde(default = "default_disclosure_ios")]
    pub disclosure_ios: String,
    #[serde(default = "default_disclosure_message")]
    pub disclosure_message: String,
}

impl Default for ConsentCopy {
    fn default() -> Self {
        Self {
            title: default_consent_title(),
            footer: default_consent_footer(),
            checkbox: default_consent_checkbox(),
            button: default_consent_button(),
            disclosure_android: default_disclosure_android(),
            disclosure_ios: default_disclosure_ios(),
            disclosure_message: default_disclosure_message(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MyDataCopy {
    #[serde(default = "default_mydata_title")]
    pub title: String,
    #[serde(default = "default_mydata_device")]
    pub device_heading: String,
    #[serde(default = "default_mydata_artefacts")]
    pub artefacts_heading: String,
    #[serde(default = "default_mydata_empty")]
    pub empty: String,
    #[serde(default = "default_mydata_clear")]
    pub clear_button: String,
}

impl Default for MyDataCopy {
    fn default() -> Self {
        Self {
            title: default_mydata_title(),
            device_heading: default_mydata_device(),
            artefacts_heading: default_mydata_artefacts(),
            empty: default_mydata_empty(),
            clear_button: default_mydata_clear(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuideCopy {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub headline: String,
    #[serde(default)]
    pub body: String,
}

impl GuideCopy {
    fn android_default() -> Self {
        Self {
            title: default_android_title(),
            headline: "We need a full system bugreport".into(),
            body: "Android will create a ZIP with logs, installed apps, and crash data. You stay in control: you start it from Developer options and share it into this app.".into(),
        }
    }
    fn ios_default() -> Self {
        Self {
            title: default_ios_title(),
            headline: "Prepare the phone".into(),
            body: "Unlock the phone, connect to Wi‑Fi, and charge above 30%. Sysdiagnose is heavy on CPU and storage.".into(),
        }
    }
}

impl Default for GuideCopy {
    fn default() -> Self {
        Self {
            title: String::new(),
            headline: String::new(),
            body: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageCopy {
    #[serde(default = "default_message_title")]
    pub title: String,
    #[serde(default = "default_message_lead")]
    pub lead: String,
    #[serde(default = "default_message_button")]
    pub button: String,
}

impl Default for MessageCopy {
    fn default() -> Self {
        Self {
            title: default_message_title(),
            lead: default_message_lead(),
            button: default_message_button(),
        }
    }
}

fn default_home_headline() -> String {
    "What do you need to send?".into()
}
fn default_home_body() -> String {
    "Your analyst asked for evidence from this phone. Follow the guided steps — we never collect anything without your consent.".into()
}
fn default_android_title() -> String {
    "Android bugreport".into()
}
fn default_android_subtitle() -> String {
    "System diagnostic ZIP via Developer options".into()
}
fn default_ios_title() -> String {
    "iOS sysdiagnose".into()
}
fn default_ios_subtitle() -> String {
    "Hardware button combo, then share the archive".into()
}
fn default_message_title() -> String {
    "Suspicious message".into()
}
fn default_message_subtitle() -> String {
    "Share, screenshot, or paste the message".into()
}
fn default_consent_title() -> String {
    "Consent before upload".into()
}
fn default_consent_footer() -> String {
    "The file is encrypted on this phone to the case key before upload. You can review or discard later from My data.".into()
}
fn default_consent_checkbox() -> String {
    "I understand and consent to send this artefact".into()
}
fn default_consent_button() -> String {
    "Encrypt and upload".into()
}
fn default_disclosure_android() -> String {
    "A bugreport can include installed apps, system logs, Wi‑Fi networks, account identifiers, and sometimes location history.".into()
}
fn default_disclosure_ios() -> String {
    "A sysdiagnose can include process lists, system logs, crash reports, network details, and other diagnostics collected by iOS.".into()
}
fn default_disclosure_message() -> String {
    "The message text, metadata you entered, and any screenshots will be sent encrypted to the analyst handling your case.".into()
}
fn default_mydata_title() -> String {
    "My data".into()
}
fn default_mydata_device() -> String {
    "Device".into()
}
fn default_mydata_artefacts() -> String {
    "Artefacts on this phone".into()
}
fn default_mydata_empty() -> String {
    "Nothing queued yet.".into()
}
fn default_mydata_clear() -> String {
    "Clear local enrollment".into()
}
fn default_message_lead() -> String {
    "We never open links on this device. Confirm any URL we detect below.".into()
}
fn default_message_button() -> String {
    "Continue to consent".into()
}
fn default_otp_notification_subject() -> String {
    "{display_name}: your verification code".into()
}
fn default_otp_notification_body() -> String {
    "Your verification code for {company_name} is {code}.\n\nIt expires in {expires_in_mins} minutes.\n\nIf you did not request this, you can ignore this message.".into()
}
fn default_notif_encrypting_title() -> String {
    "Encrypting…".into()
}
fn default_notif_uploading_title() -> String {
    "Uploading…".into()
}
fn default_notif_uploading_progress_title() -> String {
    "Uploading… {pct}%".into()
}
fn default_notif_complete_title() -> String {
    "Upload complete".into()
}
fn default_notif_failed_title() -> String {
    "Upload failed".into()
}
fn default_notif_complete_body() -> String {
    "{artefact} was sent to the lab.".into()
}
fn default_notif_failed_body() -> String {
    "{artefact} could not be sent.".into()
}

fn default_branding_copy() -> BrandingCopy {
    BrandingCopy {
        home: HomeCopy::default(),
        consent: ConsentCopy::default(),
        my_data: MyDataCopy::default(),
        android_bugreport: GuideCopy::android_default(),
        ios_sysdiagnose: GuideCopy::ios_default(),
        message_report: MessageCopy::default(),
        notifications: NotificationsCopy::default(),
    }
}

fn clip_copy(s: String, max: usize) -> String {
    s.chars().take(max).collect()
}

fn sanitize_copy(mut c: BrandingCopy) -> BrandingCopy {
    let fill = |s: String, fallback: String| {
        let t = s.trim().to_string();
        if t.is_empty() { fallback } else { t }
    };
    c.home.headline = clip_copy(fill(c.home.headline, default_home_headline()), 120);
    c.home.body = clip_copy(fill(c.home.body, default_home_body()), 500);
    c.home.android_title = clip_copy(fill(c.home.android_title, default_android_title()), 80);
    c.home.android_subtitle = clip_copy(
        fill(c.home.android_subtitle, default_android_subtitle()),
        160,
    );
    c.home.ios_title = clip_copy(fill(c.home.ios_title, default_ios_title()), 80);
    c.home.ios_subtitle = clip_copy(fill(c.home.ios_subtitle, default_ios_subtitle()), 160);
    c.home.message_title = clip_copy(fill(c.home.message_title, default_message_title()), 80);
    c.home.message_subtitle = clip_copy(
        fill(c.home.message_subtitle, default_message_subtitle()),
        160,
    );

    c.consent.title = clip_copy(fill(c.consent.title, default_consent_title()), 80);
    c.consent.footer = clip_copy(fill(c.consent.footer, default_consent_footer()), 400);
    c.consent.checkbox = clip_copy(fill(c.consent.checkbox, default_consent_checkbox()), 200);
    c.consent.button = clip_copy(fill(c.consent.button, default_consent_button()), 80);
    c.consent.disclosure_android = clip_copy(
        fill(c.consent.disclosure_android, default_disclosure_android()),
        500,
    );
    c.consent.disclosure_ios = clip_copy(
        fill(c.consent.disclosure_ios, default_disclosure_ios()),
        500,
    );
    c.consent.disclosure_message = clip_copy(
        fill(c.consent.disclosure_message, default_disclosure_message()),
        500,
    );

    c.my_data.title = clip_copy(fill(c.my_data.title, default_mydata_title()), 80);
    c.my_data.device_heading =
        clip_copy(fill(c.my_data.device_heading, default_mydata_device()), 80);
    c.my_data.artefacts_heading = clip_copy(
        fill(c.my_data.artefacts_heading, default_mydata_artefacts()),
        120,
    );
    c.my_data.empty = clip_copy(fill(c.my_data.empty, default_mydata_empty()), 160);
    c.my_data.clear_button = clip_copy(fill(c.my_data.clear_button, default_mydata_clear()), 80);

    let android = GuideCopy::android_default();
    c.android_bugreport.title =
        clip_copy(fill(c.android_bugreport.title, android.title.clone()), 80);
    c.android_bugreport.headline = clip_copy(
        fill(c.android_bugreport.headline, android.headline.clone()),
        120,
    );
    c.android_bugreport.body = clip_copy(fill(c.android_bugreport.body, android.body.clone()), 500);

    let ios = GuideCopy::ios_default();
    c.ios_sysdiagnose.title = clip_copy(fill(c.ios_sysdiagnose.title, ios.title.clone()), 80);
    c.ios_sysdiagnose.headline =
        clip_copy(fill(c.ios_sysdiagnose.headline, ios.headline.clone()), 120);
    c.ios_sysdiagnose.body = clip_copy(fill(c.ios_sysdiagnose.body, ios.body.clone()), 500);

    c.message_report.title = clip_copy(fill(c.message_report.title, default_message_title()), 80);
    c.message_report.lead = clip_copy(fill(c.message_report.lead, default_message_lead()), 400);
    c.message_report.button =
        clip_copy(fill(c.message_report.button, default_message_button()), 80);

    c.notifications.encrypting_title = clip_copy(
        fill(
            c.notifications.encrypting_title,
            default_notif_encrypting_title(),
        ),
        80,
    );
    c.notifications.uploading_title = clip_copy(
        fill(
            c.notifications.uploading_title,
            default_notif_uploading_title(),
        ),
        80,
    );
    c.notifications.uploading_progress_title = clip_copy(
        fill(
            c.notifications.uploading_progress_title,
            default_notif_uploading_progress_title(),
        ),
        80,
    );
    c.notifications.complete_title = clip_copy(
        fill(
            c.notifications.complete_title,
            default_notif_complete_title(),
        ),
        80,
    );
    c.notifications.failed_title = clip_copy(
        fill(c.notifications.failed_title, default_notif_failed_title()),
        80,
    );
    c.notifications.complete_body = clip_copy(
        fill(
            c.notifications.complete_body,
            default_notif_complete_body(),
        ),
        200,
    );
    c.notifications.failed_body = clip_copy(
        fill(c.notifications.failed_body, default_notif_failed_body()),
        200,
    );
    c
}

fn default_id() -> String {
    "default".into()
}
fn default_display_name() -> String {
    "CollecteMoi".into()
}
fn default_locale() -> String {
    "en".into()
}
fn default_enabled_locales() -> Vec<String> {
    vec!["en".into(), "fr".into(), "es".into()]
}
fn default_mode() -> String {
    "generic".into()
}

fn sanitize_locale_code(raw: &str) -> Option<String> {
    let t = raw.trim().to_ascii_lowercase();
    match t.as_str() {
        "en" | "fr" | "es" => Some(t),
        _ => None,
    }
}

fn sanitize_enabled_locales(list: Vec<String>, default_locale: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in list {
        if let Some(code) = sanitize_locale_code(&raw) {
            if !out.iter().any(|c| c == &code) {
                out.push(code);
            }
        }
    }
    if out.is_empty() {
        return default_enabled_locales();
    }
    if !out.iter().any(|c| c == "en") {
        out.insert(0, "en".into());
    }
    let def = sanitize_locale_code(default_locale).unwrap_or_else(|| "en".into());
    if !out.iter().any(|c| c == &def) {
        out.push(def);
    }
    out
}

fn json_get_str(v: &serde_json::Value, path: &[&str]) -> String {
    let mut cur = v;
    for p in path {
        cur = match cur.get(*p) {
            Some(next) => next,
            None => return String::new(),
        };
    }
    cur.as_str()
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn clip_opt(s: String, max: usize) -> String {
    if s.is_empty() {
        String::new()
    } else {
        clip_copy(s, max)
    }
}

/// Build a BrandingCopy from a partial JSON object (missing → empty, not English defaults).
fn branding_copy_from_partial(v: &serde_json::Value) -> BrandingCopy {
    BrandingCopy {
        home: HomeCopy {
            headline: clip_opt(json_get_str(v, &["home", "headline"]), 120),
            body: clip_opt(json_get_str(v, &["home", "body"]), 500),
            android_title: clip_opt(json_get_str(v, &["home", "android_title"]), 80),
            android_subtitle: clip_opt(json_get_str(v, &["home", "android_subtitle"]), 160),
            ios_title: clip_opt(json_get_str(v, &["home", "ios_title"]), 80),
            ios_subtitle: clip_opt(json_get_str(v, &["home", "ios_subtitle"]), 160),
            message_title: clip_opt(json_get_str(v, &["home", "message_title"]), 80),
            message_subtitle: clip_opt(json_get_str(v, &["home", "message_subtitle"]), 160),
        },
        consent: ConsentCopy {
            title: clip_opt(json_get_str(v, &["consent", "title"]), 80),
            footer: clip_opt(json_get_str(v, &["consent", "footer"]), 400),
            checkbox: clip_opt(json_get_str(v, &["consent", "checkbox"]), 200),
            button: clip_opt(json_get_str(v, &["consent", "button"]), 80),
            disclosure_android: clip_opt(json_get_str(v, &["consent", "disclosure_android"]), 500),
            disclosure_ios: clip_opt(json_get_str(v, &["consent", "disclosure_ios"]), 500),
            disclosure_message: clip_opt(json_get_str(v, &["consent", "disclosure_message"]), 500),
        },
        my_data: MyDataCopy {
            title: clip_opt(json_get_str(v, &["my_data", "title"]), 80),
            device_heading: clip_opt(json_get_str(v, &["my_data", "device_heading"]), 80),
            artefacts_heading: clip_opt(json_get_str(v, &["my_data", "artefacts_heading"]), 120),
            empty: clip_opt(json_get_str(v, &["my_data", "empty"]), 160),
            clear_button: clip_opt(json_get_str(v, &["my_data", "clear_button"]), 80),
        },
        android_bugreport: GuideCopy {
            title: clip_opt(json_get_str(v, &["android_bugreport", "title"]), 80),
            headline: clip_opt(json_get_str(v, &["android_bugreport", "headline"]), 120),
            body: clip_opt(json_get_str(v, &["android_bugreport", "body"]), 500),
        },
        ios_sysdiagnose: GuideCopy {
            title: clip_opt(json_get_str(v, &["ios_sysdiagnose", "title"]), 80),
            headline: clip_opt(json_get_str(v, &["ios_sysdiagnose", "headline"]), 120),
            body: clip_opt(json_get_str(v, &["ios_sysdiagnose", "body"]), 500),
        },
        message_report: MessageCopy {
            title: clip_opt(json_get_str(v, &["message_report", "title"]), 80),
            lead: clip_opt(json_get_str(v, &["message_report", "lead"]), 400),
            button: clip_opt(json_get_str(v, &["message_report", "button"]), 80),
        },
        notifications: NotificationsCopy {
            encrypting_title: clip_opt(
                json_get_str(v, &["notifications", "encrypting_title"]),
                80,
            ),
            uploading_title: clip_opt(
                json_get_str(v, &["notifications", "uploading_title"]),
                80,
            ),
            uploading_progress_title: clip_opt(
                json_get_str(v, &["notifications", "uploading_progress_title"]),
                80,
            ),
            complete_title: clip_opt(
                json_get_str(v, &["notifications", "complete_title"]),
                80,
            ),
            failed_title: clip_opt(json_get_str(v, &["notifications", "failed_title"]), 80),
            complete_body: clip_opt(
                json_get_str(v, &["notifications", "complete_body"]),
                200,
            ),
            failed_body: clip_opt(json_get_str(v, &["notifications", "failed_body"]), 200),
        },
    }
}

fn branding_copy_to_partial_json(c: &BrandingCopy) -> serde_json::Value {
    json!({
        "home": {
            "headline": c.home.headline,
            "body": c.home.body,
            "android_title": c.home.android_title,
            "android_subtitle": c.home.android_subtitle,
            "ios_title": c.home.ios_title,
            "ios_subtitle": c.home.ios_subtitle,
            "message_title": c.home.message_title,
            "message_subtitle": c.home.message_subtitle,
        },
        "consent": {
            "title": c.consent.title,
            "footer": c.consent.footer,
            "checkbox": c.consent.checkbox,
            "button": c.consent.button,
            "disclosure_android": c.consent.disclosure_android,
            "disclosure_ios": c.consent.disclosure_ios,
            "disclosure_message": c.consent.disclosure_message,
        },
        "my_data": {
            "title": c.my_data.title,
            "device_heading": c.my_data.device_heading,
            "artefacts_heading": c.my_data.artefacts_heading,
            "empty": c.my_data.empty,
            "clear_button": c.my_data.clear_button,
        },
        "android_bugreport": {
            "title": c.android_bugreport.title,
            "headline": c.android_bugreport.headline,
            "body": c.android_bugreport.body,
        },
        "ios_sysdiagnose": {
            "title": c.ios_sysdiagnose.title,
            "headline": c.ios_sysdiagnose.headline,
            "body": c.ios_sysdiagnose.body,
        },
        "message_report": {
            "title": c.message_report.title,
            "lead": c.message_report.lead,
            "button": c.message_report.button,
        },
        "notifications": {
            "encrypting_title": c.notifications.encrypting_title,
            "uploading_title": c.notifications.uploading_title,
            "uploading_progress_title": c.notifications.uploading_progress_title,
            "complete_title": c.notifications.complete_title,
            "failed_title": c.notifications.failed_title,
            "complete_body": c.notifications.complete_body,
            "failed_body": c.notifications.failed_body,
        },
    })
}

fn sanitize_strings_i18n(mut s: TenantStringsI18n) -> TenantStringsI18n {
    s.tagline = clip_opt(s.tagline, 120);
    s.enroll_lead = clip_opt(s.enroll_lead, 400);
    s.otp_notification_subject = clip_opt(s.otp_notification_subject, 160);
    s.otp_notification_body = clip_opt(s.otp_notification_body, 2000);
    s
}

fn sanitize_i18n_maps(
    enabled: &[String],
    copy_i18n: std::collections::BTreeMap<String, serde_json::Value>,
    strings_i18n: std::collections::BTreeMap<String, TenantStringsI18n>,
) -> (
    std::collections::BTreeMap<String, serde_json::Value>,
    std::collections::BTreeMap<String, TenantStringsI18n>,
) {
    let enabled: std::collections::BTreeSet<_> = enabled.iter().cloned().collect();
    let mut out_copy = std::collections::BTreeMap::new();
    for (k, v) in copy_i18n {
        let Some(code) = sanitize_locale_code(&k) else { continue };
        if code == "en" || !enabled.contains(&code) {
            continue;
        }
        if !v.is_object() {
            continue;
        }
        let copy = branding_copy_from_partial(&v);
        out_copy.insert(code, branding_copy_to_partial_json(&copy));
    }
    let mut out_strings = std::collections::BTreeMap::new();
    for (k, v) in strings_i18n {
        let Some(code) = sanitize_locale_code(&k) else { continue };
        if code == "en" || !enabled.contains(&code) {
            continue;
        }
        out_strings.insert(code, sanitize_strings_i18n(v));
    }
    (out_copy, out_strings)
}
fn default_collectors() -> Vec<String> {
    vec![
        "android_bugreport".into(),
        "ios_sysdiagnose".into(),
        "message_report".into(),
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrandingTheme {
    #[serde(default = "default_seed")]
    pub seed_color: String,
    #[serde(default = "default_brightness")]
    pub brightness_default: String,
    #[serde(default)]
    pub light: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    pub dark: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    pub launcher_icon: LauncherIconStyle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LauncherIconStyle {
    pub enabled: bool,
    pub source: String,
    pub text: String,
    pub font: String,
    /// Text size as a percentage of the icon canvas.
    pub size: u8,
    /// Text center in percentage coordinates, origin at the top left.
    pub x: u8,
    pub y: u8,
    /// Launcher image scale, where 100 means the full icon canvas.
    pub scale: u8,
    /// Clockwise launcher image rotation in degrees.
    pub rotation: i16,
    pub color: String,
    pub badge: bool,
    pub badge_color: String,
    pub badge_opacity: u8,
}

impl Default for LauncherIconStyle {
    fn default() -> Self {
        Self {
            enabled: false,
            source: "logo".into(),
            text: String::new(),
            font: "system".into(),
            size: 14,
            x: 50,
            y: 82,
            scale: 100,
            rotation: 0,
            color: "#FFFFFF".into(),
            badge: true,
            badge_color: "#000000".into(),
            badge_opacity: 72,
        }
    }
}

fn default_seed() -> String {
    "#0B6E4F".into()
}
fn default_brightness() -> String {
    "light".into()
}

fn valid_icon_color(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value[1..].chars().all(|c| c.is_ascii_hexdigit())
}

impl Default for BrandingTheme {
    fn default() -> Self {
        Self {
            seed_color: default_seed(),
            brightness_default: default_brightness(),
            light: serde_json::Map::new(),
            dark: serde_json::Map::new(),
            launcher_icon: LauncherIconStyle::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrandingPatch {
    pub display_name: Option<String>,
    pub tagline: Option<String>,
    pub enroll_lead: Option<String>,
    pub server_base_url: Option<String>,
    pub support_contact: Option<String>,
    pub privacy_policy_url: Option<String>,
    pub default_locale: Option<String>,
    pub enabled_locales: Option<Vec<String>>,
    pub otp_notification_subject: Option<String>,
    pub otp_notification_body: Option<String>,
    pub seed_color: Option<String>,
    pub launcher_icon: Option<LauncherIconStyle>,
    pub enabled_collectors: Option<Vec<String>>,
    pub copy: Option<BrandingCopy>,
    pub copy_i18n: Option<std::collections::BTreeMap<String, serde_json::Value>>,
    pub strings_i18n: Option<std::collections::BTreeMap<String, TenantStringsI18n>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildStatus {
    Idle,
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    Running,
    Done,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildStep {
    pub id: String,
    pub label: String,
    pub status: StepStatus,
    #[serde(default)]
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildJob {
    pub id: String,
    pub status: BuildStatus,
    #[serde(default)]
    pub company_id: String,
    #[serde(default)]
    pub company_name: String,
    #[serde(default)]
    pub version: String,
    pub platforms: Vec<String>,
    pub mode: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub message: String,
    pub log_tail: String,
    pub artefacts: Vec<String>,
    #[serde(default)]
    pub steps: Vec<BuildStep>,
}

impl Default for BuildJob {
    fn default() -> Self {
        Self {
            id: String::new(),
            status: BuildStatus::Idle,
            company_id: DEFAULT_COMPANY_ID.into(),
            company_name: "Default (shared)".into(),
            version: DEFAULT_VERSION.into(),
            platforms: vec![],
            mode: "debug".into(),
            started_at: String::new(),
            finished_at: None,
            message: String::new(),
            log_tail: String::new(),
            artefacts: vec![],
            steps: vec![],
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct BrandingQuery {
    #[serde(default)]
    pub company_id: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VersionsIndex {
    #[serde(default = "default_version_string")]
    active: String,
    #[serde(default)]
    versions: Vec<String>,
}

fn default_version_string() -> String {
    DEFAULT_VERSION.into()
}

impl Default for VersionsIndex {
    fn default() -> Self {
        Self {
            active: DEFAULT_VERSION.into(),
            versions: vec![DEFAULT_VERSION.into()],
        }
    }
}

#[derive(Clone)]
pub struct BuildGate {
    inner: Arc<Mutex<BuildGateInner>>,
}

struct BuildGateInner {
    busy: bool,
    started: Option<Instant>,
    job_id: Option<String>,
    cancel: bool,
    child_pid: Option<u32>,
}

impl BuildGate {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(BuildGateInner {
                busy: false,
                started: None,
                job_id: None,
                cancel: false,
                child_pid: None,
            })),
        }
    }

    fn try_begin(&self, job_id: &str) -> bool {
        let mut g = self.inner.lock().unwrap();
        if g.busy {
            if let Some(t) = g.started {
                if t.elapsed() > Duration::from_secs(45 * 60) {
                    g.busy = false;
                    g.cancel = false;
                    g.child_pid = None;
                    g.job_id = None;
                } else {
                    return false;
                }
            } else {
                return false;
            }
        }
        g.busy = true;
        g.started = Some(Instant::now());
        g.job_id = Some(job_id.to_string());
        g.cancel = false;
        g.child_pid = None;
        true
    }

    fn is_busy(&self) -> bool {
        self.inner.lock().unwrap().busy
    }

    fn end(&self) {
        let mut g = self.inner.lock().unwrap();
        g.busy = false;
        g.started = None;
        g.job_id = None;
        g.cancel = false;
        g.child_pid = None;
    }

    fn request_cancel(&self) -> Option<String> {
        let mut g = self.inner.lock().unwrap();
        if !g.busy {
            return None;
        }
        g.cancel = true;
        let job_id = g.job_id.clone();
        if let Some(pid) = g.child_pid.take() {
            drop(g);
            kill_pid_tree(pid);
        }
        job_id
    }

    fn is_cancelled(&self) -> bool {
        self.inner.lock().unwrap().cancel
    }

    fn set_child_pid(&self, pid: Option<u32>) {
        let mut g = self.inner.lock().unwrap();
        g.child_pid = pid;
    }
}

fn kill_pid_tree(pid: u32) {
    let pid_s = pid.to_string();
    let _ = Command::new("pkill")
        .args(["-TERM", "-P", &pid_s])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = Command::new("kill")
        .args(["-TERM", &pid_s])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    std::thread::sleep(Duration::from_millis(250));
    let _ = Command::new("pkill")
        .args(["-KILL", "-P", &pid_s])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = Command::new("kill")
        .args(["-KILL", &pid_s])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn branding_root(data_dir: &Path) -> PathBuf {
    data_dir.join(BRANDING_DIR)
}

fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    time::OffsetDateTime::from_unix_timestamp(secs)
        .ok()
        .and_then(|odt| {
            odt.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .unwrap_or_default()
}

fn sanitize_company_id(raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.is_empty() || t.len() > 64 {
        return None;
    }
    if !t
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    if t == "." || t == ".." || t.contains('/') || t.contains('\\') {
        return None;
    }
    Some(t.to_string())
}

/// Move legacy flat `branding/tenant.json` into `branding/_default/`.
fn migrate_legacy_flat_branding(data_dir: &Path) {
    let root = branding_root(data_dir);
    let legacy_tenant = root.join(TENANT_FILE);
    let dest = root.join(DEFAULT_COMPANY_ID);
    if !legacy_tenant.is_file() || dest.join(TENANT_FILE).is_file() {
        return;
    }
    let _ = fs::create_dir_all(dest.join(ASSETS_DIR));
    for name in [TENANT_FILE, THEME_FILE, BUILD_STATE] {
        let from = root.join(name);
        if from.is_file() {
            let _ = fs::rename(&from, dest.join(name));
        }
    }
    let legacy_assets = root.join(ASSETS_DIR);
    if legacy_assets.is_dir() {
        if let Ok(entries) = fs::read_dir(&legacy_assets) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_file() {
                    if let Some(name) = p.file_name() {
                        let _ = fs::rename(&p, dest.join(ASSETS_DIR).join(name));
                    }
                }
            }
        }
    }
}

fn company_branding_dir(data_dir: &Path, company_id: &str) -> PathBuf {
    branding_root(data_dir).join(company_id)
}

fn sanitize_version_id(raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.is_empty() || t.len() > 32 {
        return None;
    }
    if !t
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return None;
    }
    if t == "." || t == ".." || t.starts_with('.') {
        return None;
    }
    Some(t.to_string())
}

fn versions_index_path(company_root: &Path) -> PathBuf {
    company_root.join(VERSIONS_FILE)
}

fn read_versions_index(company_root: &Path) -> VersionsIndex {
    read_json_file(&versions_index_path(company_root)).unwrap_or_default()
}

fn write_versions_index(company_root: &Path, idx: &VersionsIndex) -> anyhow::Result<()> {
    write_json_pretty(&versions_index_path(company_root), idx)
}

/// Move company-level `tenant.json` into `1.0.0/` and write versions.json.
fn migrate_company_to_versions(company_root: &Path) {
    let legacy_tenant = company_root.join(TENANT_FILE);
    let vdir = company_root.join(DEFAULT_VERSION);
    if !legacy_tenant.is_file() || vdir.join(TENANT_FILE).is_file() {
        // Still ensure versions.json exists when version dirs are present.
        if !versions_index_path(company_root).is_file() {
            if let Ok(entries) = fs::read_dir(company_root) {
                let mut versions: Vec<String> = entries
                    .flatten()
                    .filter_map(|e| {
                        let p = e.path();
                        if p.is_dir() {
                            let name = e.file_name().to_string_lossy().to_string();
                            if name != ASSETS_DIR && sanitize_version_id(&name).is_some() {
                                return Some(name);
                            }
                        }
                        None
                    })
                    .collect();
                if !versions.is_empty() {
                    versions.sort();
                    let active = versions
                        .last()
                        .cloned()
                        .unwrap_or_else(|| DEFAULT_VERSION.into());
                    let _ = write_versions_index(company_root, &VersionsIndex { active, versions });
                }
            }
        }
        return;
    }
    let _ = fs::create_dir_all(vdir.join(ASSETS_DIR));
    for name in [TENANT_FILE, THEME_FILE] {
        let from = company_root.join(name);
        if from.is_file() {
            let _ = fs::rename(&from, vdir.join(name));
        }
    }
    let legacy_assets = company_root.join(ASSETS_DIR);
    if legacy_assets.is_dir() {
        if let Ok(entries) = fs::read_dir(&legacy_assets) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_file() {
                    if let Some(name) = p.file_name() {
                        let _ = fs::rename(&p, vdir.join(ASSETS_DIR).join(name));
                    }
                }
            }
        }
        let _ = fs::remove_dir(&legacy_assets);
    }
    let _ = write_versions_index(company_root, &VersionsIndex::default());
}

fn version_branding_dir(data_dir: &Path, company_id: &str, version: &str) -> PathBuf {
    company_branding_dir(data_dir, company_id).join(version)
}

fn ensure_company_branding_dir(data_dir: &Path, company_id: &str) -> std::io::Result<PathBuf> {
    migrate_legacy_flat_branding(data_dir);
    let root = company_branding_dir(data_dir, company_id);
    fs::create_dir_all(&root)?;
    migrate_company_to_versions(&root);
    Ok(root)
}

fn ensure_version_branding_dir(
    data_dir: &Path,
    company_id: &str,
    version: &str,
) -> std::io::Result<PathBuf> {
    let company_root = ensure_company_branding_dir(data_dir, company_id)?;
    let root = company_root.join(version);
    fs::create_dir_all(root.join(ASSETS_DIR))?;
    let mut idx = read_versions_index(&company_root);
    if !idx.versions.iter().any(|v| v == version) {
        idx.versions.push(version.to_string());
        idx.versions.sort();
        idx.versions.dedup();
    }
    if idx.active.is_empty() {
        idx.active = version.to_string();
    }
    let _ = write_versions_index(&company_root, &idx);
    Ok(root)
}

fn resolve_version(
    data_dir: &Path,
    company_id: &str,
    raw: Option<&str>,
) -> Result<String, (StatusCode, Json<serde_json::Value>)> {
    let company_root = ensure_company_branding_dir(data_dir, company_id).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
    })?;
    let mut idx = read_versions_index(&company_root);
    if idx.versions.is_empty() {
        let _ = ensure_version_branding_dir(data_dir, company_id, DEFAULT_VERSION);
        idx = read_versions_index(&company_root);
    }
    let requested = raw.map(str::trim).filter(|s| !s.is_empty());
    let version = if let Some(r) = requested {
        let v = sanitize_version_id(r).ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "invalid version (use letters, numbers, . _ -)" })),
            )
        })?;
        if !idx.versions.iter().any(|x| x == &v) {
            // Auto-create empty version dir on first select via API create; resolve fails if unknown.
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": format!("unknown version '{v}' — create it first"),
                    "versions": idx.versions,
                })),
            ));
        }
        v
    } else if idx.versions.iter().any(|v| v == &idx.active) {
        idx.active.clone()
    } else {
        idx.versions
            .last()
            .cloned()
            .unwrap_or_else(|| DEFAULT_VERSION.into())
    };
    Ok(version)
}

fn list_versions_json(data_dir: &Path, company_id: &str) -> serde_json::Value {
    let company_root = company_branding_dir(data_dir, company_id);
    migrate_company_to_versions(&company_root);
    let idx = read_versions_index(&company_root);
    json!({
        "active": idx.active,
        "versions": idx.versions,
    })
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &to)?;
        } else if ty.is_file() {
            fs::copy(entry.path(), to)?;
        }
    }
    Ok(())
}

/// Resolve the lab title shown in the mobile app for a company.
/// Prefers that company's branding `display_name`, then the company name,
/// then the global lab display name (for the shared default company).
pub fn company_lab_identity(
    data_dir: &Path,
    company_id: &str,
    company_name: &str,
    global_display_name: &str,
) -> (String, String) {
    let name = {
        let t = company_name.trim();
        if !t.is_empty() {
            t.to_string()
        } else if company_id == DEFAULT_COMPANY_ID {
            "Default (shared)".into()
        } else if !company_id.trim().is_empty() {
            company_id.to_string()
        } else {
            global_display_name.trim().to_string()
        }
    };

    migrate_legacy_flat_branding(data_dir);
    let company_root = company_branding_dir(data_dir, company_id);
    migrate_company_to_versions(&company_root);
    let idx = read_versions_index(&company_root);
    let version = if !idx.active.trim().is_empty() {
        idx.active
    } else if let Some(v) = idx.versions.first() {
        v.clone()
    } else {
        DEFAULT_VERSION.to_string()
    };
    let tenant_path = company_root.join(&version).join(TENANT_FILE);
    let legacy_tenant_path = company_root.join(TENANT_FILE);
    for path in [&tenant_path, &legacy_tenant_path] {
        if let Some(tenant) = read_json_file::<BrandingTenant>(path) {
            let dn = tenant.display_name.trim();
            if !dn.is_empty() {
                return (name, dn.to_string());
            }
        }
    }

    if company_id == DEFAULT_COMPANY_ID {
        let global = global_display_name.trim();
        (
            name,
            if global.is_empty() {
                default_display_name()
            } else {
                global.to_string()
            },
        )
    } else {
        (name.clone(), name)
    }
}

fn resolve_company(
    state: &AppState,
    raw: Option<&str>,
) -> Result<(String, String), (StatusCode, Json<serde_json::Value>)> {
    let requested = raw
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_COMPANY_ID);
    let id = sanitize_company_id(requested).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "invalid company_id" })),
        )
    })?;
    if id == DEFAULT_COMPANY_ID {
        return Ok((id, "Default (shared)".into()));
    }
    let companies = state.db.company_list().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "db error" })),
        )
    })?;
    let c = companies.iter().find(|c| c.id == id).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("unknown company_id '{id}' — create it under Enrollment → Companies first") })),
        )
    })?;
    Ok((c.id.clone(), c.name.clone()))
}

fn list_branding_companies(state: &AppState) -> Vec<serde_json::Value> {
    let mut out = vec![json!({
        "id": DEFAULT_COMPANY_ID,
        "name": "Default (shared)",
        "enabled": true,
    })];
    if let Ok(rows) = state.db.company_list() {
        for c in rows {
            out.push(json!({
                "id": c.id,
                "name": c.name,
                "enabled": c.enabled,
                "domains": c.domains,
            }));
        }
    }
    out
}

fn default_tenant(public_url: &str, display_name: &str, tenant_id: &str) -> BrandingTenant {
    BrandingTenant {
        id: if tenant_id == DEFAULT_COMPANY_ID {
            default_id()
        } else {
            tenant_id.to_string()
        },
        display_name: if display_name.trim().is_empty() {
            default_display_name()
        } else {
            display_name.to_string()
        },
        tagline: "Secure artefact collection".into(),
        enroll_lead: "Enter your work email to receive a one-time verification code.".into(),
        server_base_url: public_url.trim_end_matches('/').to_string(),
        support_contact: "support@collectemoi.example".into(),
        privacy_policy_url: "https://collectemoi.example/privacy".into(),
        default_locale: default_locale(),
        enabled_locales: default_enabled_locales(),
        mode: default_mode(),
        enabled_collectors: default_collectors(),
        tls_pins_spki_sha256: vec![],
        case_encryption_public_key_hex: String::new(),
        otp_notification_subject: default_otp_notification_subject(),
        otp_notification_body: default_otp_notification_body(),
        copy: default_branding_copy(),
        copy_i18n: std::collections::BTreeMap::new(),
        strings_i18n: std::collections::BTreeMap::new(),
    }
}

fn fill_otp_placeholders(
    template: &str,
    company_name: &str,
    display_name: &str,
    email: &str,
    code: &str,
    expires_in_secs: u64,
) -> String {
    let mins = expires_in_secs.div_ceil(60).max(1);
    template
        .replace("{code}", code)
        .replace("{company_name}", company_name)
        .replace("{display_name}", display_name)
        .replace("{email}", email)
        .replace("{expires_in_secs}", &expires_in_secs.to_string())
        .replace("{expires_in_mins}", &mins.to_string())
}

/// Load active branding OTP templates for a company and render subject/body.
pub fn render_otp_notification(
    data_dir: &Path,
    company_id: &str,
    company_name: &str,
    display_name: &str,
    email: &str,
    code: &str,
    expires_in_secs: u64,
) -> (String, String) {
    migrate_legacy_flat_branding(data_dir);
    let company_root = company_branding_dir(data_dir, company_id);
    migrate_company_to_versions(&company_root);
    let idx = read_versions_index(&company_root);
    let version = if !idx.active.trim().is_empty() {
        idx.active
    } else if let Some(v) = idx.versions.first() {
        v.clone()
    } else {
        DEFAULT_VERSION.to_string()
    };
    let tenant_path = company_root.join(&version).join(TENANT_FILE);
    let legacy_tenant_path = company_root.join(TENANT_FILE);
    let mut subject = default_otp_notification_subject();
    let mut body = default_otp_notification_body();
    for path in [&tenant_path, &legacy_tenant_path] {
        if let Some(tenant) = read_json_file::<BrandingTenant>(path) {
            let s = tenant.otp_notification_subject.trim();
            if !s.is_empty() {
                subject = s.to_string();
            }
            let b = tenant.otp_notification_body.trim();
            if !b.is_empty() {
                body = b.to_string();
            }
            break;
        }
    }
    (
        fill_otp_placeholders(
            &subject,
            company_name,
            display_name,
            email,
            code,
            expires_in_secs,
        ),
        fill_otp_placeholders(&body, company_name, display_name, email, code, expires_in_secs),
    )
}

fn read_json_file<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_json_pretty(path: &Path, value: &impl Serialize) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(value)?;
    fs::write(path, format!("{json}\n"))?;
    Ok(())
}

fn load_or_init_branding(
    state: &AppState,
    company_id: &str,
    company_name: &str,
    version: &str,
) -> anyhow::Result<(BrandingTenant, BrandingTheme, PathBuf)> {
    let root = ensure_version_branding_dir(&state.data_dir, company_id, version)?;
    let tenant_path = root.join(TENANT_FILE);
    let theme_path = root.join(THEME_FILE);

    let settings = state.settings.lock().unwrap().clone();
    let seed_name = if company_id == DEFAULT_COMPANY_ID {
        settings.display_name.clone()
    } else {
        company_name.to_string()
    };
    let mut tenant = read_json_file::<BrandingTenant>(&tenant_path)
        .unwrap_or_else(|| default_tenant(&settings.public_url, &seed_name, company_id));
    if tenant.server_base_url.trim().is_empty() {
        tenant.server_base_url = settings.public_url.clone();
    }
    if tenant.display_name.trim().is_empty() {
        tenant.display_name = seed_name;
    }
    if company_id != DEFAULT_COMPANY_ID {
        tenant.id = company_id.to_string();
    }
    let theme = read_json_file::<BrandingTheme>(&theme_path).unwrap_or_default();

    if !tenant_path.is_file() {
        write_json_pretty(&tenant_path, &tenant)?;
    }
    if !theme_path.is_file() {
        write_json_pretty(&theme_path, &theme)?;
    }
    Ok((tenant, theme, root))
}

fn asset_flags(root: &Path) -> (bool, bool) {
    let logo = root.join(ASSETS_DIR).join("logo.png").is_file()
        || root.join(ASSETS_DIR).join("logo.jpg").is_file()
        || root.join(ASSETS_DIR).join("logo.webp").is_file();
    let icon = root.join(ASSETS_DIR).join("icon.png").is_file()
        || root.join(ASSETS_DIR).join("icon-1024.png").is_file();
    (logo, icon)
}

fn find_asset(root: &Path, names: &[&str]) -> Option<PathBuf> {
    let dir = root.join(ASSETS_DIR);
    for n in names {
        let p = dir.join(n);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn load_job(data_dir: &Path) -> BuildJob {
    migrate_legacy_flat_branding(data_dir);
    // Prefer active job under a company dir; fall back to shared/_default then legacy root.
    let root = branding_root(data_dir);
    let mut best: Option<BuildJob> = None;
    if let Ok(entries) = fs::read_dir(&root) {
        for e in entries.flatten() {
            let p = e.path().join(BUILD_STATE);
            if let Some(job) = read_json_file::<BuildJob>(&p) {
                let take = match (&best, &job.status) {
                    (None, _) => true,
                    (Some(prev), BuildStatus::Running | BuildStatus::Queued)
                        if !matches!(prev.status, BuildStatus::Running | BuildStatus::Queued) =>
                    {
                        true
                    }
                    (Some(prev), _) => job.started_at > prev.started_at,
                };
                if take {
                    best = Some(job);
                }
            }
        }
    }
    best.or_else(|| read_json_file(&root.join(BUILD_STATE)))
        .unwrap_or_default()
}

/// Mark disk jobs that claim to be active as failed when no in-process build is running
/// (e.g. after API restart). Prevents Save branding from looking like it started a build.
fn reconcile_job(data_dir: &Path, gate: &BuildGate) -> BuildJob {
    let mut job = load_job(data_dir);
    if matches!(job.status, BuildStatus::Running | BuildStatus::Queued) && !gate.is_busy() {
        job.status = BuildStatus::Failed;
        job.finished_at = Some(now_rfc3339());
        if job.message.is_empty()
            || job.message.contains("building")
            || job.message.contains("queued")
        {
            job.message = "Build interrupted (server restarted or process ended)".into();
        }
        for step in &mut job.steps {
            if matches!(step.status, StepStatus::Running | StepStatus::Pending) {
                step.status = StepStatus::Failed;
                if step.detail.is_empty() {
                    step.detail = "interrupted".into();
                }
            }
        }
        let company = if job.company_id.is_empty() {
            DEFAULT_COMPANY_ID
        } else {
            job.company_id.as_str()
        };
        save_job(data_dir, company, &job);
    }
    job
}

fn save_job(data_dir: &Path, company_id: &str, job: &BuildJob) {
    if let Ok(root) = ensure_company_branding_dir(data_dir, company_id) {
        let _ = write_json_pretty(&root.join(BUILD_STATE), job);
    }
}

fn make_build_steps(platforms: &[String]) -> Vec<BuildStep> {
    let mut steps = vec![
        BuildStep {
            id: "prepare".into(),
            label: "Prepare branding".into(),
            status: StepStatus::Pending,
            detail: String::new(),
        },
        BuildStep {
            id: "toolchain".into(),
            label: "Check Flutter toolchain".into(),
            status: StepStatus::Pending,
            detail: String::new(),
        },
    ];
    for p in platforms {
        let (id, label) = if p == "android" {
            ("android", "Build Android APK")
        } else {
            ("ios", "Build iOS (simulator)")
        };
        steps.push(BuildStep {
            id: id.into(),
            label: label.into(),
            status: StepStatus::Pending,
            detail: String::new(),
        });
    }
    steps.push(BuildStep {
        id: "publish".into(),
        label: "Publish artefacts".into(),
        status: StepStatus::Pending,
        detail: String::new(),
    });
    steps
}

fn set_step(job: &mut BuildJob, id: &str, status: StepStatus, detail: &str) {
    if let Some(step) = job.steps.iter_mut().find(|s| s.id == id) {
        step.status = status;
        if !detail.is_empty() {
            step.detail = detail.to_string();
        }
    }
}

fn repo_root(data_dir: &Path) -> Option<PathBuf> {
    if let Ok(raw) = std::env::var("COLLECTEMOI_REPO") {
        let p = PathBuf::from(raw.trim());
        if p.join("scripts/collectemoi-app").is_file() {
            return Some(p);
        }
    }
    let parent = data_dir.parent()?;
    if parent.join("scripts/collectemoi-app").is_file() {
        return Some(parent.to_path_buf());
    }
    let cwd = std::env::current_dir().ok()?;
    if cwd.join("scripts/collectemoi-app").is_file() {
        return Some(cwd);
    }
    None
}

fn branding_asset_query(company_id: &str, version: &str) -> String {
    let mut parts = Vec::new();
    if company_id != DEFAULT_COMPANY_ID {
        parts.push(format!("company_id={company_id}"));
    }
    if version != DEFAULT_VERSION {
        parts.push(format!("version={version}"));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("?{}", parts.join("&"))
    }
}

const I18N_CATALOG_JSON: &str = include_str!("../static/i18n_catalog.json");

/// Built-in FR/ES dictionary used to preview auto-translations of English branding copy.
pub async fn get_i18n_catalog() -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    match serde_json::from_str::<serde_json::Value>(I18N_CATALOG_JSON) {
        Ok(v) => Ok(Json(v)),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("i18n catalog: {e}") })),
        )),
    }
}

pub async fn get_branding(
    State(state): State<AppState>,
    Query(q): Query<BrandingQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let (company_id, company_name) = resolve_company(&state, q.company_id.as_deref())?;
    let version = resolve_version(&state.data_dir, &company_id, q.version.as_deref())?;
    let (tenant, theme, root) = load_or_init_branding(&state, &company_id, &company_name, &version)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
        })?;
    let (has_logo, has_icon) = asset_flags(&root);
    let job = reconcile_job(&state.data_dir, &state.build_gate);
    let qparam = branding_asset_query(&company_id, &version);
    let versions = list_versions_json(&state.data_dir, &company_id);
    Ok(Json(json!({
        "ok": true,
        "company_id": company_id,
        "company_name": company_name,
        "version": version,
        "versions": versions,
        "companies": list_branding_companies(&state),
        "tenant": tenant,
        "theme": theme,
        "assets": {
            "logo": has_logo,
            "icon": has_icon,
            "logo_url": if has_logo { format!("/v1/admin/branding/assets/logo{qparam}") } else { String::new() },
            "icon_url": if has_icon { format!("/v1/admin/branding/assets/icon{qparam}") } else { String::new() },
        },
        "build": job,
        "hint": "Save branding when ready, then click Generate APK/IPA. Requires Flutter on the lab host.",
    })))
}

pub async fn put_branding(
    State(state): State<AppState>,
    Query(q): Query<BrandingQuery>,
    Json(patch): Json<BrandingPatch>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let (company_id, company_name) = resolve_company(&state, q.company_id.as_deref())?;
    let version = resolve_version(&state.data_dir, &company_id, q.version.as_deref())?;
    let (mut tenant, mut theme, root) =
        load_or_init_branding(&state, &company_id, &company_name, &version).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
        })?;

    if let Some(v) = patch.display_name {
        let t = v.trim();
        if t.is_empty() || t.len() > 64 {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "display_name must be 1–64 characters" })),
            ));
        }
        tenant.display_name = t.to_string();
    }
    if let Some(v) = patch.tagline {
        tenant.tagline = v.chars().take(120).collect();
    }
    if let Some(v) = patch.enroll_lead {
        tenant.enroll_lead = v.chars().take(400).collect();
    }
    if let Some(v) = patch.server_base_url {
        let t = v.trim().trim_end_matches('/').to_string();
        if !t.is_empty() {
            tenant.server_base_url = t;
        }
    }
    if let Some(v) = patch.support_contact {
        tenant.support_contact = v.chars().take(120).collect();
    }
    if let Some(v) = patch.privacy_policy_url {
        tenant.privacy_policy_url = v.chars().take(300).collect();
    }
    if let Some(v) = patch.default_locale {
        tenant.default_locale =
            sanitize_locale_code(&v).unwrap_or_else(|| default_locale());
    }
    if let Some(v) = patch.enabled_locales {
        tenant.enabled_locales = sanitize_enabled_locales(v, &tenant.default_locale);
    } else {
        tenant.enabled_locales =
            sanitize_enabled_locales(tenant.enabled_locales.clone(), &tenant.default_locale);
    }
    if let Some(v) = patch.otp_notification_subject {
        let t = v.trim();
        tenant.otp_notification_subject = if t.is_empty() {
            default_otp_notification_subject()
        } else {
            clip_copy(t.to_string(), 160)
        };
    }
    if let Some(v) = patch.otp_notification_body {
        let t = v.trim();
        tenant.otp_notification_body = if t.is_empty() {
            default_otp_notification_body()
        } else {
            clip_copy(t.to_string(), 2000)
        };
    }
    if let Some(v) = patch.seed_color {
        let t = v.trim();
        if t.starts_with('#') && (t.len() == 7 || t.len() == 4) {
            theme.seed_color = t.to_string();
        } else if !t.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "seed_color must look like #0B6E4F" })),
            ));
        }
    }
    if let Some(mut style) = patch.launcher_icon {
        style.text = style.text.trim().to_string();
        style.source = style.source.to_ascii_lowercase();
        style.font = style.font.to_ascii_lowercase();
        style.color = style.color.to_ascii_uppercase();
        style.badge_color = style.badge_color.to_ascii_uppercase();
        if style.text.chars().count() > 24 || style.text.chars().any(char::is_control) {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "launcher icon text must be 24 characters or fewer" })),
            ));
        }
        if !["logo", "icon"].contains(&style.source.as_str())
            || !["system", "rounded", "serif", "monospaced"].contains(&style.font.as_str())
            || !(8..=32).contains(&style.size)
            || style.x > 100
            || style.y > 100
            || !(40..=180).contains(&style.scale)
            || !(-180..=180).contains(&style.rotation)
            || style.badge_opacity > 100
            || !valid_icon_color(&style.color)
            || !valid_icon_color(&style.badge_color)
        {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "invalid launcher icon text style" })),
            ));
        }
        theme.launcher_icon = style;
    }
    if let Some(cols) = patch.enabled_collectors {
        let allowed = ["android_bugreport", "ios_sysdiagnose", "message_report"];
        tenant.enabled_collectors = cols
            .into_iter()
            .filter(|c| allowed.contains(&c.as_str()))
            .collect();
    }
    if let Some(copy) = patch.copy {
        tenant.copy = sanitize_copy(copy);
    }
    if patch.copy_i18n.is_some() || patch.strings_i18n.is_some() {
        let copy_i18n = patch.copy_i18n.unwrap_or_else(|| tenant.copy_i18n.clone());
        let strings_i18n = patch
            .strings_i18n
            .unwrap_or_else(|| tenant.strings_i18n.clone());
        let (c, s) = sanitize_i18n_maps(&tenant.enabled_locales, copy_i18n, strings_i18n);
        tenant.copy_i18n = c;
        tenant.strings_i18n = s;
    } else {
        let (c, s) = sanitize_i18n_maps(
            &tenant.enabled_locales,
            tenant.copy_i18n.clone(),
            tenant.strings_i18n.clone(),
        );
        tenant.copy_i18n = c;
        tenant.strings_i18n = s;
    }

    write_json_pretty(&root.join(TENANT_FILE), &tenant).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
    })?;
    write_json_pretty(&root.join(THEME_FILE), &theme).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
    })?;

    // Mark this version active when saved.
    if let Ok(company_root) = ensure_company_branding_dir(&state.data_dir, &company_id) {
        let mut idx = read_versions_index(&company_root);
        idx.active = version.clone();
        if !idx.versions.iter().any(|v| v == &version) {
            idx.versions.push(version.clone());
            idx.versions.sort();
            idx.versions.dedup();
        }
        let _ = write_versions_index(&company_root, &idx);
    }

    Ok(Json(json!({
        "ok": true,
        "company_id": company_id,
        "company_name": company_name,
        "version": version,
        "tenant": tenant,
        "theme": theme
    })))
}

pub async fn upload_branding_asset(
    State(state): State<AppState>,
    Query(q): Query<BrandingQuery>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let (company_id, company_name) = resolve_company(&state, q.company_id.as_deref())?;
    let version = resolve_version(&state.data_dir, &company_id, q.version.as_deref())?;
    let root =
        ensure_version_branding_dir(&state.data_dir, &company_id, &version).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
        })?;
    let _ = load_or_init_branding(&state, &company_id, &company_name, &version);
    let assets = root.join(ASSETS_DIR);
    let mut saved = Vec::new();

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": e.to_string() })),
        )
    })? {
        let name = field.name().unwrap_or("").to_string();
        let filename = field.file_name().unwrap_or("").to_string();
        let data = field.bytes().await.map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": e.to_string() })),
            )
        })?;
        if data.len() > 8 * 1024 * 1024 {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "asset too large (max 8 MiB)" })),
            ));
        }
        let kind = if name == "logo" || filename.to_ascii_lowercase().contains("logo") {
            "logo"
        } else if name == "icon" || filename.to_ascii_lowercase().contains("icon") {
            "icon"
        } else {
            continue;
        };
        let ext = Path::new(&filename)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("png")
            .to_ascii_lowercase();
        let ext = match ext.as_str() {
            "png" | "jpg" | "jpeg" | "webp" => {
                if ext == "jpeg" {
                    "jpg".into()
                } else {
                    ext
                }
            }
            _ => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": "logo/icon must be png, jpg, or webp" })),
                ));
            }
        };
        let dest_name = if kind == "logo" {
            format!("logo.{ext}")
        } else {
            format!("icon.{ext}")
        };
        // Clear previous variants of this kind only (keep the other asset).
        let clear = if kind == "logo" {
            &["logo.png", "logo.jpg", "logo.webp"][..]
        } else {
            &["icon.png", "icon.jpg", "icon.webp", "icon-1024.png"][..]
        };
        for old in clear {
            let _ = fs::remove_file(assets.join(old));
        }
        let dest = assets.join(&dest_name);
        fs::write(&dest, &data).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
        })?;
        if kind == "icon" && dest_name != "icon.png" {
            let _ = fs::copy(&dest, assets.join("icon.png"));
        }
        saved.push(dest_name);
    }

    if saved.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "expected multipart fields logo and/or icon" })),
        ));
    }
    Ok(Json(json!({
        "ok": true,
        "company_id": company_id,
        "version": version,
        "saved": saved
    })))
}

pub async fn get_branding_asset(
    State(state): State<AppState>,
    Query(q): Query<BrandingQuery>,
    axum::extract::Path(kind): axum::extract::Path<String>,
) -> Result<axum::response::Response, (StatusCode, Json<serde_json::Value>)> {
    use axum::body::Body;
    use axum::http::{HeaderValue, header};
    use axum::response::IntoResponse;

    let (company_id, _) = resolve_company(&state, q.company_id.as_deref())?;
    let version = resolve_version(&state.data_dir, &company_id, q.version.as_deref())?;
    let root = version_branding_dir(&state.data_dir, &company_id, &version);
    let path = match kind.as_str() {
        "logo" => find_asset(&root, &["logo.png", "logo.jpg", "logo.webp"]),
        "icon" => find_asset(
            &root,
            &["icon.png", "icon-1024.png", "icon.jpg", "icon.webp"],
        ),
        _ => None,
    }
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "asset not found" })),
        )
    })?;

    let bytes = fs::read(&path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
    })?;
    let ctype = match path.extension().and_then(|e| e.to_str()).unwrap_or("png") {
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        _ => "image/png",
    };
    let mut res = Body::from(bytes).into_response();
    res.headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(ctype));
    res.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(res)
}

#[derive(Debug, Deserialize)]
pub struct BuildRequest {
    #[serde(default = "default_platforms")]
    pub platforms: Vec<String>,
    #[serde(default = "default_build_mode")]
    pub mode: String,
    #[serde(default)]
    pub company_id: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateVersionRequest {
    #[serde(default)]
    pub company_id: Option<String>,
    /// New version id (e.g. 1.1.0). Required.
    pub version: String,
    /// Copy from this version (default: active / latest).
    #[serde(default)]
    pub from_version: Option<String>,
}

fn default_platforms() -> Vec<String> {
    vec!["android".into()]
}
fn default_build_mode() -> String {
    "debug".into()
}

pub async fn create_version(
    State(state): State<AppState>,
    Json(req): Json<CreateVersionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let (company_id, company_name) = resolve_company(&state, req.company_id.as_deref())?;
    let new_version = sanitize_version_id(&req.version).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "invalid version (use letters, numbers, . _ -)" })),
        )
    })?;
    let company_root = ensure_company_branding_dir(&state.data_dir, &company_id).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
    })?;
    let mut idx = read_versions_index(&company_root);
    if idx.versions.iter().any(|v| v == &new_version) {
        return Err((
            StatusCode::CONFLICT,
            Json(json!({ "error": format!("version '{new_version}' already exists") })),
        ));
    }
    let from = if let Some(f) = req.from_version.as_deref() {
        sanitize_version_id(f).ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "invalid from_version" })),
            )
        })?
    } else if idx.versions.iter().any(|v| v == &idx.active) {
        idx.active.clone()
    } else {
        idx.versions
            .last()
            .cloned()
            .unwrap_or_else(|| DEFAULT_VERSION.into())
    };
    // Ensure source exists (create default if needed).
    let _ = load_or_init_branding(&state, &company_id, &company_name, &from).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
    })?;
    let src = version_branding_dir(&state.data_dir, &company_id, &from);
    let dst = version_branding_dir(&state.data_dir, &company_id, &new_version);
    if dst.exists() {
        return Err((
            StatusCode::CONFLICT,
            Json(json!({ "error": "version directory already exists" })),
        ));
    }
    copy_dir_recursive(&src, &dst).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("copy failed: {e}") })),
        )
    })?;
    idx.versions.push(new_version.clone());
    idx.versions.sort();
    idx.versions.dedup();
    idx.active = new_version.clone();
    write_versions_index(&company_root, &idx).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
    })?;
    Ok(Json(json!({
        "ok": true,
        "company_id": company_id,
        "company_name": company_name,
        "version": new_version,
        "from_version": from,
        "versions": list_versions_json(&state.data_dir, &company_id),
    })))
}

#[derive(Debug, Deserialize)]
pub struct DeleteVersionQuery {
    #[serde(default)]
    pub company_id: Option<String>,
    /// When true (default), also remove `data/mobile/<company>/<version>/`.
    #[serde(default = "default_true_bool")]
    pub delete_builds: bool,
}

fn default_true_bool() -> bool {
    true
}

fn remove_dir_if_under(root: &Path, target: &Path) -> Result<(), String> {
    let root = root
        .canonicalize()
        .map_err(|e| format!("canonicalize {}: {e}", root.display()))?;
    if !target.exists() {
        return Ok(());
    }
    let canon = target
        .canonicalize()
        .map_err(|e| format!("canonicalize {}: {e}", target.display()))?;
    if !canon.starts_with(&root) {
        return Err(format!(
            "refusing to delete path outside allowed root ({})",
            canon.display()
        ));
    }
    fs::remove_dir_all(&canon).map_err(|e| format!("remove {}: {e}", canon.display()))?;
    Ok(())
}

/// Delete a branding version directory and drop it from `versions.json`.
/// Refuses to delete the last remaining version for a company.
pub async fn delete_version(
    State(state): State<AppState>,
    axum::extract::Path(version_raw): axum::extract::Path<String>,
    Query(q): Query<DeleteVersionQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let (company_id, company_name) = resolve_company(&state, q.company_id.as_deref())?;
    let version = sanitize_version_id(&version_raw).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "invalid version" })),
        )
    })?;
    let company_root = ensure_company_branding_dir(&state.data_dir, &company_id).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
    })?;
    migrate_company_to_versions(&company_root);
    let mut idx = read_versions_index(&company_root);
    if !idx.versions.iter().any(|v| v == &version) {
        let dir = version_branding_dir(&state.data_dir, &company_id, &version);
        if !dir.exists() {
            return Err((
                StatusCode::NOT_FOUND,
                Json(json!({ "error": format!("unknown version '{version}'") })),
            ));
        }
    }
    if idx.versions.len() <= 1 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "cannot delete the last branding version — create another first"
            })),
        ));
    }

    let branding_dir = version_branding_dir(&state.data_dir, &company_id, &version);
    let branding_root = branding_root(&state.data_dir);
    remove_dir_if_under(&branding_root, &branding_dir).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
    })?;

    let mut deleted_builds = false;
    if q.delete_builds {
        let mobile_root = state.data_dir.join("mobile");
        let builds_dir = mobile_root.join(&company_id).join(&version);
        let had_builds = builds_dir.exists();
        match remove_dir_if_under(&mobile_root, &builds_dir) {
            Ok(()) => deleted_builds = had_builds,
            Err(e) => {
                tracing::warn!(error = %e, "failed to remove mobile builds for deleted version");
            }
        }
        // Best-effort: remove empty company mobile folder.
        let company_mobile = mobile_root.join(&company_id);
        if company_mobile.is_dir() {
            if let Ok(mut rd) = fs::read_dir(&company_mobile) {
                if rd.next().is_none() {
                    let _ = fs::remove_dir(&company_mobile);
                }
            }
        }
    }

    idx.versions.retain(|v| v != &version);
    if idx.active == version {
        idx.active = idx
            .versions
            .last()
            .cloned()
            .unwrap_or_else(|| DEFAULT_VERSION.into());
    }
    write_versions_index(&company_root, &idx).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
    })?;

    Ok(Json(json!({
        "ok": true,
        "company_id": company_id,
        "company_name": company_name,
        "deleted_version": version,
        "active": idx.active,
        "deleted_builds": deleted_builds,
        "versions": list_versions_json(&state.data_dir, &company_id),
    })))
}

pub async fn start_build(
    State(state): State<AppState>,
    Json(req): Json<BuildRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let (company_id, company_name) = resolve_company(&state, req.company_id.as_deref())?;
    let version = resolve_version(&state.data_dir, &company_id, req.version.as_deref())?;
    let platforms: Vec<String> = req
        .platforms
        .into_iter()
        .map(|p| p.to_ascii_lowercase())
        .filter(|p| p == "android" || p == "apk" || p == "ios")
        .map(|p| if p == "apk" { "android".into() } else { p })
        .collect();
    if platforms.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "platforms must include android and/or ios" })),
        ));
    }
    let mode = match req.mode.to_ascii_lowercase().as_str() {
        "release" => "release",
        "profile" => "profile",
        _ => "debug",
    }
    .to_string();
    let repo = repo_root(&state.data_dir).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "cannot find scripts/collectemoi-app — set COLLECTEMOI_REPO or keep data-dir as <repo>/data"
            })),
        )
    })?;
    let script = repo.join("scripts/collectemoi-app");
    if !script.is_file() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("missing {}", script.display()) })),
        ));
    }

    let (_, theme, branding) = load_or_init_branding(&state, &company_id, &company_name, &version)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
        })?;

    let job_id = uuid::Uuid::new_v4().to_string();
    if !state.build_gate.try_begin(&job_id) {
        return Err((
            StatusCode::CONFLICT,
            Json(json!({ "error": "a mobile build is already running" })),
        ));
    }

    let job = BuildJob {
        id: job_id.clone(),
        status: BuildStatus::Queued,
        company_id: company_id.clone(),
        company_name: company_name.clone(),
        version: version.clone(),
        platforms: platforms.clone(),
        mode: mode.clone(),
        started_at: now_rfc3339(),
        finished_at: None,
        message: format!("queued for {company_name} · v{version}"),
        log_tail: String::new(),
        artefacts: vec![],
        steps: make_build_steps(&platforms),
    };
    save_job(&state.data_dir, &company_id, &job);

    let data_dir = state.data_dir.clone();
    let gate = state.build_gate.clone();
    let public_url = state.settings.lock().unwrap().public_url.clone();
    let company_id_spawn = company_id.clone();
    let version_spawn = version.clone();

    tokio::task::spawn_blocking(move || {
        run_build_job(
            &repo,
            &script,
            &branding,
            &data_dir,
            &company_id_spawn,
            &version_spawn,
            &platforms,
            &mode,
            &theme.launcher_icon,
            &public_url,
            &job_id,
            gate,
        );
    });

    Ok(Json(json!({
        "ok": true,
        "company_id": company_id,
        "company_name": company_name,
        "version": version,
        "build": job,
        "hint": "Poll GET /v1/admin/branding until build.status is succeeded, failed, or cancelled",
    })))
}

pub async fn stop_build(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let Some(job_id) = state.build_gate.request_cancel() else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "no build is running" })),
        ));
    };
    let mut job = load_job(&state.data_dir);
    if job.id == job_id || matches!(job.status, BuildStatus::Running | BuildStatus::Queued) {
        job.status = BuildStatus::Cancelled;
        job.finished_at = Some(now_rfc3339());
        job.message = "stop requested — waiting for process to exit…".into();
        for s in job.steps.iter_mut() {
            if matches!(s.status, StepStatus::Running | StepStatus::Pending) {
                s.status = StepStatus::Skipped;
                if s.detail.is_empty() {
                    s.detail = "cancelled".into();
                }
            }
        }
        let company = if job.company_id.is_empty() {
            DEFAULT_COMPANY_ID
        } else {
            job.company_id.as_str()
        };
        save_job(&state.data_dir, company, &job);
    }
    Ok(Json(json!({
        "ok": true,
        "job_id": job_id,
        "build": load_job(&state.data_dir),
        "hint": "Build stop signal sent",
    })))
}

pub async fn get_build(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    Ok(Json(json!({
        "ok": true,
        "build": reconcile_job(&state.data_dir, &state.build_gate),
    })))
}

fn persist_job_tail(job: &mut BuildJob, log: &str) {
    job.log_tail = log
        .chars()
        .rev()
        .take(8000)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
}

enum RunCmdResult {
    Ok {
        stdout: String,
        stderr: String,
        code: i32,
    },
    Cancelled,
    SpawnErr(String),
}

fn run_command_cancellable(mut cmd: Command, gate: &BuildGate) -> RunCmdResult {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return RunCmdResult::SpawnErr(e.to_string()),
    };
    gate.set_child_pid(Some(child.id()));
    loop {
        if gate.is_cancelled() {
            kill_pid_tree(child.id());
            let _ = child.kill();
            let _ = child.wait();
            gate.set_child_pid(None);
            return RunCmdResult::Cancelled;
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                gate.set_child_pid(None);
                let mut stdout = String::new();
                let mut stderr = String::new();
                if let Some(mut out) = child.stdout.take() {
                    let _ = out.read_to_string(&mut stdout);
                }
                if let Some(mut err) = child.stderr.take() {
                    let _ = err.read_to_string(&mut stderr);
                }
                return RunCmdResult::Ok {
                    stdout,
                    stderr,
                    code: status.code().unwrap_or(-1),
                };
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(200)),
            Err(e) => {
                gate.set_child_pid(None);
                return RunCmdResult::SpawnErr(e.to_string());
            }
        }
    }
}

fn finish_cancelled(
    job: &mut BuildJob,
    data_dir: &Path,
    company_id: &str,
    log: &str,
    gate: &BuildGate,
) {
    job.status = BuildStatus::Cancelled;
    job.finished_at = Some(now_rfc3339());
    job.message = "build stopped by user".into();
    for s in job.steps.iter_mut() {
        if matches!(s.status, StepStatus::Running | StepStatus::Pending) {
            s.status = StepStatus::Skipped;
            if s.detail.is_empty() {
                s.detail = "cancelled".into();
            }
        }
    }
    persist_job_tail(job, log);
    save_job(data_dir, company_id, job);
    gate.end();
}

#[allow(clippy::too_many_arguments)]
fn run_build_job(
    repo: &Path,
    script: &Path,
    branding: &Path,
    data_dir: &Path,
    company_id: &str,
    version: &str,
    platforms: &[String],
    mode: &str,
    icon_style: &LauncherIconStyle,
    public_url: &str,
    job_id: &str,
    gate: BuildGate,
) {
    let mut job =
        read_json_file::<BuildJob>(&company_branding_dir(data_dir, company_id).join(BUILD_STATE))
            .unwrap_or_else(|| load_job(data_dir));
    if job.id != job_id {
        gate.end();
        return;
    }
    job.version = version.to_string();
    job.status = BuildStatus::Running;
    job.message = format!("Preparing branding v{version}…");
    set_step(
        &mut job,
        "prepare",
        StepStatus::Running,
        "Loading tenant + assets",
    );
    save_job(data_dir, company_id, &job);
    if gate.is_cancelled() {
        finish_cancelled(&mut job, data_dir, company_id, "", &gate);
        return;
    }

    let tenant = read_json_file::<BrandingTenant>(&branding.join(TENANT_FILE));
    set_step(
        &mut job,
        "prepare",
        StepStatus::Done,
        &format!(
            "{} · {}",
            tenant
                .as_ref()
                .map(|t| t.display_name.as_str())
                .unwrap_or("CollecteMoi"),
            branding.display()
        ),
    );
    set_step(
        &mut job,
        "toolchain",
        StepStatus::Running,
        "flutter --version",
    );
    job.message = "Checking Flutter toolchain…".into();
    save_job(data_dir, company_id, &job);

    let mut combined_log = String::new();
    let mut flutter_cmd = Command::new("flutter");
    flutter_cmd.arg("--version").current_dir(repo);
    match run_command_cancellable(flutter_cmd, &gate) {
        RunCmdResult::Ok {
            stdout,
            stderr,
            code,
        } => {
            let text = format!("{stdout}{stderr}");
            combined_log.push_str(&text);
            if code == 0 {
                let first = text.lines().next().unwrap_or("flutter ok");
                set_step(&mut job, "toolchain", StepStatus::Done, first);
            } else {
                set_step(
                    &mut job,
                    "toolchain",
                    StepStatus::Failed,
                    "flutter --version failed",
                );
                job.status = BuildStatus::Failed;
                job.finished_at = Some(now_rfc3339());
                job.message = "Flutter toolchain check failed".into();
                persist_job_tail(&mut job, &combined_log);
                save_job(data_dir, company_id, &job);
                gate.end();
                return;
            }
        }
        RunCmdResult::Cancelled => {
            finish_cancelled(&mut job, data_dir, company_id, &combined_log, &gate);
            return;
        }
        RunCmdResult::SpawnErr(e) => {
            combined_log.push_str(&format!("flutter not found: {e}\n"));
            set_step(
                &mut job,
                "toolchain",
                StepStatus::Failed,
                "flutter not on PATH",
            );
            job.status = BuildStatus::Failed;
            job.finished_at = Some(now_rfc3339());
            job.message = "Flutter not found on PATH".into();
            persist_job_tail(&mut job, &combined_log);
            save_job(data_dir, company_id, &job);
            gate.end();
            return;
        }
    }
    persist_job_tail(&mut job, &combined_log);
    save_job(data_dir, company_id, &job);

    let targets: Vec<&str> = platforms
        .iter()
        .map(|p| if p == "android" { "android" } else { "ios" })
        .collect();

    let mut ok_all = true;
    let mut artefacts = Vec::new();

    for target in targets {
        let step_id = if target == "android" {
            "android"
        } else {
            "ios"
        };
        set_step(
            &mut job,
            step_id,
            StepStatus::Running,
            &format!("{mode} build…"),
        );
        job.message = format!("Building {target} ({mode})…");
        save_job(data_dir, company_id, &job);

        let mut cmd_args = vec![
            format!("--{mode}"),
            "--tenant-dir".into(),
            branding.display().to_string(),
            "--display-name".into(),
            tenant
                .as_ref()
                .map(|t| t.display_name.clone())
                .unwrap_or_else(|| "CollecteMoi".into()),
            "--server".into(),
            tenant
                .as_ref()
                .map(|t| t.server_base_url.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| public_url.to_string()),
            // Bake branding version into Flutter --build-name / artefact names.
            "--app-version".into(),
            version.to_string(),
        ];
        if let Some(logo) = find_asset(branding, &["logo.png", "logo.jpg", "logo.webp"]) {
            cmd_args.push("--logo".into());
            cmd_args.push(logo.display().to_string());
        }
        let icon = if icon_style.source == "logo" {
            find_asset(branding, &["logo.png", "logo.jpg", "logo.webp"])
                .or_else(|| {
                    find_asset(branding, &["icon.png", "icon-1024.png", "icon.jpg", "icon.webp"])
                })
        } else {
            find_asset(branding, &["icon.png", "icon-1024.png", "icon.jpg", "icon.webp"])
                .or_else(|| find_asset(branding, &["logo.png", "logo.jpg", "logo.webp"]))
        };
        if let Some(icon) = icon {
            cmd_args.push("--icon".into());
            cmd_args.push(icon.display().to_string());
        }
        cmd_args.extend([
            "--icon-scale".into(),
            icon_style.scale.to_string(),
            "--icon-rotation".into(),
            icon_style.rotation.to_string(),
        ]);
        if icon_style.enabled && !icon_style.text.is_empty() {
            cmd_args.push("--icon-text".into());
            cmd_args.push(icon_style.text.clone());
            cmd_args.extend([
                "--icon-font".into(),
                icon_style.font.clone(),
                "--icon-size".into(),
                icon_style.size.to_string(),
                "--icon-x".into(),
                icon_style.x.to_string(),
                "--icon-y".into(),
                icon_style.y.to_string(),
                "--icon-color".into(),
                icon_style.color.clone(),
                "--icon-badge".into(),
                icon_style.badge.to_string(),
                "--icon-badge-color".into(),
                icon_style.badge_color.clone(),
                "--icon-badge-opacity".into(),
                icon_style.badge_opacity.to_string(),
            ]);
        }
        if target == "android" {
            cmd_args.insert(0, "apk".into());
        } else {
            cmd_args.insert(0, "ios".into());
            cmd_args.push("--simulator".into());
        }

        combined_log.push_str(&format!("$ collectemoi-app {}\n", cmd_args.join(" ")));
        persist_job_tail(&mut job, &combined_log);
        save_job(data_dir, company_id, &job);

        let mut build_cmd = Command::new(script);
        build_cmd.args(&cmd_args).current_dir(repo);
        match run_command_cancellable(build_cmd, &gate) {
            RunCmdResult::Ok {
                stdout,
                stderr,
                code,
            } => {
                combined_log.push_str(&stdout);
                combined_log.push_str(&stderr);
                if code != 0 {
                    ok_all = false;
                    combined_log.push_str(&format!("\nexit {code}\n"));
                    set_step(
                        &mut job,
                        step_id,
                        StepStatus::Failed,
                        &format!("exit {code}"),
                    );
                    break;
                }
                set_step(&mut job, step_id, StepStatus::Done, "ok");
            }
            RunCmdResult::Cancelled => {
                finish_cancelled(&mut job, data_dir, company_id, &combined_log, &gate);
                return;
            }
            RunCmdResult::SpawnErr(e) => {
                ok_all = false;
                combined_log.push_str(&format!("spawn failed: {e}\n"));
                set_step(&mut job, step_id, StepStatus::Failed, &e);
                break;
            }
        }
        persist_job_tail(&mut job, &combined_log);
        save_job(data_dir, company_id, &job);
    }

    if gate.is_cancelled() {
        finish_cancelled(&mut job, data_dir, company_id, &combined_log, &gate);
        return;
    }

    if ok_all {
        set_step(
            &mut job,
            "publish",
            StepStatus::Running,
            "Collecting APK/IPA",
        );
        job.message = "Publishing artefacts…".into();
        save_job(data_dir, company_id, &job);
    } else {
        set_step(
            &mut job,
            "publish",
            StepStatus::Skipped,
            "skipped after failure",
        );
        for s in job.steps.iter_mut() {
            if matches!(s.status, StepStatus::Pending) {
                s.status = StepStatus::Skipped;
            }
        }
    }

    // Collect published artefacts and copy into company/version folder.
    // Only copy *flat* publish dirs (not company subfolders) so old branded
    // builds are not re-copied into the active branding version.
    let company_mobile = data_dir.join("mobile").join(company_id).join(version);
    let _ = fs::create_dir_all(&company_mobile);
    let mut published_names: Vec<String> = Vec::new();
    for dir in [data_dir.join("mobile"), repo.join("dist/mobile")] {
        if let Ok(entries) = fs::read_dir(&dir) {
            for e in entries.flatten() {
                let p = e.path();
                if !p.is_file() {
                    continue;
                }
                if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                    let lower = name.to_ascii_lowercase();
                    if lower.ends_with(".apk")
                        || lower.ends_with(".ipa")
                        || (lower.ends_with(".zip") && lower.contains("ios"))
                    {
                        if !artefacts.iter().any(|a: &String| a == name) {
                            artefacts.push(name.to_string());
                        }
                        if ok_all {
                            let dest = company_mobile.join(name);
                            let _ = fs::copy(&p, &dest);
                            published_names.push(name.to_string());
                            // Keep SHA/version sidecar with the published copy.
                            let meta_src = {
                                let mut s = p.as_os_str().to_owned();
                                s.push(".meta.json");
                                PathBuf::from(s)
                            };
                            if meta_src.is_file() {
                                let mut meta_dst = dest.as_os_str().to_owned();
                                meta_dst.push(".meta.json");
                                let _ = fs::copy(&meta_src, PathBuf::from(meta_dst));
                            }
                        }
                    }
                }
            }
        }
    }

    // Drop superseded app builds (e.g. 1.0.8 APK left under branding 1.0.12).
    let mut pruned = 0usize;
    if ok_all {
        pruned += prune_superseded_mobile_builds(&company_mobile, &published_names);
        pruned += prune_superseded_mobile_builds(&data_dir.join("mobile"), &published_names);
    }

    if ok_all {
        set_step(
            &mut job,
            "publish",
            StepStatus::Done,
            &format!(
                "{} file(s) → data/mobile/{company_id}/{version}/{}",
                artefacts.len(),
                if pruned > 0 {
                    format!(" · pruned {pruned} older app build(s)")
                } else {
                    String::new()
                }
            ),
        );
    }

    persist_job_tail(&mut job, &combined_log);
    job.status = if ok_all {
        BuildStatus::Succeeded
    } else {
        BuildStatus::Failed
    };
    job.finished_at = Some(now_rfc3339());
    job.message = if ok_all {
        format!(
            "built {} for {} v{}",
            platforms.join("+"),
            job.company_name,
            version
        )
    } else {
        format!(
            "build failed for {} v{} — see log",
            job.company_name, version
        )
    };
    job.artefacts = artefacts;
    save_job(data_dir, company_id, &job);
    gate.end();
}

/// Versioned app artefacts look like `collectemoi-1.0.8-debug.apk`.
fn is_versioned_mobile_name(name: &str) -> bool {
    let stem = name
        .strip_suffix(".apk")
        .or_else(|| name.strip_suffix(".ipa"))
        .or_else(|| name.strip_suffix(".zip"))
        .unwrap_or(name);
    let Some(rest) = stem.strip_prefix("collectemoi-") else {
        return false;
    };
    // 1.2.3 or 1.2.3-debug / 1.2.3+10012 …
    let ver = rest.split(['-', '+']).next().unwrap_or("");
    let mut parts = ver.split('.');
    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(a), Some(b), Some(c), None)
            if !a.is_empty()
                && !b.is_empty()
                && !c.is_empty()
                && a.chars().all(|ch| ch.is_ascii_digit())
                && b.chars().all(|ch| ch.is_ascii_digit())
                && c.chars().all(|ch| ch.is_ascii_digit())
    )
}

fn is_mobile_alias_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "collectemoi-latest.apk"
            | "collectemoi-latest.ipa"
            | "collectemoi-debug.apk"
            | "collectemoi-release.apk"
            | "collectemoi-profile.apk"
            | "collectemoi-debug.ipa"
            | "collectemoi-release.ipa"
            | "collectemoi-profile.ipa"
    )
}

/// Remove older versioned APK/IPA files not in `keep`, leaving aliases alone.
fn prune_superseded_mobile_builds(dir: &Path, keep: &[String]) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let keep: std::collections::HashSet<&str> = keep.iter().map(String::as_str).collect();
    let mut removed = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.ends_with(".meta.json") {
            continue;
        }
        if is_mobile_alias_name(name) {
            continue;
        }
        if !is_versioned_mobile_name(name) {
            continue;
        }
        if keep.contains(name) {
            continue;
        }
        let meta = {
            let mut s = path.as_os_str().to_owned();
            s.push(".meta.json");
            PathBuf::from(s)
        };
        if fs::remove_file(&path).is_ok() {
            removed += 1;
            let _ = fs::remove_file(meta);
            tracing::info!(file = %path.display(), "pruned superseded mobile build");
        }
    }
    removed
}
