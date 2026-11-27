//! Schema DDL. Table/column names are fixed constants — never derived from user input.

pub const SCHEMA_VERSION: i64 = 8;

pub const DDL: &str = r#"
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS companies (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    domains_json TEXT NOT NULL,
    case_id TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    updated_at TEXT NOT NULL,
    color TEXT NOT NULL DEFAULT '',
    -- Max active (non-banned) devices per work email. 1 = unique email↔device; 0 = unlimited.
    max_devices_per_email INTEGER NOT NULL DEFAULT 1,
    -- New upload sessions per device per rolling hour. 0 = unlimited.
    device_uploads_per_hour INTEGER NOT NULL DEFAULT 10
);

CREATE TABLE IF NOT EXISTS lab_keys (
    key TEXT PRIMARY KEY NOT NULL,
    label TEXT NOT NULL DEFAULT '',
    email TEXT NOT NULL DEFAULT '',
    case_id TEXT NOT NULL,
    reusable INTEGER NOT NULL DEFAULT 1,
    enabled INTEGER NOT NULL DEFAULT 1,
    updated_at TEXT NOT NULL,
    company_id TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS devices (
    device_id TEXT PRIMARY KEY NOT NULL,
    case_id TEXT NOT NULL,
    public_key TEXT NOT NULL,
    noise_key TEXT NOT NULL DEFAULT '',
    noise_key_fingerprint TEXT NOT NULL DEFAULT '',
    platform TEXT NOT NULL DEFAULT '',
    model TEXT NOT NULL DEFAULT '',
    os_version TEXT NOT NULL DEFAULT '',
    enrolled_at TEXT NOT NULL,
    banned INTEGER NOT NULL DEFAULT 0,
    email TEXT NOT NULL DEFAULT '',
    company_id TEXT NOT NULL DEFAULT '',
    app_version TEXT NOT NULL DEFAULT '',
    manufacturer TEXT NOT NULL DEFAULT '',
    brand TEXT NOT NULL DEFAULT '',
    device_name TEXT NOT NULL DEFAULT '',
    details_json TEXT NOT NULL DEFAULT '',
    last_seen_at TEXT NOT NULL DEFAULT '',
    -- When set, this device ignores the company upload-per-hour quota.
    rate_limit_bypass INTEGER NOT NULL DEFAULT 0,
    -- JSON array of previous public keys / noise fingerprints after rebind.
    key_history_json TEXT NOT NULL DEFAULT '[]'
);

CREATE INDEX IF NOT EXISTS idx_devices_public_key ON devices(public_key);
CREATE INDEX IF NOT EXISTS idx_devices_email ON devices(email);

CREATE TABLE IF NOT EXISTS banned_public_keys (
    public_key TEXT PRIMARY KEY NOT NULL,
    banned_at TEXT NOT NULL,
    reason TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS uploads (
    upload_id TEXT PRIMARY KEY NOT NULL,
    case_id TEXT NOT NULL,
    artefact TEXT NOT NULL,
    total_size INTEGER NOT NULL,
    chunk_size INTEGER NOT NULL,
    chunk_count INTEGER NOT NULL,
    ciphertext_sha256 TEXT NOT NULL,
    received_json TEXT NOT NULL,
    complete INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    completed_at TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS idx_uploads_created ON uploads(created_at DESC);

CREATE TABLE IF NOT EXISTS jobs (
    upload_id TEXT PRIMARY KEY NOT NULL,
    case_id TEXT NOT NULL DEFAULT '',
    artefact TEXT NOT NULL,
    envelope_path TEXT NOT NULL,
    device_public_key TEXT NOT NULL,
    queued_at TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    claimed_at TEXT,
    finished_at TEXT,
    error TEXT
);

CREATE INDEX IF NOT EXISTS idx_jobs_status ON jobs(status, queued_at);

CREATE TABLE IF NOT EXISTS blobs (
    blob_id TEXT PRIMARY KEY NOT NULL,
    upload_id TEXT NOT NULL,
    case_id TEXT NOT NULL DEFAULT '',
    artefact TEXT NOT NULL,
    size INTEGER NOT NULL,
    sha256 TEXT NOT NULL,
    created_at TEXT NOT NULL,
    queued_at TEXT NOT NULL DEFAULT '',
    device_public_key TEXT NOT NULL DEFAULT '',
    filename TEXT NOT NULL DEFAULT '',
    content_type TEXT NOT NULL DEFAULT 'application/octet-stream'
);

CREATE INDEX IF NOT EXISTS idx_blobs_created ON blobs(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_blobs_case ON blobs(case_id);
CREATE INDEX IF NOT EXISTS idx_blobs_artefact ON blobs(artefact);

CREATE TABLE IF NOT EXISTS access_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ts TEXT NOT NULL,
    method TEXT NOT NULL,
    path TEXT NOT NULL,
    status INTEGER NOT NULL,
    duration_ms INTEGER NOT NULL,
    remote TEXT NOT NULL DEFAULT '',
    user_agent TEXT NOT NULL DEFAULT '',
    device_id TEXT NOT NULL DEFAULT '',
    kind TEXT NOT NULL DEFAULT '',
    resource_id TEXT NOT NULL DEFAULT '',
    is_admin INTEGER NOT NULL DEFAULT 0,
    admin_user TEXT NOT NULL DEFAULT '',
    app_version TEXT NOT NULL DEFAULT '',
    platform TEXT NOT NULL DEFAULT '',
    model TEXT NOT NULL DEFAULT '',
    os_version TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS idx_access_ts ON access_events(ts DESC);
CREATE INDEX IF NOT EXISTS idx_access_kind ON access_events(kind);
CREATE INDEX IF NOT EXISTS idx_access_resource ON access_events(resource_id);

CREATE TABLE IF NOT EXISTS enrollment_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ts INTEGER NOT NULL,
    event TEXT NOT NULL,
    email TEXT,
    company_id TEXT,
    challenge_id TEXT,
    device_id TEXT,
    lab_key TEXT,
    detail TEXT
);

CREATE INDEX IF NOT EXISTS idx_enrollment_ts ON enrollment_events(ts DESC);

CREATE TABLE IF NOT EXISTS otp_challenges (
    id TEXT PRIMARY KEY NOT NULL,
    email TEXT NOT NULL,
    company_id TEXT NOT NULL,
    case_id TEXT NOT NULL,
    code_hash TEXT NOT NULL,
    code_plain TEXT,
    expires_at INTEGER NOT NULL,
    used INTEGER NOT NULL DEFAULT 0,
    via_lab_key TEXT
);

CREATE TABLE IF NOT EXISTS admin_account (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    username TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    totp_secret TEXT NOT NULL,
    totp_enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS admin_sessions (
    token_hash TEXT PRIMARY KEY NOT NULL,
    stage TEXT NOT NULL DEFAULT 'full',
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    remote TEXT NOT NULL DEFAULT '',
    user_agent TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS idx_admin_sessions_expires ON admin_sessions(expires_at);

CREATE TABLE IF NOT EXISTS admin_webauthn (
    cred_id TEXT PRIMARY KEY NOT NULL,
    passkey_json TEXT NOT NULL,
    label TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    last_used_at TEXT
);

CREATE TABLE IF NOT EXISTS admin_webauthn_state (
    id TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL,
    state_json TEXT NOT NULL,
    expires_at TEXT NOT NULL
);
"#;
