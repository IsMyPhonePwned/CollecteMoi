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
    /// Max active (non-banned) devices per work email.
    /// `1` = one device per email (unique association); `0` = unlimited.
    #[serde(default = "default_max_devices_per_email")]
    pub max_devices_per_email: u32,
    /// New upload sessions allowed per device per rolling hour (`0` = unlimited).
    #[serde(default = "default_device_uploads_per_hour")]
    pub device_uploads_per_hour: u32,
}

fn default_true() -> bool {
    true
}

fn default_max_devices_per_email() -> u32 {
    1
}

fn default_device_uploads_per_hour() -> u32 {
    10
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

/// Internal outcome of [`OtpStore::start`] — includes plaintext code for webhook delivery.
pub struct EnrollStartOutcome {
    pub response: EnrollStartResponse,
    /// Set for email enrollments that should be delivered out-of-band (not lab-key shortcuts).
    pub delivery: Option<crate::otp_delivery::OtpWebhookPayload>,
    pub via_lab_key: bool,
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

impl OtpStore {
    pub fn open(
        data_dir: &Path,
        db: Arc<Db>,
        reveal_otp: bool,
        seed_demo_lab_keys: bool,
    ) -> anyhow::Result<Self> {
        let companies_path = data_dir.join("companies.json");
        let lab_keys_path = data_dir.join("lab_keys.json");
        let log_path = data_dir.join("enrollment_log.jsonl");
        fs::create_dir_all(data_dir)?;

        let demo_case = Uuid::parse_str("00000000-0000-4000-8000-000000000001").unwrap();
        // Demo companies/lab keys seed only on loopback lab binds. Docker (0.0.0.0) starts empty.
        let mut companies = load_companies_from_db(&db)?.unwrap_or_else(|| {
            load_or_seed_companies(&companies_path, demo_case, seed_demo_lab_keys)
                .unwrap_or_default()
        });
        for c in &mut companies {
            c.color = sanitize_company_color(&c.color, &c.id);
        }
        let mut lab_keys = load_lab_keys_from_db(&db)?.unwrap_or_else(|| {
            load_or_seed_lab_keys(&lab_keys_path, demo_case, seed_demo_lab_keys)
                .unwrap_or_default()
        });
        if !seed_demo_lab_keys {
            let default_labels = [
                ("LAB-DEMO01", "Primary lab key"),
                ("LAB-DEMO02", "Secondary lab key"),
                ("LAB-QUICK1", "Quick test 1"),
                ("LAB-QUICK2", "Quick test 2"),
                ("LAB-QUICK3", "Quick test 3"),
            ];
            let now = now_secs().to_string();
            let mut disabled_seed = false;
            for (map_key, key) in &mut lab_keys {
                let is_seed = default_labels
                    .iter()
                    .any(|(seed, _)| map_key.eq_ignore_ascii_case(seed));
                if is_seed && key.enabled {
                    key.enabled = false;
                    key.updated_at = now.clone();
                    disabled_seed = true;
                }
            }
            if disabled_seed {
                for key in lab_keys.values() {
                    db.lab_key_upsert(&lab_key_to_row(key))?;
                }
                let list: Vec<_> = lab_keys.values().cloned().collect();
                persist_json(&lab_keys_path, &list)?;
                tracing::warn!("disabled well-known demo lab keys on a non-loopback listener");
            }
        }
        // Persist whatever we loaded (or seeded) into SQL when files were the source.
        for c in &companies {
            let _ = db.company_upsert(&company_to_row(c));
        }
        for k in lab_keys.values() {
            let _ = db.lab_key_upsert(&lab_key_to_row(k));
        }
        if companies.is_empty() && seed_demo_lab_keys {
            companies = load_or_seed_companies(&companies_path, demo_case, true)?;
            for c in &companies {
                let _ = db.company_upsert(&company_to_row(c));
            }
        }
        if lab_keys.is_empty() {
            lab_keys = load_or_seed_lab_keys(&lab_keys_path, demo_case, seed_demo_lab_keys)?;
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
                        lab_key: r.lab_key.map(|key| mask_lab_key(&key)),
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
        if company.max_devices_per_email > 1000 {
            return Err("max_devices_per_email must be between 0 and 1000".into());
        }
        if company.device_uploads_per_hour > 10_000 {
            return Err("device_uploads_per_hour must be between 0 and 10000".into());
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

    /// Append a structured enrollment audit event (SQLite + JSONL + tracing).
    pub fn audit(
        &self,
        event: &str,
        email: Option<&str>,
        company_id: Option<&str>,
        challenge_id: Option<&str>,
        device_id: Option<&str>,
        lab_key: Option<&str>,
        detail: Option<&str>,
    ) {
        self.log(EnrollmentLogEvent {
            ts: now_secs(),
            event: event.into(),
            email: email.map(|s| s.to_string()),
            company_id: company_id.map(|s| s.to_string()),
            challenge_id: challenge_id.map(|s| s.to_string()),
            device_id: device_id.map(|s| s.to_string()),
            lab_key: lab_key.map(|s| s.to_string()),
            detail: detail.map(|s| s.to_string()),
        });
    }

    fn log(&self, event: EnrollmentLogEvent) {
        let mut event = event;
        event.lab_key = event.lab_key.map(|key| mask_lab_key(&key));
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
        let fail = event.event.contains("fail") || event.event.contains("error");
        if fail {
            tracing::warn!(
                event = %event.event,
                email = ?event.email,
                company = ?event.company_id,
                challenge = ?event.challenge_id,
                device = ?event.device_id,
                lab_key = ?event.lab_key,
                detail = ?event.detail,
                "enrollment"
            );
        } else {
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
    }

    pub fn drop_challenge(&self, challenge_id: Uuid) {
        self.challenges.lock().unwrap().remove(&challenge_id);
    }

    pub fn start<F>(
        &self,
        req: EnrollStartRequest,
        slot_ok: F,
    ) -> Result<EnrollStartOutcome, String>
    where
        F: FnOnce(&str, u32) -> Result<(), String>,
    {
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
                    max_devices_per_email: 0,
                    device_uploads_per_hour: 10,
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

        // Reject before issuing an OTP when this email is already at capacity.
        slot_ok(&email, company.max_devices_per_email)?;

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
            event: "otp_start_ok".into(),
            email: Some(email.clone()),
            company_id: Some(company.id.clone()),
            challenge_id: Some(challenge_id.to_string()),
            device_id: None,
            lab_key: via_lab_key.clone(),
            detail: Some(format!(
                "otp_issued ttl={OTP_TTL_SECS}s{}",
                if accept_lab_as_code.is_some() {
                    "; lab key also accepted as code"
                } else {
                    ""
                }
            )),
        });

        let reveal = self.reveal_otp || via_lab_key.is_some();
        let via_lab = via_lab_key.is_some();
        let email_masked = mask_email(&email);
        let delivery = if via_lab {
            None
        } else {
            Some(crate::otp_delivery::OtpWebhookPayload {
                event: "enroll_otp",
                email: email.clone(),
                email_masked: email_masked.clone(),
                code: code.clone(),
                challenge_id,
                company_id: company.id.clone(),
                company_name: company.name.clone(),
                expires_in_secs: OTP_TTL_SECS,
                expires_at: expires_at,
                subject: String::new(),
                body: String::new(),
            })
        };
        Ok(EnrollStartOutcome {
            response: EnrollStartResponse {
                challenge_id,
                email: email.clone(),
                email_masked,
                company_id: company.id.clone(),
                company_name: company.name.clone(),
                expires_in_secs: OTP_TTL_SECS,
                message: if via_lab {
                    "Lab key accepted. Use the returned lab_otp (or the lab key itself) to confirm."
                        .into()
                } else {
                    // Refined by the HTTP handler once webhook delivery succeeds/fails.
                    "Verification code issued.".into()
                },
                lab_otp: reveal.then_some(code),
            },
            delivery,
            via_lab_key: via_lab,
        })
    }

    pub fn confirm(
        &self,
        req: EnrollConfirmRequest,
        enroll_fn: impl FnOnce(ConfirmEnrollInput) -> Result<EnrollResponse, String>,
    ) -> Result<(EnrollResponse, String), String> {
        let now = now_secs();
        let mut challenges = self.challenges.lock().unwrap();
        let Some(ch) = challenges.get_mut(&req.challenge_id) else {
            self.log(EnrollmentLogEvent {
                ts: now,
                event: "otp_confirm_fail".into(),
                email: None,
                company_id: None,
                challenge_id: Some(req.challenge_id.to_string()),
                device_id: None,
                lab_key: None,
                detail: Some("unknown_challenge".into()),
            });
            return Err("unknown or expired challenge".into());
        };

        if ch.used {
            self.log(EnrollmentLogEvent {
                ts: now,
                event: "otp_confirm_fail".into(),
                email: Some(ch.email.clone()),
                company_id: Some(ch.company_id.clone()),
                challenge_id: Some(ch.id.to_string()),
                device_id: None,
                lab_key: ch.via_lab_key.clone(),
                detail: Some("already_used".into()),
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
        if !(hash_ok || lab_ok) {
            self.log(EnrollmentLogEvent {
                ts: now,
                event: "otp_confirm_fail".into(),
                email: Some(ch.email.clone()),
                company_id: Some(ch.company_id.clone()),
                challenge_id: Some(ch.id.to_string()),
                device_id: None,
                lab_key: ch.via_lab_key.clone(),
                detail: Some("invalid_code".into()),
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
                    self.log(EnrollmentLogEvent {
                        ts: now,
                        event: "lab_key_consumed".into(),
                        email: Some(email.clone()),
                        company_id: Some(company_id.clone()),
                        challenge_id: Some(challenge_id.to_string()),
                        device_id: None,
                        lab_key: Some(lk.clone()),
                        detail: Some("one_shot_disabled".into()),
                    });
                }
            }
        }

        let res = match enroll_fn(ConfirmEnrollInput {
            case_id,
            email: email.clone(),
            company_id: company_id.clone(),
            device_public_key: req.device_public_key,
            noise_key: req.noise_key,
            platform: req.platform,
            model: req.model,
            os_version: req.os_version,
            app_version: req.app_version,
            manufacturer: req.manufacturer,
            brand: req.brand,
            name: req.name,
            details: req.details,
        }) {
            Ok(r) => r,
            Err(e) => {
                let reason = classify_enroll_fail(&e);
                self.log(EnrollmentLogEvent {
                    ts: now,
                    event: "enroll_fail".into(),
                    email: Some(email),
                    company_id: Some(company_id),
                    challenge_id: Some(challenge_id.to_string()),
                    device_id: None,
                    lab_key,
                    detail: Some(format!("{reason}: {e}")),
                });
                return Err(e);
            }
        };

        self.log(EnrollmentLogEvent {
            ts: now,
            event: "enroll_ok".into(),
            email: Some(email.clone()),
            company_id: Some(company_id),
            challenge_id: Some(challenge_id.to_string()),
            device_id: Some(res.device_id.to_string()),
            lab_key,
            detail: Some("enrolled_via_otp".into()),
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

fn mask_lab_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 4 {
        return "•".repeat(chars.len());
    }
    let prefix: String = chars.iter().take(2).copied().collect();
    let suffix: String = chars[chars.len() - 2..].iter().copied().collect();
    format!("{prefix}…{suffix}")
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
    pub app_version: String,
    pub manufacturer: String,
    pub brand: String,
    pub name: String,
    pub details: serde_json::Value,
}

fn load_or_seed_companies(
    path: &Path,
    demo_case: Uuid,
    seed_demo: bool,
) -> anyhow::Result<Vec<Company>> {
    if path.exists() {
        let bytes = fs::read(path)?;
        if let Ok(list) = serde_json::from_slice::<Vec<Company>>(&bytes) {
            if !list.is_empty() {
                return Ok(list);
            }
        }
    }
    if !seed_demo {
        return Ok(Vec::new());
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
            // Lab testing often reuses the same mailbox across phones.
            max_devices_per_email: 0,
            device_uploads_per_hour: 0,
        },
        Company {
            id: "demo-corp".into(),
            name: "Demo Corp".into(),
            domains: vec!["demo.corp".into()],
            case_id: demo_case,
            enabled: true,
            updated_at: seeded_at,
            color: "#3B82F6".into(),
            max_devices_per_email: 1,
            device_uploads_per_hour: 10,
        },
    ];
    persist_json(path, &seeded)?;
    Ok(seeded)
}

fn load_or_seed_lab_keys(
    path: &Path,
    demo_case: Uuid,
    seed_demo_lab_keys: bool,
) -> anyhow::Result<HashMap<String, LabKey>> {
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
    if map.is_empty() && seed_demo_lab_keys {
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
        max_devices_per_email: c.max_devices_per_email,
        device_uploads_per_hour: c.device_uploads_per_hour,
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
                    max_devices_per_email: r.max_devices_per_email,
                    device_uploads_per_hour: r.device_uploads_per_hour,
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
    for event in &mut events {
        event.lab_key = event.lab_key.take().map(|key| mask_lab_key(&key));
    }
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

fn classify_enroll_fail(err: &str) -> &'static str {
    let e = err.to_ascii_lowercase();
    if e.contains("banned") {
        "banned_public_key"
    } else if e.contains("maximum") || e.contains("already enrolled") {
        "device_slot_full"
    } else if e.contains("noise") {
        "invalid_noise_key"
    } else if e.contains("public_key") || e.contains("hex") {
        "invalid_public_key"
    } else {
        "enroll_rejected"
    }
}

fn classify_otp_start_fail(err: &str) -> &'static str {
    let e = err.to_ascii_lowercase();
    if e.contains("unknown lab key") {
        "unknown_lab_key"
    } else if e.contains("lab key disabled") {
        "lab_key_disabled"
    } else if e.contains("company disabled") {
        "company_disabled"
    } else if e.contains("company") && e.contains("not found") {
        "lab_key_company_missing"
    } else if e.contains("domain not allowed") {
        "domain_not_allowed"
    } else if e.contains("email") {
        "invalid_email"
    } else if e.contains("maximum") || e.contains("already enrolled") {
        "device_slot_full"
    } else {
        "start_rejected"
    }
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
    let enrollment = state.enrollment.clone();
    let req_email = req
        .email
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let req_lab = req
        .lab_key
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_uppercase());
    let outcome = match state.otp.start(req, |email, max| {
        enrollment.assert_email_device_slot(email, max, None)
    }) {
        Ok(o) => o,
        Err(e) => {
            let reason = classify_otp_start_fail(&e);
            state.otp.audit(
                "otp_start_fail",
                req_email.as_deref(),
                None,
                None,
                None,
                req_lab.as_deref(),
                Some(&format!("{reason}: {e}")),
            );
            let status = if e.contains("maximum") || e.contains("already enrolled") {
                StatusCode::CONFLICT
            } else {
                StatusCode::BAD_REQUEST
            };
            return Err((status, Json(serde_json::json!({ "error": e }))));
        }
    };

    let mut res = outcome.response;
    if let Some(mut payload) = outcome.delivery {
        let settings = state.settings.lock().unwrap().clone();
        let (_, display_name) = crate::branding::company_lab_identity(
            &state.data_dir,
            &payload.company_id,
            &payload.company_name,
            &settings.display_name,
        );
        let (subject, body) = crate::branding::render_otp_notification(
            &state.data_dir,
            &payload.company_id,
            &payload.company_name,
            &display_name,
            &payload.email,
            &payload.code,
            payload.expires_in_secs,
        );
        payload.subject = subject;
        payload.body = body;
        if settings.otp_webhook_configured() {
            match crate::otp_delivery::deliver_enroll_otp(&settings, &payload).await {
                Ok(true) => {
                    state.otp.audit(
                        "otp_delivery_ok",
                        Some(&res.email),
                        Some(&res.company_id),
                        Some(&res.challenge_id.to_string()),
                        None,
                        None,
                        Some("webhook_delivered"),
                    );
                    res.message =
                        "Verification code sent. Check your work email for the one-time code."
                            .into();
                }
                Ok(false) => {
                    // Should not happen when configured; treat as misconfig.
                    state.otp.drop_challenge(res.challenge_id);
                    state.otp.audit(
                        "otp_delivery_fail",
                        Some(&res.email),
                        Some(&res.company_id),
                        Some(&res.challenge_id.to_string()),
                        None,
                        None,
                        Some("webhook_not_configured"),
                    );
                    return Err((
                        StatusCode::BAD_GATEWAY,
                        Json(serde_json::json!({
                            "error": "otp delivery webhook is not configured"
                        })),
                    ));
                }
                Err(e) => {
                    state.otp.drop_challenge(res.challenge_id);
                    state.otp.audit(
                        "otp_delivery_fail",
                        Some(&res.email),
                        Some(&res.company_id),
                        Some(&res.challenge_id.to_string()),
                        None,
                        None,
                        Some(&format!("webhook_error: {e}")),
                    );
                    return Err((
                        StatusCode::BAD_GATEWAY,
                        Json(serde_json::json!({
                            "error": format!("could not deliver verification code: {e}")
                        })),
                    ));
                }
            }
        } else if outcome.via_lab_key {
            // unreachable — delivery is None for lab keys
        } else if res.lab_otp.is_some() {
            state.otp.audit(
                "otp_delivery_ok",
                Some(&res.email),
                Some(&res.company_id),
                Some(&res.challenge_id.to_string()),
                None,
                None,
                Some("lab_reveal"),
            );
            res.message =
                "Verification code issued (lab reveal). Use lab_otp from this response.".into();
        } else {
            state.otp.audit(
                "otp_delivery_fail",
                Some(&res.email),
                Some(&res.company_id),
                Some(&res.challenge_id.to_string()),
                None,
                None,
                Some("no_webhook_configured"),
            );
            res.message =
                "Verification code issued. Configure an OTP delivery webhook to email codes."
                    .into();
        }
    }

    Ok((StatusCode::OK, Json(res)))
}

pub async fn enroll_confirm(
    State(state): State<AppState>,
    Json(req): Json<EnrollConfirmRequest>,
) -> Result<(StatusCode, Json<EnrollResponse>), (StatusCode, Json<serde_json::Value>)> {
    let enrollment = state.enrollment.clone();
    let otp = state.otp.clone();
    let data_dir = state.data_dir.clone();
    let global_display_name = state.settings.lock().unwrap().display_name.clone();
    match state.otp.confirm(req, |input| {
        let company = otp.get_company(&input.company_id);
        let company_name = company
            .as_ref()
            .map(|c| c.name.clone())
            .unwrap_or_else(|| input.company_id.clone());
        let max = company
            .as_ref()
            .map(|c| c.max_devices_per_email)
            .unwrap_or(1);
        let (company_name, display_name) = crate::branding::company_lab_identity(
            &data_dir,
            &input.company_id,
            &company_name,
            &global_display_name,
        );
        enrollment
            .enroll_verified_limited(
                input.case_id,
                input.device_public_key,
                input.noise_key,
                crate::enroll::DeviceClientMeta::from_enroll_fields(
                    input.platform,
                    input.model,
                    input.os_version,
                    input.app_version,
                    input.manufacturer,
                    input.brand,
                    input.name,
                    input.details,
                ),
                input.email,
                input.company_id.clone(),
                max,
            )
            .map(|mut res| {
                res.company_id = input.company_id;
                res.company_name = company_name;
                res.display_name = display_name;
                res
            })
    }) {
        Ok((res, _)) => Ok((StatusCode::CREATED, Json(res))),
        Err(e) => {
            let status = if e.contains("banned") {
                StatusCode::FORBIDDEN
            } else if e.contains("maximum") || e.contains("already enrolled") {
                StatusCode::CONFLICT
            } else {
                StatusCode::BAD_REQUEST
            };
            Err((status, Json(serde_json::json!({ "error": e }))))
        }
    }
}
