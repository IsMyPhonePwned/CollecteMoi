//! Runtime settings that the admin console can inspect and tweak
//! without restarting (bind address still requires restart).
//!
//! Mutable lab toggles (`display_name`, `public_url`, `lab_seal`,
//! `allow_dev_envelope`) are persisted in SQLite so they survive restart.

use serde::{Deserialize, Serialize};

pub const SETTINGS_KEY: &str = "runtime_settings";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeSettings {
    pub data_dir: String,
    pub bind: String,
    pub public_url: String,
    pub display_name: String,
    pub lab_seal: bool,
    pub allow_dev_envelope: bool,
    pub log: String,
    /// TLS enabled at process start (restart required to change).
    pub tls: bool,
    pub tls_cert: String,
    pub tls_key: String,
    /// Plain HTTP companion (lab admin without trusting the self-signed cert).
    pub http_bind: String,
    pub lab_reveal_otp: bool,
}

/// Subset of settings written to SQLite across process restarts.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PersistedSettings {
    #[serde(default)]
    pub public_url: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub lab_seal: Option<bool>,
    #[serde(default)]
    pub allow_dev_envelope: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct SettingsPatch {
    pub public_url: Option<String>,
    pub display_name: Option<String>,
    pub lab_seal: Option<bool>,
    pub allow_dev_envelope: Option<bool>,
}

impl RuntimeSettings {
    pub fn apply_patch(&mut self, patch: SettingsPatch) {
        if let Some(v) = patch.public_url {
            self.public_url = v.trim_end_matches('/').to_string();
        }
        if let Some(v) = patch.display_name {
            if !v.trim().is_empty() {
                self.display_name = v;
            }
        }
        if let Some(v) = patch.lab_seal {
            self.lab_seal = v;
        }
        if let Some(v) = patch.allow_dev_envelope {
            self.allow_dev_envelope = v;
        }
    }

    pub fn apply_persisted(&mut self, p: &PersistedSettings) {
        if let Some(ref url) = p.public_url {
            let t = url.trim().trim_end_matches('/');
            if !t.is_empty() {
                self.public_url = t.to_string();
            }
        }
        if let Some(ref name) = p.display_name {
            if !name.trim().is_empty() {
                self.display_name = name.clone();
            }
        }
        if let Some(v) = p.lab_seal {
            self.lab_seal = v;
        }
        if let Some(v) = p.allow_dev_envelope {
            self.allow_dev_envelope = v;
        }
    }

    pub fn to_persisted(&self) -> PersistedSettings {
        PersistedSettings {
            public_url: Some(self.public_url.clone()),
            display_name: Some(self.display_name.clone()),
            lab_seal: Some(self.lab_seal),
            allow_dev_envelope: Some(self.allow_dev_envelope),
        }
    }
}

/// Load persisted lab settings from SQLite (if any).
pub fn load_persisted(db: &collectemoi_db::Db) -> Option<PersistedSettings> {
    let raw = db.setting_get(SETTINGS_KEY).ok().flatten()?;
    serde_json::from_str(&raw).ok()
}

/// Save mutable lab settings to SQLite.
pub fn save_persisted(db: &collectemoi_db::Db, settings: &RuntimeSettings) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(&settings.to_persisted())?;
    db.setting_set(SETTINGS_KEY, &json)?;
    Ok(())
}
