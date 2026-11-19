//! Email OTP enrollment: start → confirm, company domains, lab keys, audit log.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{Json, extract::State, http::StatusCode};
use collectemoi_db::{CompanyRow, Db, EnrollmentEventRow, LabKeyRow};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::AppState;
use crate::enroll::EnrollResponse;

const OTP_TTL_SECS: u64 = 600; // 10 minutes

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Company {
    pub id: String,
    pub name: String,
    pub domains: Vec<String>,
    pub case_id: Uuid,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Unix seconds when last saved (admin).
    #[serde(default)]
    pub updated_at: String,
    /// Label color `#RRGGBB` used in the admin console.
    #[serde(default)]
    pub color: String,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabKey {
    /// Unique lab testing key (e.g. LAB-DEMO01). Accepted at start and/or as OTP.
    pub key: String,
    #[serde(default)]
    pub label: String,
    /// Optional email stamped on the enrolled device.
    #[serde(default)]
    pub email: String,
    pub case_id: Uuid,
    #[serde(default = "default_true")]
    pub reusable: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Unix seconds when last saved (admin).
    #[serde(default)]
    pub updated_at: String,
    /// When set, enrollments via this key stamp this company (and use its case_id if empty).
    #[serde(default)]
    pub company_id: String,
}

#[derive(Debug, Clone)]
struct Challenge {
    id: Uuid,
    email: String,
    company_id: String,
    case_id: Uuid,
    code_hash: String,
    /// Plain code kept only until expiry for lab reveal / logs (not returned after start except lab_otp).
    code_plain: String,
    expires_at: u64,
    used: bool,
    via_lab_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrollmentLogEvent {
    pub ts: u64,
    pub event: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub company_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub challenge_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lab_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

pub struct OtpStore {
    db: Arc<Db>,
    companies: Mutex<Vec<Company>>,
    lab_keys: Mutex<HashMap<String, LabKey>>,
    challenges: Mutex<HashMap<Uuid, Challenge>>,
    companies_path: PathBuf,
    lab_keys_path: PathBuf,
    log_path: PathBuf,
    /// When true, /enroll/start returns `lab_otp` (lab/dev convenience).
    pub reveal_otp: bool,
}

#[derive(Debug, Deserialize)]
pub struct EnrollStartRequest {
    /// Work email (must match a company domain).
    pub email: Option<String>,
    /// Lab shortcut key (skips mailbox; still creates a challenge).
    pub lab_key: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct EnrollStartResponse {
    pub challenge_id: Uuid,
    pub email: String,
    pub email_masked: String,
    pub company_id: String,
    pub company_name: String,
    pub expires_in_secs: u64,
    pub message: String,
    /// Present only when lab reveal is enabled (or start used a lab key).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lab_otp: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct EnrollConfirmRequest {
    pub challenge_id: Uuid,
    pub code: String,
    pub device_public_key: String,
    pub noise_key: String,
    pub platform: String,
    pub model: String,
    pub os_version: String,
}

impl OtpStore {
    pub fn open(data_dir: &Path, db: Arc<Db>, reveal_otp: bool) -> anyhow::Result<Self> {
        let companies_path = data_dir.join("companies.json");
        let lab_keys_path = data_dir.join("lab_keys.json");
        let log_path = data_dir.join("enrollment_log.jsonl");
        fs::create_dir_all(data_dir)?;

        let demo_case = Uuid::parse_str("00000000-0000-4000-8000-000000000001").unwrap();
        let mut companies = load_companies_from_db(&db)?.unwrap_or_else(|| {
            load_or_seed_companies(&companies_path, demo_case).unwrap_or_default()
        });
        for c in &mut companies {
            c.color = sanitize_company_color(&c.color, &c.id);
        }
        let mut lab_keys = load_lab_keys_from_db(&db)?.unwrap_or_else(|| {
            load_or_seed_lab_keys(&lab_keys_path, demo_case).unwrap_or_default()
        });
        // Ensure seeds exist in SQL when DB was empty and files were seeded.
        for c in &companies {
            let _ = db.company_upsert(&company_to_row(c));
        }
        for k in lab_keys.values() {
            let _ = db.lab_key_upsert(&lab_key_to_row(k));
        }
        if companies.is_empty() {
            companies = load_or_seed_companies(&companies_path, demo_case)?;
            for c in &companies {
                let _ = db.company_upsert(&company_to_row(c));
            }
        }
        if lab_keys.is_empty() {
            lab_keys = load_or_seed_lab_keys(&lab_keys_path, demo_case)?;
            for k in lab_keys.values() {
                let _ = db.lab_key_upsert(&lab_key_to_row(k));
            }
        }

        Ok(Self {
            db,
            companies: Mutex::new(companies),
            lab_keys: Mutex::new(lab_keys),
            challenges: Mutex::new(HashMap::new()),
            companies_path,
            lab_keys_path,
            log_path,
            reveal_otp,
        })
    }

    /// Reload companies + lab keys from SQLite (after backup import).
    pub fn reload_from_db(&self) -> anyhow::Result<()> {
        if let Some(companies) = load_companies_from_db(&self.db)? {
            *self.companies.lock().unwrap() = companies;
        }
        if let Some(lab_keys) = load_lab_keys_from_db(&self.db)? {
            *self.lab_keys.lock().unwrap() = lab_keys;
        }
        Ok(())
    }

    pub fn list_companies(&self) -> Vec<Company> {
        self.companies.lock().unwrap().clone()
    }

    pub fn list_lab_keys(&self) -> Vec<LabKey> {
        let mut list: Vec<_> = self.lab_keys.lock().unwrap().values().cloned().collect();
        list.sort_by(|a, b| a.key.cmp(&b.key));
        list
    }

    pub fn list_log(&self, limit: usize) -> Vec<EnrollmentLogEvent> {
        if let Ok(rows) = self.db.enrollment_list(limit) {
            if !rows.is_empty() {
                return rows
                    .into_iter()
                    .map(|r| EnrollmentLogEvent {
                        ts: r.ts,
                        event: r.event,
                        email: r.email,
                        company_id: r.company_id,
                        challenge_id: r.challenge_id,
                        device_id: r.device_id,
                        lab_key: r.lab_key,
                        detail: r.detail,
                    })
                    .collect();
            }
        }
        read_log_tail(&self.log_path, limit)
    }

    pub fn get_company(&self, id: &str) -> Option<Company> {
        let id = id.trim();
        self.companies
            .lock()
            .unwrap()
            .iter()
            .find(|c| c.id == id)
            .cloned()
    }

    pub fn upsert_company(&self, mut company: Company) -> Result<Company, String> {
        if company.id.trim().is_empty() {
            return Err("company id required".into());
        }
        company.id = company.id.trim().to_ascii_lowercase();
        if company.domains.is_empty() {
            return Err("at least one domain required".into());
        }
        company.domains = company
            .domains
            .into_iter()
            .map(|d| d.trim().to_ascii_lowercase())
            .filter(|d| !d.is_empty())
            .collect();
        if company.domains.is_empty() {
            return Err("at least one domain required".into());
        }
        company.color = sanitize_company_color(&company.color, &company.id);
        company.updated_at = now_secs().to_string();
        let mut list = self.companies.lock().unwrap();
        if let Some(existing) = list.iter_mut().find(|c| c.id == company.id) {
            *existing = company.clone();
        } else {
            list.push(company.clone());
        }
        self.db
            .company_upsert(&company_to_row(&company))
            .map_err(|e| e.to_string())?;
        persist_json(&self.companies_path, &*list).map_err(|e| e.to_string())?;
        Ok(company)
    }

    pub fn set_company_enabled(&self, id: &str, enabled: bool) -> Result<Company, String> {
        let mut company = self
            .get_company(id)
            .ok_or_else(|| format!("unknown company '{id}'"))?;
        company.enabled = enabled;
        self.upsert_company(company)
    }

    pub fn delete_company(&self, id: &str) -> Result<(), String> {
        let id = id.trim();
        if id.is_empty() {
            return Err("company id required".into());
        }
        let mut list = self.companies.lock().unwrap();
        let before = list.len();
        list.retain(|c| c.id != id);
        if list.len() == before {
            return Err(format!("unknown company '{id}'"));
        }
        self.db.company_delete(id).map_err(|e| e.to_string())?;
        persist_json(&self.companies_path, &*list).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn upsert_lab_key(&self, key: LabKey) -> Result<LabKey, String> {
        let k = key.key.trim().to_uppercase();
        if k.is_empty() {
            return Err("lab key required".into());
        }
        let mut record = key;
        record.key = k.clone();
        record.company_id = record.company_id.trim().to_string();
        if !record.company_id.is_empty() {
            let company = self
                .get_company(&record.company_id)
                .ok_or_else(|| format!("unknown company '{}'", record.company_id))?;
            if record.case_id == Uuid::nil() {
                record.case_id = company.case_id;
            }
        }
        record.updated_at = now_secs().to_string();
        self.lab_keys.lock().unwrap().insert(k, record.clone());
        self.db
            .lab_key_upsert(&lab_key_to_row(&record))
            .map_err(|e| e.to_string())?;
        let list = self.list_lab_keys();
        persist_json(&self.lab_keys_path, &list).map_err(|e| e.to_string())?;
        Ok(record)
    }

    pub fn delete_lab_key(&self, key: &str) -> Result<(), String> {
        let k = key.trim().to_uppercase();
        if k.is_empty() {
            return Err("lab key required".into());
        }
        let removed = self.lab_keys.lock().unwrap().remove(&k).is_some();
        if !removed {
            return Err(format!("unknown lab key '{k}'"));
        }
        self.db.lab_key_delete(&k).map_err(|e| e.to_string())?;
        let list = self.list_lab_keys();
        persist_json(&self.lab_keys_path, &list).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn list_lab_keys_for_company(&self, company_id: &str) -> Vec<LabKey> {
        let id = company_id.trim();
        let mut list: Vec<_> = self
            .lab_keys
            .lock()
            .unwrap()
            .values()
            .filter(|k| k.company_id == id)
            .cloned()
            .collect();
        list.sort_by(|a, b| a.key.cmp(&b.key));
        list
    }

    fn log(&self, event: EnrollmentLogEvent) {
        let _ = self.db.enrollment_log(&EnrollmentEventRow {
            ts: event.ts,
            event: event.event.clone(),
            email: event.email.clone(),
            company_id: event.company_id.clone(),
            challenge_id: event.challenge_id.clone(),
            device_id: event.device_id.clone(),
            lab_key: event.lab_key.clone(),
            detail: event.detail.clone(),
        });
        if let Ok(mut f) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
        {
            if let Ok(line) = serde_json::to_string(&event) {
                let _ = writeln!(f, "{line}");
            }
        }
        tracing::info!(
            event = %event.event,
            email = ?event.email,
            company = ?event.company_id,
            challenge = ?event.challenge_id,
            device = ?event.device_id,
            lab_key = ?event.lab_key,
            detail = ?event.detail,
            "enrollment"
        );
    }

    pub fn start(&self, req: EnrollStartRequest) -> Result<EnrollStartResponse, String> {
        let now = now_secs();
        let lab_key_raw = req
            .lab_key
            .as_ref()
            .map(|s| s.trim().to_uppercase())
            .filter(|s| !s.is_empty());

        let (email, company, via_lab_key) = if let Some(ref lk) = lab_key_raw {
            let keys = self.lab_keys.lock().unwrap();
            let lab = keys
                .get(lk)
                .cloned()
                .ok_or_else(|| "unknown lab key".to_string())?;
            if !lab.enabled {
                return Err("lab key disabled".into());
            }
            let email = if lab.email.trim().is_empty() {
                format!("{}@lab.collectemoi.local", lk.to_lowercase())
            } else {
                normalize_email(&lab.email)?
            };
            let mut company = if !lab.company_id.trim().is_empty() {
                self.get_company(lab.company_id.trim()).ok_or_else(|| {
                    format!("lab key company '{}' not found", lab.company_id.trim())
                })?
            } else {
                self.find_company_for_email(&email).unwrap_or(Company {
                    id: "lab".into(),
                    name: "CollecteMoi Lab".into(),
                    domains: vec!["lab.collectemoi.local".into()],
                    case_id: lab.case_id,
                    enabled: true,
                    updated_at: now_secs().to_string(),
                    color: default_company_color("lab"),
                })
            };
            if !company.enabled {
                return Err("company disabled".into());
            }
            // Prefer lab key's case_id when set (non-nil).
            if lab.case_id != Uuid::nil() {
                company.case_id = lab.case_id;
            }
            (email, company, Some(lk.clone()))
        } else {
            let email = normalize_email(req.email.as_deref().unwrap_or(""))?;
            let company = self.find_company_for_email(&email).ok_or_else(|| {
                format!(
                    "email domain not allowed for any company ({})",
                    domain_of(&email).unwrap_or_default()
                )
            })?;
            if !company.enabled {
                return Err("company disabled".into());
            }
            (email, company, None)
        };

        let code = generate_otp();
        let challenge_id = Uuid::new_v4();
        let code_hash = hash_otp(challenge_id, &code);
        let expires_at = now + OTP_TTL_SECS;

        {
            let mut challenges = self.challenges.lock().unwrap();
            challenges.retain(|_, c| !c.used && c.expires_at > now);
            challenges.insert(
                challenge_id,
                Challenge {
                    id: challenge_id,
                    email: email.clone(),
                    company_id: company.id.clone(),
                    case_id: company.case_id,
                    code_hash,
                    code_plain: code.clone(),
                    expires_at,
                    used: false,
                    via_lab_key: via_lab_key.clone(),
                },
            );
        }

        // Lab keys also work as the OTP code itself for quick testing.
        let accept_lab_as_code = via_lab_key.clone();

        self.log(EnrollmentLogEvent {
            ts: now,
            event: "otp_start".into(),
            email: Some(email.clone()),
            company_id: Some(company.id.clone()),
            challenge_id: Some(challenge_id.to_string()),
            device_id: None,
            lab_key: via_lab_key.clone(),
            detail: Some(format!(
                "otp_issued ttl={OTP_TTL_SECS}s code={code}{}",
                accept_lab_as_code
                    .as_ref()
                    .map(|k| format!(" (lab key {k} also valid as code)"))
                    .unwrap_or_default()
            )),
        });

        let reveal = self.reveal_otp || via_lab_key.is_some();
        Ok(EnrollStartResponse {
            challenge_id,
            email: email.clone(),
            email_masked: mask_email(&email),
            company_id: company.id.clone(),
            company_name: company.name.clone(),
            expires_in_secs: OTP_TTL_SECS,
            message: if via_lab_key.is_some() {
                "Lab key accepted. Use the returned lab_otp (or the lab key itself) to confirm."
                    .into()
            } else {
                "Verification code sent (lab: see lab_otp / server enrollment log).".into()
            },
            lab_otp: reveal.then_some(code),
        })
    }

    pub fn confirm(
        &self,
        req: EnrollConfirmRequest,
        enroll_fn: impl FnOnce(ConfirmEnrollInput) -> Result<EnrollResponse, String>,
    ) -> Result<(EnrollResponse, String), String> {
        let now = now_secs();
        let mut challenges = self.challenges.lock().unwrap();
        let ch = challenges
            .get_mut(&req.challenge_id)
            .ok_or_else(|| "unknown or expired challenge".to_string())?;

        if ch.used {
            self.log(EnrollmentLogEvent {
                ts: now,
                event: "otp_confirm_fail".into(),
                email: Some(ch.email.clone()),
                company_id: Some(ch.company_id.clone()),
                challenge_id: Some(ch.id.to_string()),
                device_id: None,
                lab_key: ch.via_lab_key.clone(),
                detail: Some("already used".into()),
            });
            return Err("challenge already used".into());
        }
        if ch.expires_at < now {
            self.log(EnrollmentLogEvent {
                ts: now,
                event: "otp_confirm_fail".into(),
                email: Some(ch.email.clone()),
                company_id: Some(ch.company_id.clone()),
                challenge_id: Some(ch.id.to_string()),
                device_id: None,
                lab_key: ch.via_lab_key.clone(),
                detail: Some("expired".into()),
            });
            return Err("challenge expired".into());
        }

        let code = req.code.trim();
        let lab_ok = ch
            .via_lab_key
            .as_ref()
            .map(|k| k.eq_ignore_ascii_case(code))
            .unwrap_or(false);
        let hash_ok = hash_otp(ch.id, code) == ch.code_hash;
        // Also accept the stored plain code (normalization).
        let plain_ok = code == ch.code_plain;

        if !(hash_ok || plain_ok || lab_ok) {
            self.log(EnrollmentLogEvent {
                ts: now,
                event: "otp_confirm_fail".into(),
                email: Some(ch.email.clone()),
                company_id: Some(ch.company_id.clone()),
                challenge_id: Some(ch.id.to_string()),
                device_id: None,
                lab_key: ch.via_lab_key.clone(),
                detail: Some("invalid code".into()),
            });
            return Err("invalid verification code".into());
        }

        ch.used = true;
        let email = ch.email.clone();
        let company_id = ch.company_id.clone();
        let case_id = ch.case_id;
        let lab_key = ch.via_lab_key.clone();
        let challenge_id = ch.id;
        drop(challenges);

        // Consume one-shot lab keys.
        if let Some(ref lk) = lab_key {
            let mut keys = self.lab_keys.lock().unwrap();
            if let Some(k) = keys.get_mut(lk) {
                if !k.reusable {
                    k.enabled = false;
                    let _ = self.db.lab_key_upsert(&lab_key_to_row(k));
                    let list: Vec<_> = keys.values().cloned().collect();
                    let _ = persist_json(&self.lab_keys_path, &list);
                }
            }
        }

        let res = enroll_fn(ConfirmEnrollInput {
            case_id,
            email: email.clone(),
            company_id: company_id.clone(),
            device_public_key: req.device_public_key,
            noise_key: req.noise_key,
            platform: req.platform,
            model: req.model,
            os_version: req.os_version,
        })?;

        self.log(EnrollmentLogEvent {
            ts: now,
            event: "otp_confirm_ok".into(),
            email: Some(email.clone()),
            company_id: Some(company_id),
            challenge_id: Some(challenge_id.to_string()),
            device_id: Some(res.device_id.to_string()),
            lab_key,
            detail: Some("enrolled".into()),
        });

        Ok((res, email))
    }

    fn find_company_for_email(&self, email: &str) -> Option<Company> {
        let domain = domain_of(email)?;
        self.companies
            .lock()
            .unwrap()
            .iter()
            .find(|c| c.enabled && c.domains.iter().any(|d| d.eq_ignore_ascii_case(&domain)))
            .cloned()
    }
}

pub struct ConfirmEnrollInput {
    pub case_id: Uuid,
    pub email: String,
    pub company_id: String,
    pub device_public_key: String,
    pub noise_key: String,
    pub platform: String,
    pub model: String,
    pub os_version: String,
}

fn load_or_seed_companies(path: &Path, demo_case: Uuid) -> anyhow::Result<Vec<Company>> {
    if path.exists() {
        let bytes = fs::read(path)?;
        if let Ok(list) = serde_json::from_slice::<Vec<Company>>(&bytes) {
            if !list.is_empty() {
                return Ok(list);
            }
        }
    }
    let seeded_at = now_secs().to_string();
    let seeded = vec![
        Company {
            id: "lab".into(),
            name: "CollecteMoi Lab".into(),
            domains: vec![
                "lab.collectemoi.local".into(),
                "example.com".into(),
                "acme.test".into(),
            ],
            case_id: demo_case,
            enabled: true,
            updated_at: seeded_at.clone(),
            color: "#2EC4A0".into(),
        },
        Company {
            id: "demo-corp".into(),
            name: "Demo Corp".into(),
            domains: vec!["demo.corp".into()],
            case_id: demo_case,
            enabled: true,
            updated_at: seeded_at,
            color: "#3B82F6".into(),
        },
    ];
    persist_json(path, &seeded)?;
    Ok(seeded)
}

fn load_or_seed_lab_keys(path: &Path, demo_case: Uuid) -> anyhow::Result<HashMap<String, LabKey>> {
    let list: Vec<LabKey> = if path.exists() {
        let bytes = fs::read(path)?;
        serde_json::from_slice(&bytes).unwrap_or_default()
    } else {
        Vec::new()
    };
    let mut map = HashMap::new();
    for k in list {
        map.insert(k.key.to_uppercase(), k);
    }
    if map.is_empty() {
        for (key, label) in [
            ("LAB-DEMO01", "Primary lab key"),
            ("LAB-DEMO02", "Secondary lab key"),
            ("LAB-QUICK1", "Quick test 1"),
            ("LAB-QUICK2", "Quick test 2"),
            ("LAB-QUICK3", "Quick test 3"),
        ] {
            map.insert(
                key.into(),
                LabKey {
                    key: key.into(),
                    label: label.into(),
                    email: format!("{}@lab.collectemoi.local", key.to_lowercase()),
                    case_id: demo_case,
                    reusable: true,
                    enabled: true,
                    updated_at: now_secs().to_string(),
                    company_id: String::new(),
                },
            );
        }
        let list: Vec<_> = map.values().cloned().collect();
        persist_json(path, &list)?;
    }
    Ok(map)
}

fn company_to_row(c: &Company) -> CompanyRow {
    CompanyRow {
        id: c.id.clone(),
        name: c.name.clone(),
        domains: c.domains.clone(),
        case_id: c.case_id.to_string(),
        enabled: c.enabled,
        updated_at: c.updated_at.clone(),
        color: c.color.clone(),
    }
}

const COMPANY_COLOR_PALETTE: &[&str] = &[
    "#2EC4A0", "#3B82F6", "#F59E0B", "#EF4444", "#8B5CF6", "#EC4899", "#14B8A6", "#F97316",
    "#06B6D4", "#84CC16", "#A855F7", "#E11D48",
];

fn default_company_color(id: &str) -> String {
    match id {
        "lab" => return "#2EC4A0".into(),
        "demo-corp" => return "#3B82F6".into(),
        _ => {}
    }
    let mut hash: u32 = 2166136261;
    for b in id.bytes() {
        hash ^= u32::from(b);
        hash = hash.wrapping_mul(16777619);
    }
    COMPANY_COLOR_PALETTE[(hash as usize) % COMPANY_COLOR_PALETTE.len()].to_string()
}

fn sanitize_company_color(raw: &str, company_id: &str) -> String {
    let t = raw.trim();
    if t.len() == 7 && t.starts_with('#') && t.as_bytes()[1..].iter().all(|b| b.is_ascii_hexdigit())
    {
        return t.to_ascii_uppercase();
    }
    // Accept RRGGBB without '#'.
    if t.len() == 6 && t.bytes().all(|b| b.is_ascii_hexdigit()) {
        return format!("#{}", t.to_ascii_uppercase());
    }
    default_company_color(company_id)
}

fn lab_key_to_row(k: &LabKey) -> LabKeyRow {
    LabKeyRow {
        key: k.key.clone(),
        label: k.label.clone(),
        email: k.email.clone(),
        case_id: k.case_id.to_string(),
        reusable: k.reusable,
        enabled: k.enabled,
        updated_at: k.updated_at.clone(),
        company_id: k.company_id.clone(),
    }
}

fn load_companies_from_db(db: &Db) -> anyhow::Result<Option<Vec<Company>>> {
    let rows = db.company_list()?;
    if rows.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        rows.into_iter()
            .map(|r| {
                let id = r.id;
                let color = if r.color.trim().is_empty() {
                    default_company_color(&id)
                } else {
                    sanitize_company_color(&r.color, &id)
                };
                Company {
                    id,
                    name: r.name,
                    domains: r.domains,
                    case_id: Uuid::parse_str(&r.case_id).unwrap_or(Uuid::nil()),
                    enabled: r.enabled,
                    updated_at: r.updated_at,
                    color,
                }
            })
            .collect(),
    ))
}

fn load_lab_keys_from_db(db: &Db) -> anyhow::Result<Option<HashMap<String, LabKey>>> {
    let rows = db.lab_key_list()?;
    if rows.is_empty() {
        return Ok(None);
    }
    let mut map = HashMap::new();
    for r in rows {
        map.insert(
            r.key.clone(),
            LabKey {
                key: r.key,
                label: r.label,
                email: r.email,
                case_id: Uuid::parse_str(&r.case_id).unwrap_or(Uuid::nil()),
                reusable: r.reusable,
                enabled: r.enabled,
                updated_at: r.updated_at,
                company_id: r.company_id,
            },
        );
    }
    Ok(Some(map))
}

fn persist_json<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

fn read_log_tail(path: &Path, limit: usize) -> Vec<EnrollmentLogEvent> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut events: Vec<_> = text
        .lines()
        .filter_map(|l| serde_json::from_str::<EnrollmentLogEvent>(l).ok())
        .collect();
    if events.len() > limit {
        events = events.split_off(events.len() - limit);
    }
    events.reverse(); // newest first
    events
}

fn normalize_email(raw: &str) -> Result<String, String> {
    let e = raw.trim().to_lowercase();
    if e.is_empty() || !e.contains('@') || e.starts_with('@') || e.ends_with('@') {
        return Err("valid email required".into());
    }
    if e.chars().any(|c| c.is_whitespace()) {
        return Err("email must not contain spaces".into());
    }
    Ok(e)
}

fn domain_of(email: &str) -> Option<String> {
    email.split('@').nth(1).map(|s| s.to_lowercase())
}

fn mask_email(email: &str) -> String {
    let Some((user, domain)) = email.split_once('@') else {
        return "***".into();
    };
    let keep = user.chars().next().unwrap_or('*');
    format!("{keep}***@{domain}")
}

fn generate_otp() -> String {
    let n: u32 = rand::thread_rng().gen_range(0..1_000_000);
    format!("{n:06}")
}

fn hash_otp(challenge_id: Uuid, code: &str) -> String {
    let mut h = Sha256::new();
    h.update(challenge_id.as_bytes());
    h.update(b"|");
    h.update(code.trim().as_bytes());
    hex::encode(h.finalize())
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub async fn enroll_start(
    State(state): State<AppState>,
    Json(req): Json<EnrollStartRequest>,
) -> Result<(StatusCode, Json<EnrollStartResponse>), (StatusCode, Json<serde_json::Value>)> {
    match state.otp.start(req) {
        Ok(res) => Ok((StatusCode::OK, Json(res))),
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e })),
        )),
    }
}

pub async fn enroll_confirm(
    State(state): State<AppState>,
    Json(req): Json<EnrollConfirmRequest>,
) -> Result<(StatusCode, Json<EnrollResponse>), (StatusCode, Json<serde_json::Value>)> {
    let enrollment = state.enrollment.clone();
    match state.otp.confirm(req, |input| {
        enrollment.enroll_verified(
            input.case_id,
            input.device_public_key,
            input.noise_key,
            input.platform,
            input.model,
            input.os_version,
            input.email,
            input.company_id,
        )
    }) {
        Ok((res, _)) => Ok((StatusCode::CREATED, Json(res))),
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
