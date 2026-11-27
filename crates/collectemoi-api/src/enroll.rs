use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::{Json, extract::State, http::StatusCode};
use collectemoi_db::{Db, DeviceRow};
use collectemoi_envelope::CaseKeyPair;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::AppState;
use crate::noise::{fingerprint_key, validate_noise_key_hex};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrollRequest {
    pub token: String,
    /// Device signing public key (P-256 SEC1 uncompressed), hex.
    pub device_public_key: String,
    /// Client-generated ChaCha20-Poly1305 noise key (32 bytes), hex.
    /// Exchanged at enroll; used to wrap CM01 envelopes as `CMNK` blobs.
    pub noise_key: String,
    pub platform: String,
    pub model: String,
    pub os_version: String,
    #[serde(default)]
    pub app_version: String,
    #[serde(default)]
    pub manufacturer: String,
    #[serde(default)]
    pub brand: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub details: serde_json::Value,
}

/// Client device + app identity carried at enroll and on subsequent packets.
#[derive(Debug, Clone, Default)]
pub struct DeviceClientMeta {
    pub platform: String,
    pub model: String,
    pub os_version: String,
    pub app_version: String,
    pub manufacturer: String,
    pub brand: String,
    pub name: String,
    pub details_json: String,
}

impl DeviceClientMeta {
    pub fn from_enroll_fields(
        platform: String,
        model: String,
        os_version: String,
        app_version: String,
        manufacturer: String,
        brand: String,
        name: String,
        details: serde_json::Value,
    ) -> Self {
        let details_json = match details {
            serde_json::Value::Null => String::new(),
            serde_json::Value::Object(map) if map.is_empty() => String::new(),
            other => other.to_string(),
        };
        Self {
            platform,
            model,
            os_version,
            app_version,
            manufacturer,
            brand,
            name,
            details_json,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrollResponse {
    pub device_id: Uuid,
    pub case_id: Uuid,
    pub case_public_key: String,
    pub server_base_url: String,
    /// Lab / product title for the enrolled company (from company branding).
    pub display_name: String,
    #[serde(default)]
    pub company_id: String,
    #[serde(default)]
    pub company_name: String,
    pub enabled_collectors: Vec<String>,
    /// SHA-256 of the exchanged noise key (server ack; never echoes the secret).
    pub noise_key_fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestResponse {
    pub display_name: String,
    pub server_base_url: String,
    pub case_public_key: String,
    pub enabled_collectors: Vec<String>,
    pub support_contact: String,
    pub privacy_policy_url: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionRequestDto {
    pub id: Uuid,
    pub case_id: Uuid,
    pub title: String,
    pub artefacts: Vec<String>,
    pub deadline: Option<String>,
}

/// Previous device keys retained after wipe/reinstall rebind (admin history).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceKeyHistoryEntry {
    pub replaced_at: String,
    pub reason: String,
    pub public_key: String,
    #[serde(default)]
    pub noise_key_fingerprint: String,
    #[serde(default)]
    pub platform: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub app_version: String,
    #[serde(default)]
    pub enrolled_at: String,
    #[serde(default)]
    pub last_seen_at: String,
    /// When absorbed from a superseded sibling row.
    #[serde(default)]
    pub from_device_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceRecord {
    pub device_id: Uuid,
    pub case_id: Uuid,
    pub public_key: String,
    /// ChaCha20 noise key (hex). Stored for unwrap; redacted in admin list views.
    #[serde(default)]
    pub noise_key: String,
    #[serde(default)]
    pub noise_key_fingerprint: String,
    pub platform: String,
    pub model: String,
    #[serde(default)]
    pub os_version: String,
    #[serde(default)]
    pub enrolled_at: String,
    #[serde(default)]
    pub banned: bool,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub company_id: String,
    #[serde(default)]
    pub app_version: String,
    #[serde(default)]
    pub manufacturer: String,
    #[serde(default)]
    pub brand: String,
    #[serde(default)]
    pub device_name: String,
    #[serde(default)]
    pub details_json: String,
    #[serde(default)]
    pub last_seen_at: String,
    /// When true, this device ignores the company upload-per-hour quota.
    #[serde(default)]
    pub rate_limit_bypass: bool,
    /// Previous public keys / fingerprints after rebind (newest last).
    #[serde(default)]
    pub key_history: Vec<DeviceKeyHistoryEntry>,
}

/// Lab admin device view — includes full keys (console is local/dev, no auth).
#[derive(Debug, Clone, Serialize)]
pub struct DevicePublicView {
    pub device_id: Uuid,
    pub case_id: Uuid,
    pub public_key: String,
    /// ChaCha20 noise key (hex). Exposed in lab admin only.
    pub noise_key: String,
    pub noise_key_fingerprint: String,
    pub has_noise_key: bool,
    pub platform: String,
    pub model: String,
    pub os_version: String,
    pub enrolled_at: String,
    pub banned: bool,
    pub public_key_banned: bool,
    pub email: String,
    pub company_id: String,
    pub app_version: String,
    pub manufacturer: String,
    pub brand: String,
    pub device_name: String,
    pub details_json: String,
    /// Convenience extract from `details_json` for admin list columns.
    pub security_patch: String,
    /// Convenience extract from `details_json` (`api_level` or `sdk`).
    pub api_level: String,
    /// Exact Android build string (`Build.DISPLAY` / Settings → Build number).
    pub build_display: String,
    pub build_type: String,
    pub android_release: String,
    /// `yes` | `emulator` (from device_info_plus).
    pub physical: String,
    pub low_ram: String,
    pub last_seen_at: String,
    pub rate_limit_bypass: bool,
    /// Previous keys for this identity (wipe/reinstall history).
    #[serde(default)]
    pub key_history: Vec<DeviceKeyHistoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenRecord {
    pub token: String,
    pub case_id: Uuid,
    pub reusable: bool,
    #[serde(default)]
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BannedKeyRecord {
    pub public_key: String,
    pub banned_at: String,
    #[serde(default)]
    pub reason: String,
}

pub struct EnrollmentStore {
    db: Arc<Db>,
    tokens: Mutex<HashMap<String, Uuid>>,
    reusable_tokens: Mutex<HashSet<String>>,
    /// Unix-seconds created_at for minted tokens (demo seeds use process start).
    token_created_at: Mutex<HashMap<String, String>>,
    devices: Mutex<HashMap<Uuid, DeviceRecord>>,
    banned_public_keys: Mutex<HashMap<String, BannedKeyRecord>>,
    case_public_key: String,
    case_private_key: String,
    keys_dir: PathBuf,
    devices_dir: PathBuf,
    banned_path: PathBuf,
    display_name: Mutex<String>,
    server_base_url: Mutex<String>,
}

pub struct EnrollmentOptions {
    pub public_url: String,
    pub case_pk: Option<PathBuf>,
    pub case_sk: Option<PathBuf>,
    /// Seed well-known enrollment tokens only for loopback development servers.
    pub seed_demo_tokens: bool,
}

impl EnrollmentStore {
    pub fn open(data_dir: &Path, db: Arc<Db>, opts: EnrollmentOptions) -> anyhow::Result<Self> {
        let mut tokens = HashMap::new();
        let mut reusable = HashSet::new();
        let demo_case = Uuid::parse_str("00000000-0000-4000-8000-000000000001").unwrap();
        let seeded_at = now_secs_str();
        let mut token_created_at = HashMap::new();
        if opts.seed_demo_tokens {
            tokens.insert("demo".into(), demo_case);
            tokens.insert("DEVTOKEN1".into(), demo_case);
            reusable.insert("demo".into());
            reusable.insert("DEVTOKEN1".into());
            token_created_at.insert("demo".into(), seeded_at.clone());
            token_created_at.insert("DEVTOKEN1".into(), seeded_at);
        }

        let keys_dir = data_dir.join("keys");
        let devices_dir = data_dir.join("devices");
        let banned_path = data_dir.join("banned_public_keys.json");
        fs::create_dir_all(&keys_dir)?;
        fs::create_dir_all(&devices_dir)?;

        let (case_public_key, case_private_key) =
            load_or_create_case_keys(&keys_dir, opts.case_pk.as_deref(), opts.case_sk.as_deref())?;

        // Prefer SQLite (already imported from legacy files on Db::open).
        let devices = load_devices_from_db(&db)?
            .or_else(|| load_devices(&devices_dir).ok())
            .unwrap_or_default();
        let banned_public_keys = load_banned_from_db(&db)?
            .or_else(|| load_banned(&banned_path).ok())
            .unwrap_or_default();

        Ok(Self {
            db,
            tokens: Mutex::new(tokens),
            reusable_tokens: Mutex::new(reusable),
            token_created_at: Mutex::new(token_created_at),
            devices: Mutex::new(devices),
            banned_public_keys: Mutex::new(banned_public_keys),
            case_public_key,
            case_private_key,
            keys_dir,
            devices_dir,
            banned_path,
            display_name: Mutex::new("CollecteMoi Lab".into()),
            server_base_url: Mutex::new(opts.public_url),
        })
    }

    /// Reload devices + banned keys from SQLite (after backup import).
    pub fn reload_from_db(&self) -> anyhow::Result<()> {
        if let Some(devices) = load_devices_from_db(&self.db)? {
            *self.devices.lock().unwrap() = devices;
        }
        if let Some(banned) = load_banned_from_db(&self.db)? {
            *self.banned_public_keys.lock().unwrap() = banned;
        }
        Ok(())
    }

    pub fn case_public_key_hex(&self) -> &str {
        &self.case_public_key
    }

    pub fn case_private_key_hex(&self) -> &str {
        &self.case_private_key
    }

    pub fn case_keypair(&self) -> anyhow::Result<CaseKeyPair> {
        let pk = hex::decode(&self.case_public_key)?;
        let sk = hex::decode(&self.case_private_key)?;
        CaseKeyPair::from_parts(&pk, &sk).map_err(|e| anyhow::anyhow!(e))
    }

    pub fn set_display_name(&self, name: &str) {
        *self.display_name.lock().unwrap() = name.to_string();
    }

    pub fn set_public_url(&self, url: &str) {
        *self.server_base_url.lock().unwrap() = url.trim_end_matches('/').to_string();
    }

    pub fn is_public_key_banned(&self, public_key: &str) -> bool {
        let key = normalize_pk(public_key);
        if key.is_empty() {
            return false;
        }
        if self.banned_public_keys.lock().unwrap().contains_key(&key) {
            return true;
        }
        if self
            .devices
            .lock()
            .unwrap()
            .values()
            .any(|d| normalize_pk(&d.public_key) == key && d.banned)
        {
            return true;
        }
        // SQLite is authoritative if the in-memory maps ever drift.
        if self.db.banned_contains(&key).unwrap_or(false) {
            return true;
        }
        matches!(
            self.db.device_find_by_pubkey(&key).ok().flatten(),
            Some(d) if d.banned
        )
    }

    /// True when the enrolled device id is marked banned (memory, then SQLite).
    pub fn is_device_id_banned(&self, device_id: &str) -> bool {
        let Ok(id) = Uuid::parse_str(device_id.trim()) else {
            return false;
        };
        if self
            .devices
            .lock()
            .unwrap()
            .get(&id)
            .map(|d| d.banned)
            .unwrap_or(false)
        {
            return true;
        }
        matches!(self.db.device_get(&id.to_string()).ok().flatten(), Some(d) if d.banned)
    }

    /// Refresh last_seen + client hardware/version from request headers.
    /// Optional `detail_updates` (security_patch, api_level, …) are merged into
    /// `details_json` only when values change, so chunk uploads stay cheap.
    pub fn touch_device(
        &self,
        device_id: &str,
        app_version: &str,
        platform: &str,
        model: &str,
        os_version: &str,
        manufacturer: &str,
        brand: &str,
        device_name: &str,
        detail_updates: &[(&str, &str)],
    ) {
        let Ok(id) = Uuid::parse_str(device_id) else {
            return;
        };
        let now = now_secs_str();
        let mut details_json_out: Option<String> = None;
        if let Some(d) = self.devices.lock().unwrap().get_mut(&id) {
            if !app_version.is_empty() {
                d.app_version = app_version.to_string();
            }
            if !platform.is_empty() {
                d.platform = platform.to_string();
            }
            if !model.is_empty() {
                d.model = model.to_string();
            }
            if !os_version.is_empty() {
                d.os_version = os_version.to_string();
            }
            if !manufacturer.is_empty() {
                d.manufacturer = manufacturer.to_string();
            }
            if !brand.is_empty() {
                d.brand = brand.to_string();
            }
            if !device_name.is_empty() {
                d.device_name = device_name.to_string();
            }
            if let Some(merged) = merge_device_details_json(&d.details_json, detail_updates) {
                d.details_json = merged.clone();
                details_json_out = Some(merged);
            }
            d.last_seen_at = now.clone();
        }
        let _ = self.db.device_touch(
            device_id,
            app_version,
            platform,
            model,
            os_version,
            manufacturer,
            brand,
            device_name,
            &now,
        );
        if let Some(details) = details_json_out {
            let _ = self.db.device_set_details_json(device_id, &details);
        }
    }

    pub fn list_devices(&self) -> Vec<DevicePublicView> {
        let banned = self.banned_public_keys.lock().unwrap();
        let mut list: Vec<_> = self
            .devices
            .lock()
            .unwrap()
            .values()
            .map(|d| device_public_view(d, banned.contains_key(&normalize_pk(&d.public_key))))
            .collect();
        list.sort_by(|a, b| a.enrolled_at.cmp(&b.enrolled_at));
        list
    }

    pub fn list_client_keys(&self) -> Vec<serde_json::Value> {
        self.list_devices()
            .into_iter()
            .map(|d| {
                serde_json::json!({
                    "device_id": d.device_id,
                    "case_id": d.case_id,
                    "public_key": d.public_key,
                    "noise_key": d.noise_key,
                    "noise_key_fingerprint": d.noise_key_fingerprint,
                    "has_noise_key": d.has_noise_key,
                    "banned": d.banned || d.public_key_banned,
                    "platform": d.platform,
                    "model": d.model,
                    "os_version": d.os_version,
                    "app_version": d.app_version,
                    "manufacturer": d.manufacturer,
                    "brand": d.brand,
                    "device_name": d.device_name,
                    "details_json": d.details_json,
                    "security_patch": d.security_patch,
                    "api_level": d.api_level,
                    "build_display": d.build_display,
                    "build_type": d.build_type,
                    "android_release": d.android_release,
                    "physical": d.physical,
                    "low_ram": d.low_ram,
                    "last_seen_at": d.last_seen_at,
                    "email": d.email,
                    "company_id": d.company_id,
                    "enrolled_at": d.enrolled_at,
                    "rate_limit_bypass": d.rate_limit_bypass,
                })
            })
            .collect()
    }

    pub fn list_banned_keys(&self) -> Vec<BannedKeyRecord> {
        let mut list: Vec<_> = self
            .banned_public_keys
            .lock()
            .unwrap()
            .values()
            .cloned()
            .collect();
        list.sort_by(|a, b| a.banned_at.cmp(&b.banned_at));
        list
    }

    pub fn ban_public_key(
        &self,
        public_key: &str,
        reason: &str,
    ) -> Result<BannedKeyRecord, String> {
        let key = normalize_pk(public_key);
        if key.is_empty() {
            return Err("public_key required".into());
        }
        hex::decode(&key).map_err(|_| "public_key must be hex".to_string())?;

        let record = BannedKeyRecord {
            public_key: key.clone(),
            banned_at: now_secs_str(),
            reason: reason.to_string(),
        };
        {
            let mut banned = self.banned_public_keys.lock().unwrap();
            banned.insert(key.clone(), record.clone());
            persist_banned(&self.db, &self.banned_path, &banned).map_err(|e| e.to_string())?;
        }
        let _ = self.db.banned_upsert(&key, &now_secs_str(), reason);

        // Mark matching devices.
        let mut devices = self.devices.lock().unwrap();
        for d in devices.values_mut() {
            if normalize_pk(&d.public_key) == key {
                d.banned = true;
                let _ = persist_device(&self.db, &self.devices_dir, d);
            }
        }
        let _ = self.db.device_ban_matching_pubkey(&key, true);
        Ok(record)
    }

    pub fn unban_public_key(&self, public_key: &str) -> Result<(), String> {
        let key = normalize_pk(public_key);
        {
            let mut banned = self.banned_public_keys.lock().unwrap();
            banned.remove(&key);
            persist_banned(&self.db, &self.banned_path, &banned).map_err(|e| e.to_string())?;
        }
        let _ = self.db.banned_remove(&key);

        let mut devices = self.devices.lock().unwrap();
        for d in devices.values_mut() {
            if normalize_pk(&d.public_key) == key {
                d.banned = false;
                let _ = persist_device(&self.db, &self.devices_dir, d);
            }
        }
        let _ = self.db.device_ban_matching_pubkey(&key, false);
        Ok(())
    }

    pub fn set_device_banned(
        &self,
        device_id: Uuid,
        banned: bool,
    ) -> Result<DevicePublicView, String> {
        let public_key = {
            let mut devices = self.devices.lock().unwrap();
            let d = devices
                .get_mut(&device_id)
                .ok_or_else(|| "device not found".to_string())?;
            d.banned = banned;
            persist_device(&self.db, &self.devices_dir, d).map_err(|e| e.to_string())?;
            d.public_key.clone()
        };
        if banned {
            let _ = self.ban_public_key(&public_key, "banned via device");
        } else {
            let _ = self.unban_public_key(&public_key);
        }
        self.list_devices()
            .into_iter()
            .find(|v| v.device_id == device_id)
            .ok_or_else(|| "device not found".into())
    }

    pub fn set_device_rate_limit_bypass(
        &self,
        device_id: Uuid,
        bypass: bool,
    ) -> Result<DevicePublicView, String> {
        {
            let mut devices = self.devices.lock().unwrap();
            let d = devices
                .get_mut(&device_id)
                .ok_or_else(|| "device not found".to_string())?;
            d.rate_limit_bypass = bypass;
            persist_device(&self.db, &self.devices_dir, d).map_err(|e| e.to_string())?;
        }
        let _ = self
            .db
            .device_set_rate_limit_bypass(&device_id.to_string(), bypass);
        self.list_devices()
            .into_iter()
            .find(|v| v.device_id == device_id)
            .ok_or_else(|| "device not found".into())
    }

    /// Company id + whether this device bypasses the company upload quota.
    pub fn device_rate_limit_context(&self, device_id: &str) -> Option<(String, bool)> {
        let id = Uuid::parse_str(device_id).ok()?;
        let devices = self.devices.lock().unwrap();
        let d = devices.get(&id)?;
        Some((d.company_id.clone(), d.rate_limit_bypass))
    }

    pub fn noise_key_for_pubkey(&self, public_key: &str) -> Option<Vec<u8>> {
        let want = normalize_pk(public_key);
        self.devices.lock().unwrap().values().find_map(|d| {
            if normalize_pk(&d.public_key) == want && !d.noise_key.is_empty() {
                hex::decode(&d.noise_key).ok()
            } else {
                None
            }
        })
    }

    pub fn list_tokens(&self) -> Vec<TokenRecord> {
        let tokens = self.tokens.lock().unwrap();
        let reusable = self.reusable_tokens.lock().unwrap();
        let created = self.token_created_at.lock().unwrap();
        let mut list: Vec<_> = tokens
            .iter()
            .map(|(token, case_id)| TokenRecord {
                token: token.clone(),
                case_id: *case_id,
                reusable: reusable.contains(token),
                created_at: created.get(token).cloned().unwrap_or_default(),
            })
            .collect();
        list.sort_by(|a, b| a.token.cmp(&b.token));
        list
    }

    pub fn add_token(&self, token: &str, case_id: Uuid) -> Result<(), String> {
        let token = token.trim();
        if token.is_empty() {
            return Err("token must not be empty".into());
        }
        let mut tokens = self.tokens.lock().unwrap();
        if tokens.contains_key(token) {
            return Err("token already exists".into());
        }
        tokens.insert(token.to_string(), case_id);
        self.token_created_at
            .lock()
            .unwrap()
            .insert(token.to_string(), now_secs_str());
        Ok(())
    }

    pub fn device_public_key(&self, device_id: Uuid) -> Option<String> {
        self.devices
            .lock()
            .unwrap()
            .get(&device_id)
            .map(|d| d.public_key.clone())
    }

    pub fn find_device_by_pubkey(&self, hex_pk: &str) -> Option<DeviceRecord> {
        let want = normalize_pk(hex_pk);
        self.devices
            .lock()
            .unwrap()
            .values()
            .find(|d| normalize_pk(&d.public_key) == want)
            .cloned()
    }

    /// Active (non-banned) devices already enrolled under this work email.
    pub fn active_device_count_for_email(&self, email: &str) -> usize {
        self.active_devices_for_email(email).len()
    }

    /// Active (non-banned) device rows for an email, newest check-in / enroll first.
    pub fn active_devices_for_email(&self, email: &str) -> Vec<DeviceRecord> {
        self.devices_for_email(email)
            .into_iter()
            .filter(|d| !d.banned)
            .collect()
    }

    /// All device rows for an email (including banned), newest check-in / enroll first.
    pub fn devices_for_email(&self, email: &str) -> Vec<DeviceRecord> {
        let email = email.trim();
        if email.is_empty() {
            return Vec::new();
        }
        let mut list: Vec<_> = self
            .devices
            .lock()
            .unwrap()
            .values()
            .filter(|d| d.email.eq_ignore_ascii_case(email))
            .cloned()
            .collect();
        list.sort_by(|a, b| {
            // Prefer non-banned, then most recently seen / enrolled.
            a.banned
                .cmp(&b.banned)
                .then_with(|| b.last_seen_at.cmp(&a.last_seen_at))
                .then_with(|| b.enrolled_at.cmp(&a.enrolled_at))
        });
        list
    }

    /// Enforce per-email device caps. `max == 0` means unlimited.
    ///
    /// Always allowed:
    /// - Re-enrolling the same device public key for the same email
    /// - When `max_devices_per_email == 1` and the slot is full (OTP/lab-proven
    ///   confirm will rebind the existing identity to the new keys after wipe/reinstall)
    pub fn assert_email_device_slot(
        &self,
        email: &str,
        max_devices_per_email: u32,
        device_public_key: Option<&str>,
    ) -> Result<(), String> {
        let email = email.trim();
        if email.is_empty() || max_devices_per_email == 0 {
            return Ok(());
        }
        if let Some(pk) = device_public_key {
            let want = normalize_pk(pk);
            if !want.is_empty() {
                if let Some(existing) = self.find_device_by_pubkey(&want) {
                    if !existing.banned && existing.email.eq_ignore_ascii_case(email) {
                        return Ok(());
                    }
                }
            }
        }
        let count = self.active_device_count_for_email(email);
        if count >= max_devices_per_email as usize {
            // Single-slot companies: wipe/reinstall mints new keys; OTP/lab proof
            // is enough to replace the one active identity on confirm.
            if max_devices_per_email == 1 {
                return Ok(());
            }
            return Err(format!(
                "this email already has the maximum of {max_devices_per_email} enrolled devices"
            ));
        }
        Ok(())
    }

    pub fn enroll(&self, req: EnrollRequest) -> Result<EnrollResponse, String> {
        let mut tokens = self.tokens.lock().unwrap();
        let case_id = tokens
            .remove(&req.token)
            .ok_or_else(|| "invalid or expired enrollment token".to_string())?;
        let reusable = self.reusable_tokens.lock().unwrap().contains(&req.token);
        if reusable {
            tokens.insert(req.token.clone(), case_id);
        }
        drop(tokens);

        let meta = DeviceClientMeta::from_enroll_fields(
            req.platform,
            req.model,
            req.os_version,
            req.app_version,
            req.manufacturer,
            req.brand,
            req.name,
            req.details,
        );
        self.enroll_verified(
            case_id,
            req.device_public_key,
            req.noise_key,
            meta,
            String::new(),
            String::new(),
        )
    }

    /// Complete enrollment after OTP (or legacy token) validation.
    pub fn enroll_verified(
        &self,
        case_id: Uuid,
        device_public_key: String,
        noise_key: String,
        meta: DeviceClientMeta,
        email: String,
        company_id: String,
    ) -> Result<EnrollResponse, String> {
        self.enroll_verified_limited(
            case_id,
            device_public_key,
            noise_key,
            meta,
            email,
            company_id,
            0, // caller supplies the company limit via the OTP confirm path
        )
    }

    pub fn enroll_verified_limited(
        &self,
        case_id: Uuid,
        device_public_key: String,
        noise_key: String,
        meta: DeviceClientMeta,
        email: String,
        company_id: String,
        max_devices_per_email: u32,
    ) -> Result<EnrollResponse, String> {
        let pk = normalize_pk(&device_public_key);
        if pk.is_empty() {
            return Err("device_public_key required".into());
        }
        hex::decode(&pk).map_err(|_| "device_public_key must be hex".to_string())?;
        if self.is_public_key_banned(&pk) {
            return Err("device public key is banned".into());
        }

        let noise_bytes = validate_noise_key_hex(&noise_key)?;
        let noise_fp = fingerprint_key(&noise_bytes);
        let noise_hex = hex::encode(&noise_bytes);
        let email_norm = email.trim().to_string();

        // Re-enroll: refresh the existing device row for the same pubkey + email.
        if let Some(existing) = self.find_device_by_pubkey(&pk) {
            if !existing.banned && existing.email.eq_ignore_ascii_case(&email_norm) {
                return self.finish_device_refresh(
                    existing,
                    case_id,
                    &pk,
                    noise_hex,
                    noise_fp,
                    meta,
                    email_norm,
                    company_id,
                    None,
                    max_devices_per_email == 1,
                );
            }
        }

        let active = self.active_devices_for_email(&email_norm);
        let at_capacity = max_devices_per_email > 0
            && active.len() >= max_devices_per_email as usize;

        // Wipe/reinstall (new keys) with max_devices_per_email == 1: rebind the
        // existing identity for this email instead of rejecting or creating a
        // duplicate row (covers both "still active" and "admin banned, then re-enroll").
        if max_devices_per_email == 1 {
            let existing = active.into_iter().next().or_else(|| {
                self.devices_for_email(&email_norm).into_iter().next()
            });
            if let Some(existing) = existing {
                let old_pk = normalize_pk(&existing.public_key);
                return self.finish_device_refresh(
                    existing,
                    case_id,
                    &pk,
                    noise_hex,
                    noise_fp,
                    meta,
                    email_norm,
                    company_id,
                    if old_pk.is_empty() || old_pk == pk {
                        None
                    } else {
                        Some(old_pk)
                    },
                    true,
                );
            }
        }

        if at_capacity {
            return Err(format!(
                "this email already has the maximum of {max_devices_per_email} enrolled devices"
            ));
        }

        let device_id = Uuid::new_v4();
        let enrolled_at = now_secs_str();
        let record = DeviceRecord {
            device_id,
            case_id,
            public_key: pk.clone(),
            noise_key: noise_hex,
            noise_key_fingerprint: noise_fp.clone(),
            platform: meta.platform,
            model: meta.model,
            os_version: meta.os_version,
            enrolled_at: enrolled_at.clone(),
            banned: false,
            email: email_norm,
            company_id: company_id.clone(),
            app_version: meta.app_version,
            manufacturer: meta.manufacturer,
            brand: meta.brand,
            device_name: meta.name,
            details_json: meta.details_json,
            last_seen_at: enrolled_at,
            rate_limit_bypass: false,
            key_history: Vec::new(),
        };
        self.devices
            .lock()
            .unwrap()
            .insert(device_id, record.clone());
        let _ = persist_device(&self.db, &self.devices_dir, &record);

        let path = self.keys_dir.join(format!("device-{device_id}.pk.hex"));
        let _ = fs::write(path, &pk);
        let noise_path = self.keys_dir.join(format!("device-{device_id}.noise.hex"));
        let _ = fs::write(noise_path, &record.noise_key);

        Ok(EnrollResponse {
            device_id,
            case_id,
            case_public_key: self.case_public_key.clone(),
            server_base_url: self.server_base_url.lock().unwrap().clone(),
            display_name: self.display_name.lock().unwrap().clone(),
            company_id,
            company_name: String::new(),
            enabled_collectors: vec![
                "android_bugreport".into(),
                "ios_sysdiagnose".into(),
                "message_report".into(),
            ],
            noise_key_fingerprint: noise_fp,
        })
    }

    /// Update an existing device row in place (same `device_id`), optionally
    /// retiring the previous public key after a wipe/reinstall rebind.
    fn finish_device_refresh(
        &self,
        existing: DeviceRecord,
        case_id: Uuid,
        pk: &str,
        noise_hex: String,
        noise_fp: String,
        meta: DeviceClientMeta,
        email: String,
        company_id: String,
        retire_old_public_key: Option<String>,
        retire_email_siblings: bool,
    ) -> Result<EnrollResponse, String> {
        let mut record = existing;
        let device_id = record.device_id;
        let old_pk = normalize_pk(&record.public_key);
        let keys_changed = old_pk != pk || record.noise_key_fingerprint != noise_fp;
        if keys_changed && !old_pk.is_empty() {
            record.key_history.push(history_entry_from_device(
                &record,
                "replaced by re-enrollment",
                "",
            ));
        }
        record.case_id = case_id;
        record.public_key = pk.to_string();
        record.noise_key = noise_hex;
        record.noise_key_fingerprint = noise_fp.clone();
        record.platform = meta.platform;
        record.model = meta.model;
        record.os_version = meta.os_version;
        record.app_version = meta.app_version;
        record.manufacturer = meta.manufacturer;
        record.brand = meta.brand;
        record.device_name = meta.name;
        record.details_json = meta.details_json;
        record.last_seen_at = now_secs_str();
        record.company_id = company_id.clone();
        record.email = email;
        record.banned = false;

        if retire_email_siblings {
            // Fold sibling rows into this identity's key history, then ban them.
            let absorbed = self.absorb_other_devices_for_email(device_id, &record.email);
            record.key_history.extend(absorbed);
        }

        self.devices
            .lock()
            .unwrap()
            .insert(device_id, record.clone());
        let _ = persist_device(&self.db, &self.devices_dir, &record);
        let path = self.keys_dir.join(format!("device-{device_id}.pk.hex"));
        let _ = fs::write(path, pk);
        let noise_path = self.keys_dir.join(format!("device-{device_id}.noise.hex"));
        let _ = fs::write(noise_path, &record.noise_key);

        if let Some(old_pk) = retire_old_public_key {
            // Device row already points at the new key, so banning the old key
            // only blocks the wiped install and does not re-ban this identity.
            let _ = self.ban_public_key(&old_pk, "replaced by re-enrollment");
        }

        Ok(EnrollResponse {
            device_id,
            case_id,
            case_public_key: self.case_public_key.clone(),
            server_base_url: self.server_base_url.lock().unwrap().clone(),
            display_name: self.display_name.lock().unwrap().clone(),
            company_id,
            company_name: String::new(),
            enabled_collectors: vec![
                "android_bugreport".into(),
                "ios_sysdiagnose".into(),
                "message_report".into(),
            ],
            noise_key_fingerprint: noise_fp,
        })
    }

    /// Capture sibling device keys into history entries and ban those rows.
    fn absorb_other_devices_for_email(
        &self,
        keep_device_id: Uuid,
        email: &str,
    ) -> Vec<DeviceKeyHistoryEntry> {
        let email = email.trim();
        if email.is_empty() {
            return Vec::new();
        }
        let stale: Vec<DeviceRecord> = self
            .devices
            .lock()
            .unwrap()
            .values()
            .filter(|d| d.device_id != keep_device_id && d.email.eq_ignore_ascii_case(email))
            .cloned()
            .collect();
        let mut absorbed = Vec::new();
        for sibling in stale {
            absorbed.extend(sibling.key_history.iter().cloned());
            if !normalize_pk(&sibling.public_key).is_empty() {
                absorbed.push(history_entry_from_device(
                    &sibling,
                    "superseded by re-enrollment",
                    &sibling.device_id.to_string(),
                ));
            }
            {
                let mut devices = self.devices.lock().unwrap();
                if let Some(d) = devices.get_mut(&sibling.device_id) {
                    d.banned = true;
                    let _ = persist_device(&self.db, &self.devices_dir, d);
                }
            }
            let pk = normalize_pk(&sibling.public_key);
            if !pk.is_empty() {
                let _ = self.ban_public_key(&pk, "superseded by re-enrollment");
            }
        }
        absorbed
    }

    pub fn manifest(&self) -> ManifestResponse {
        ManifestResponse {
            display_name: self.display_name.lock().unwrap().clone(),
            server_base_url: self.server_base_url.lock().unwrap().clone(),
            case_public_key: self.case_public_key.clone(),
            enabled_collectors: vec![
                "android_bugreport".into(),
                "ios_sysdiagnose".into(),
                "message_report".into(),
            ],
            support_contact: "support@collectemoi.example".into(),
            privacy_policy_url: "https://collectemoi.example/privacy".into(),
            signature: String::new(),
        }
    }

    pub fn requests(&self) -> Vec<CollectionRequestDto> {
        vec![CollectionRequestDto {
            id: Uuid::parse_str("00000000-0000-4000-8000-0000000000aa").unwrap(),
            case_id: Uuid::parse_str("00000000-0000-4000-8000-000000000001").unwrap(),
            title: "Initial collection".into(),
            artefacts: vec![
                "android_bugreport".into(),
                "ios_sysdiagnose".into(),
                "message_report".into(),
            ],
            deadline: None,
        }]
    }
}

fn normalize_pk(pk: &str) -> String {
    pk.trim().to_lowercase()
}

fn detail_field(details_json: &str, keys: &[&str]) -> String {
    if details_json.trim().is_empty() {
        return String::new();
    }
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(details_json)
    else {
        return String::new();
    };
    for key in keys {
        if let Some(v) = map.get(*key).and_then(|x| x.as_str()) {
            let t = v.trim();
            if !t.is_empty() {
                return t.to_string();
            }
        }
    }
    String::new()
}

fn device_public_view(d: &DeviceRecord, public_key_banned: bool) -> DevicePublicView {
    DevicePublicView {
        device_id: d.device_id,
        case_id: d.case_id,
        public_key: d.public_key.clone(),
        noise_key: d.noise_key.clone(),
        noise_key_fingerprint: d.noise_key_fingerprint.clone(),
        has_noise_key: !d.noise_key.is_empty(),
        platform: d.platform.clone(),
        model: d.model.clone(),
        os_version: d.os_version.clone(),
        enrolled_at: d.enrolled_at.clone(),
        banned: d.banned,
        public_key_banned,
        email: d.email.clone(),
        company_id: d.company_id.clone(),
        app_version: d.app_version.clone(),
        manufacturer: d.manufacturer.clone(),
        brand: d.brand.clone(),
        device_name: d.device_name.clone(),
        details_json: d.details_json.clone(),
        security_patch: detail_field(&d.details_json, &["security_patch"]),
        api_level: detail_field(&d.details_json, &["api_level", "sdk"]),
        build_display: detail_field(
            &d.details_json,
            &["build_display", "android_build"],
        ),
        build_type: detail_field(&d.details_json, &["build_type"]),
        android_release: detail_field(&d.details_json, &["android_release"]),
        physical: detail_field(&d.details_json, &["physical"]),
        low_ram: detail_field(&d.details_json, &["low_ram"]),
        last_seen_at: d.last_seen_at.clone(),
        rate_limit_bypass: d.rate_limit_bypass,
        key_history: d.key_history.clone(),
    }
}

fn history_entry_from_device(
    d: &DeviceRecord,
    reason: &str,
    from_device_id: &str,
) -> DeviceKeyHistoryEntry {
    DeviceKeyHistoryEntry {
        replaced_at: now_secs_str(),
        reason: reason.to_string(),
        public_key: normalize_pk(&d.public_key),
        noise_key_fingerprint: d.noise_key_fingerprint.clone(),
        platform: d.platform.clone(),
        model: d.model.clone(),
        app_version: d.app_version.clone(),
        enrolled_at: d.enrolled_at.clone(),
        last_seen_at: d.last_seen_at.clone(),
        from_device_id: from_device_id.to_string(),
    }
}

fn encode_key_history(history: &[DeviceKeyHistoryEntry]) -> String {
    serde_json::to_string(history).unwrap_or_else(|_| "[]".into())
}

fn decode_key_history(raw: &str) -> Vec<DeviceKeyHistoryEntry> {
    if raw.trim().is_empty() {
        return Vec::new();
    }
    serde_json::from_str(raw).unwrap_or_default()
}

/// Merge header-provided device detail keys into `details_json`.
/// Returns `Some(new_json)` only when at least one value changed.
fn merge_device_details_json(existing: &str, updates: &[(&str, &str)]) -> Option<String> {
    if updates.iter().all(|(_, v)| v.trim().is_empty()) {
        return None;
    }
    let mut map: serde_json::Map<String, serde_json::Value> = if existing.trim().is_empty() {
        serde_json::Map::new()
    } else {
        match serde_json::from_str::<serde_json::Value>(existing) {
            Ok(serde_json::Value::Object(m)) => m,
            _ => serde_json::Map::new(),
        }
    };
    let mut changed = false;
    for (key, value) in updates {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        let prev = map.get(*key).and_then(|v| v.as_str()).unwrap_or("");
        if prev != value {
            map.insert(
                (*key).to_string(),
                serde_json::Value::String(value.to_string()),
            );
            changed = true;
        }
    }
    if !changed {
        return None;
    }
    Some(serde_json::Value::Object(map).to_string())
}

fn now_secs_str() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_default()
}

fn load_devices(dir: &Path) -> anyhow::Result<HashMap<Uuid, DeviceRecord>> {
    let mut map = HashMap::new();
    if !dir.exists() {
        return Ok(map);
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        if let Ok(bytes) = fs::read(&path) {
            if let Ok(mut rec) = serde_json::from_slice::<DeviceRecord>(&bytes) {
                if rec.noise_key_fingerprint.is_empty() && !rec.noise_key.is_empty() {
                    if let Ok(bytes) = hex::decode(&rec.noise_key) {
                        rec.noise_key_fingerprint = fingerprint_key(&bytes);
                    }
                }
                map.insert(rec.device_id, rec);
            }
        }
    }
    Ok(map)
}

fn persist_device(db: &Db, dir: &Path, record: &DeviceRecord) -> anyhow::Result<()> {
    db.device_upsert(&DeviceRow {
        device_id: record.device_id.to_string(),
        case_id: record.case_id.to_string(),
        public_key: record.public_key.clone(),
        noise_key: record.noise_key.clone(),
        noise_key_fingerprint: record.noise_key_fingerprint.clone(),
        platform: record.platform.clone(),
        model: record.model.clone(),
        os_version: record.os_version.clone(),
        enrolled_at: record.enrolled_at.clone(),
        banned: record.banned,
        email: record.email.clone(),
        company_id: record.company_id.clone(),
        app_version: record.app_version.clone(),
        manufacturer: record.manufacturer.clone(),
        brand: record.brand.clone(),
        device_name: record.device_name.clone(),
        details_json: record.details_json.clone(),
        last_seen_at: record.last_seen_at.clone(),
        rate_limit_bypass: record.rate_limit_bypass,
        key_history_json: encode_key_history(&record.key_history),
    })?;
    fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.json", record.device_id));
    fs::write(path, serde_json::to_vec_pretty(record)?)?;
    Ok(())
}

fn load_devices_from_db(db: &Db) -> anyhow::Result<Option<HashMap<Uuid, DeviceRecord>>> {
    let rows = db.device_list()?;
    if rows.is_empty() {
        return Ok(None);
    }
    let mut map = HashMap::new();
    for r in rows {
        let Ok(device_id) = Uuid::parse_str(&r.device_id) else {
            continue;
        };
        let case_id = Uuid::parse_str(&r.case_id).unwrap_or(Uuid::nil());
        map.insert(
            device_id,
            DeviceRecord {
                device_id,
                case_id,
                public_key: r.public_key,
                noise_key: r.noise_key,
                noise_key_fingerprint: r.noise_key_fingerprint,
                platform: r.platform,
                model: r.model,
                os_version: r.os_version,
                enrolled_at: r.enrolled_at,
                banned: r.banned,
                email: r.email,
                company_id: r.company_id,
                app_version: r.app_version,
                manufacturer: r.manufacturer,
                brand: r.brand,
                device_name: r.device_name,
                details_json: r.details_json,
                last_seen_at: r.last_seen_at,
                rate_limit_bypass: r.rate_limit_bypass,
                key_history: decode_key_history(&r.key_history_json),
            },
        );
    }
    Ok(Some(map))
}

fn load_banned_from_db(db: &Db) -> anyhow::Result<Option<HashMap<String, BannedKeyRecord>>> {
    let rows = db.banned_list()?;
    if rows.is_empty() {
        return Ok(None);
    }
    let mut map = HashMap::new();
    for r in rows {
        map.insert(
            normalize_pk(&r.public_key),
            BannedKeyRecord {
                public_key: r.public_key,
                banned_at: r.banned_at,
                reason: r.reason,
            },
        );
    }
    Ok(Some(map))
}

fn load_banned(path: &Path) -> anyhow::Result<HashMap<String, BannedKeyRecord>> {
    let mut map = HashMap::new();
    if !path.exists() {
        return Ok(map);
    }
    let bytes = fs::read(path)?;
    let list: Vec<BannedKeyRecord> = serde_json::from_slice(&bytes).unwrap_or_default();
    for rec in list {
        map.insert(normalize_pk(&rec.public_key), rec);
    }
    Ok(map)
}

fn persist_banned(
    db: &Db,
    path: &Path,
    map: &HashMap<String, BannedKeyRecord>,
) -> anyhow::Result<()> {
    for rec in map.values() {
        db.banned_upsert(&rec.public_key, &rec.banned_at, &rec.reason)?;
    }
    let list: Vec<_> = map.values().cloned().collect();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(&list)?)?;
    Ok(())
}

fn load_or_create_case_keys(
    keys_dir: &Path,
    case_pk: Option<&Path>,
    case_sk: Option<&Path>,
) -> anyhow::Result<(String, String)> {
    let pk_path = case_pk
        .map(Path::to_path_buf)
        .unwrap_or_else(|| keys_dir.join("case.pk.hex"));
    let sk_path = case_sk
        .map(Path::to_path_buf)
        .unwrap_or_else(|| keys_dir.join("case.sk.hex"));

    if pk_path.exists() && sk_path.exists() {
        return Ok((
            fs::read_to_string(&pk_path)?.trim().to_string(),
            fs::read_to_string(&sk_path)?.trim().to_string(),
        ));
    }

    if case_pk.is_some() || case_sk.is_some() {
        anyhow::bail!(
            "case key files not found: {} / {}",
            pk_path.display(),
            sk_path.display()
        );
    }

    let kp = CaseKeyPair::generate();
    let pk = hex::encode(kp.public_bytes());
    let sk = hex::encode(kp.private_bytes());
    fs::write(&pk_path, &pk)?;
    fs::write(&sk_path, &sk)?;
    tracing::info!(path = %pk_path.display(), "generated case encryption keypair");
    Ok((pk, sk))
}

pub async fn enroll(
    State(state): State<AppState>,
    Json(req): Json<EnrollRequest>,
) -> Result<(StatusCode, Json<EnrollResponse>), (StatusCode, Json<serde_json::Value>)> {
    let token_hint = {
        let t = req.token.trim();
        if t.len() <= 4 {
            "••••".to_string()
        } else {
            format!("{}…{}", &t[..2], &t[t.len().saturating_sub(2)..])
        }
    };
    match state.enrollment.enroll(req) {
        Ok(res) => {
            state.otp.audit(
                "enroll_ok",
                None,
                None,
                None,
                Some(&res.device_id.to_string()),
                None,
                Some(&format!("enrolled_via_token token={token_hint}")),
            );
            Ok((StatusCode::CREATED, Json(res)))
        }
        Err(e) => {
            let reason = if e.contains("banned") {
                "banned_public_key"
            } else if e.contains("token") {
                "invalid_token"
            } else if e.contains("maximum") || e.contains("already enrolled") {
                "device_slot_full"
            } else {
                "enroll_rejected"
            };
            state.otp.audit(
                "enroll_token_fail",
                None,
                None,
                None,
                None,
                None,
                Some(&format!("{reason}: {e} token={token_hint}")),
            );
            let status = if e.contains("banned") {
                StatusCode::FORBIDDEN
            } else {
                StatusCode::BAD_REQUEST
            };
            Err((status, Json(serde_json::json!({ "error": e }))))
        }
    }
}

pub async fn manifest(State(state): State<AppState>) -> Json<ManifestResponse> {
    Json(state.enrollment.manifest())
}

pub async fn requests(State(state): State<AppState>) -> Json<Vec<CollectionRequestDto>> {
    Json(state.enrollment.requests())
}

pub async fn post_message(
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    tracing::info!(fields = body.as_object().map_or(0, serde_json::Map::len), "message report received (stub)");
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "status": "accepted" })),
    )
}
