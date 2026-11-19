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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrollResponse {
    pub device_id: Uuid,
    pub case_id: Uuid,
    pub case_public_key: String,
    pub server_base_url: String,
    pub display_name: String,
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
}

impl EnrollmentStore {
    pub fn open(data_dir: &Path, db: Arc<Db>, opts: EnrollmentOptions) -> anyhow::Result<Self> {
        let mut tokens = HashMap::new();
        let mut reusable = HashSet::new();
        let demo_case = Uuid::parse_str("00000000-0000-4000-8000-000000000001").unwrap();
        tokens.insert("demo".into(), demo_case);
        tokens.insert("DEVTOKEN1".into(), demo_case);
        reusable.insert("demo".into());
        reusable.insert("DEVTOKEN1".into());
        let seeded_at = now_secs_str();
        let mut token_created_at = HashMap::new();
        token_created_at.insert("demo".into(), seeded_at.clone());
        token_created_at.insert("DEVTOKEN1".into(), seeded_at);

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
        self.banned_public_keys.lock().unwrap().contains_key(&key)
            || self
                .devices
                .lock()
                .unwrap()
                .values()
                .any(|d| normalize_pk(&d.public_key) == key && d.banned)
    }

    pub fn list_devices(&self) -> Vec<DevicePublicView> {
        let banned = self.banned_public_keys.lock().unwrap();
        let mut list: Vec<_> = self
            .devices
            .lock()
            .unwrap()
            .values()
            .map(|d| DevicePublicView {
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
                public_key_banned: banned.contains_key(&normalize_pk(&d.public_key)),
                email: d.email.clone(),
                company_id: d.company_id.clone(),
            })
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
                    "email": d.email,
                    "company_id": d.company_id,
                    "enrolled_at": d.enrolled_at,
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

        self.enroll_verified(
            case_id,
            req.device_public_key,
            req.noise_key,
            req.platform,
            req.model,
            req.os_version,
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
        platform: String,
        model: String,
        os_version: String,
        email: String,
        company_id: String,
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

        let device_id = Uuid::new_v4();
        let record = DeviceRecord {
            device_id,
            case_id,
            public_key: pk.clone(),
            noise_key: noise_hex,
            noise_key_fingerprint: noise_fp.clone(),
            platform,
            model,
            os_version,
            enrolled_at: now_secs_str(),
            banned: false,
            email,
            company_id,
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
            enabled_collectors: vec![
                "android_bugreport".into(),
                "ios_sysdiagnose".into(),
                "message_report".into(),
            ],
            noise_key_fingerprint: noise_fp,
        })
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
    match state.enrollment.enroll(req) {
        Ok(res) => Ok((StatusCode::CREATED, Json(res))),
        Err(e) => {
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
    tracing::info!(?body, "message report received (stub)");
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "status": "accepted" })),
    )
}
