//! CollecteMoi device API — upload ingestion, enrollment, lab seal, admin console.

mod access;
mod admin;
mod auth;
mod backup;
mod blobs;
mod branding;
mod client_history;
mod enroll;
mod lab;
mod mobile;
mod noise;
mod otp;
mod otp_delivery;
mod ratelimit;
mod settings;
mod storage;
mod store;
mod tls;
mod uploads;
mod version;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::{
    Router,
    extract::DefaultBodyLimit,
    http::StatusCode,
    middleware,
    routing::{get, post, put},
};
use axum_server::tls_rustls::RustlsConfig;
use clap::Parser;
use tower::limit::ConcurrencyLimitLayer;
use tower_http::cors::CorsLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

use collectemoi_db::Db;

use crate::access::AccessLog;
use crate::auth::AuthState;
use crate::blobs::BlobStore;
use crate::branding::BuildGate;
use crate::enroll::{EnrollmentOptions, EnrollmentStore};
use crate::otp::OtpStore;
use crate::ratelimit::{RateLimitConfig, RateLimiter};
use crate::settings::RuntimeSettings;
use crate::store::UploadStore;

#[derive(Debug, Parser)]
#[command(
    name = "collectemoi-api",
    version = crate::version::VERSION,
    about = "CollecteMoi device API server"
)]
struct Args {
    /// Directory for uploads, keys, and analysis jobs
    #[arg(long, default_value = "./data")]
    data_dir: PathBuf,

    /// Listen address (host:port)
    #[arg(long, default_value = "127.0.0.1:8080")]
    bind: String,

    /// Public base URL advertised in enroll / manifest responses
    #[arg(long, default_value = "https://127.0.0.1:8080")]
    public_url: String,

    /// Enable TLS (HTTPS). Default: true. Auto-generates a self-signed cert under
    /// <data-dir>/tls/ when --tls-cert/--tls-key are omitted; pass those flags to use your own PEMs.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    tls: bool,

    /// TLS certificate PEM path (default: <data-dir>/tls/server.crt — auto-created when missing)
    #[arg(long)]
    tls_cert: Option<PathBuf>,

    /// TLS private key PEM path (default: <data-dir>/tls/server.key — auto-created when missing)
    #[arg(long)]
    tls_key: Option<PathBuf>,

    /// Optional plain-HTTP companion listen address (lab only). Default: off — HTTPS only.
    /// Example: `--http-bind 127.0.0.1:8081` to also expose unencrypted admin locally.
    #[arg(long, default_value = "off")]
    http_bind: String,

    /// Enable POST /v1/lab/seal for Flutter web / Node clients
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    lab_seal: bool,

    /// Max HTTP request body size in MiB (lab seal sends whole files as base64; axum default is 2 MiB).
    #[arg(long, default_value_t = 512)]
    max_body_mb: usize,

    /// Accept legacy CMDEV envelopes (bring-up only)
    #[arg(long, default_value_t = false)]
    allow_dev_envelope: bool,

    /// Optional case public key file (hex). Default: <data-dir>/keys/case.pk.hex
    #[arg(long)]
    case_pk: Option<PathBuf>,

    /// Optional case private key file (hex). Default: <data-dir>/keys/case.sk.hex
    #[arg(long)]
    case_sk: Option<PathBuf>,

    /// Log filter (e.g. collectemoi_api=debug,info)
    #[arg(long, default_value = "collectemoi_api=info")]
    log: String,

    /// Include OTP in /v1/enroll/start responses (development only; insecure for real users).
    #[arg(long, default_value_t = false, action = clap::ArgAction::Set)]
    lab_reveal_otp: bool,

    /// Webhook URL that receives enroll OTP JSON (email delivery). Empty = not configured.
    #[arg(long, default_value = "", env = "COLLECTEMOI_OTP_WEBHOOK_URL")]
    otp_delivery_webhook_url: String,

    /// Optional Bearer token for the OTP delivery webhook.
    #[arg(long, default_value = "", env = "COLLECTEMOI_OTP_WEBHOOK_TOKEN")]
    otp_delivery_webhook_token: String,

    /// Enable per-IP rate limits + concurrency cap (application-layer DDoS / abuse shield).
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    rate_limit: bool,

    /// Max in-flight HTTP requests (whole process). Extra connections wait / fail when saturated.
    #[arg(long, default_value_t = 1024)]
    max_concurrent: usize,

    /// SQLite connection pool size (`0` = auto from CPU count).
    #[arg(long, default_value_t = 0)]
    db_pool_size: usize,

    /// Per-request timeout in seconds (slowloris / hung-client protection). Upload-friendly default.
    #[arg(long, default_value_t = 300)]
    request_timeout_secs: u64,

    /// General requests allowed per IP per rolling minute.
    #[arg(long, default_value_t = 180)]
    rate_limit_general: u32,

    /// Auth (login / setup / MFA) requests allowed per IP per rolling minute.
    #[arg(long, default_value_t = 20)]
    rate_limit_auth: u32,

    /// Enroll requests allowed per IP per rolling minute.
    #[arg(long, default_value_t = 30)]
    rate_limit_enroll: u32,

    /// Chunk upload requests allowed per IP per rolling minute (raise behind shared NAT).
    #[arg(long, default_value_t = 3000)]
    rate_limit_upload: u32,

    /// New upload sessions (`POST /v1/uploads`) allowed per device id per rolling window.
    #[arg(long, default_value_t = 10)]
    rate_limit_device_uploads: u32,

    /// Rolling window (seconds) for `--rate-limit-device-uploads` (default 1 hour).
    #[arg(long, default_value_t = 3600)]
    rate_limit_device_window_secs: u64,

    /// Offline recovery: set the admin password and exit (does not start the server).
    /// Prefer `collectemoictl reset-password`. Password from `--new-password` or
    /// `COLLECTEMOI_ADMIN_PASSWORD`.
    #[arg(long, default_value_t = false)]
    reset_admin_password: bool,

    /// New admin password for `--reset-admin-password` (or set COLLECTEMOI_ADMIN_PASSWORD).
    #[arg(long, allow_hyphen_values = true)]
    new_password: Option<String>,

    /// With `--reset-admin-password`: also disable TOTP and delete security keys.
    #[arg(long, default_value_t = false)]
    clear_mfa: bool,

    /// Offline recovery: delete the admin account so `/admin` shows first-run setup, then exit.
    #[arg(long, default_value_t = false)]
    force_admin_setup: bool,
}

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Db>,
    pub store: Arc<UploadStore>,
    pub blobs: Arc<BlobStore>,
    pub access: Arc<AccessLog>,
    pub enrollment: Arc<EnrollmentStore>,
    pub otp: Arc<OtpStore>,
    pub settings: Arc<Mutex<RuntimeSettings>>,
    pub auth: AuthState,
    pub rate_limiter: RateLimiter,
    pub build_gate: BuildGate,
    pub data_dir: PathBuf,
    /// Unix seconds when this process started listening.
    pub started_at: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // rustls may see both aws-lc-rs and ring via transitive features; pick one provider.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let args = Args::parse();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(&args.log))
        .init();

    // SQLite is the source of truth for all metadata/config/audit.
    // Large ciphertext/plaintext files stay under data/{uploads,blobs}/.
    let db = Arc::new(Db::open_with_pool_size(&args.data_dir, args.db_pool_size)?);

    if args.force_admin_setup {
        let report = auth::offline_force_setup(&db)?;
        println!(
            "admin auth cleared (sessions={}, keys={}) — open /admin to create a new password",
            report.cleared_sessions, report.cleared_keys
        );
        return Ok(());
    }
    if args.reset_admin_password {
        let password = args
            .new_password
            .clone()
            .or_else(|| std::env::var("COLLECTEMOI_ADMIN_PASSWORD").ok())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "provide --new-password or COLLECTEMOI_ADMIN_PASSWORD (use collectemoictl reset-password to prompt)"
                )
            })?;
        let report = auth::offline_reset_password(&db, &password, args.clear_mfa)?;
        println!(
            "admin password {} for user '{}' (sessions cleared={}, mfa_cleared={}, keys_removed={})",
            if report.created { "created" } else { "reset" },
            report.username,
            report.cleared_sessions,
            report.mfa_cleared,
            report.cleared_keys
        );
        return Ok(());
    }

    tracing::info!(
        db = %db.path().display(),
        pool = db.pool_size(),
        "sqlite catalogue ready"
    );
    let public_url = normalize_public_url(&args.public_url, args.tls);
    let seed_demo_lab_keys = args
        .bind
        .parse::<SocketAddr>()
        .map(|addr| addr.ip().is_loopback())
        .unwrap_or(false);
    if seed_demo_lab_keys {
        tracing::info!("loopback bind — seeding demo companies, lab keys, and enrollment tokens when catalogue is empty");
    } else {
        tracing::info!("non-loopback bind — starting without demo companies/lab keys/tokens (create them in admin)");
    }
    if args.lab_reveal_otp {
        tracing::warn!("lab OTP responses are enabled; email enrollment codes will be exposed by the API");
    }
    if args.lab_seal && !seed_demo_lab_keys {
        tracing::warn!("lab sealing is enabled on a non-loopback listener; web clients send device private keys to this server");
    }

    let store = Arc::new(UploadStore::open(&args.data_dir, db.clone())?);
    let blobs = Arc::new(BlobStore::open(&args.data_dir, db.clone())?);
    let access = Arc::new(AccessLog::open(&args.data_dir, db.clone())?);
    let enrollment = Arc::new(EnrollmentStore::open(
        &args.data_dir,
        db.clone(),
        EnrollmentOptions {
            public_url: public_url.clone(),
            case_pk: args.case_pk.clone(),
            case_sk: args.case_sk.clone(),
            seed_demo_tokens: seed_demo_lab_keys,
        },
    )?);
    let otp = Arc::new(OtpStore::open(
        &args.data_dir,
        db.clone(),
        args.lab_reveal_otp,
        seed_demo_lab_keys,
    )?);

    let tls_paths = if args.tls {
        Some(tls::ensure_tls_files(
            &args.data_dir,
            args.tls_cert.as_deref(),
            args.tls_key.as_deref(),
            &public_url,
        )?)
    } else {
        None
    };

    let mut runtime = RuntimeSettings {
        data_dir: args.data_dir.display().to_string(),
        bind: args.bind.clone(),
        public_url: public_url.clone(),
        display_name: "CollecteMoi Lab".into(),
        lab_seal: args.lab_seal,
        allow_dev_envelope: args.allow_dev_envelope,
        log: args.log.clone(),
        tls: args.tls,
        tls_cert: tls_paths
            .as_ref()
            .map(|(c, _)| c.display().to_string())
            .unwrap_or_default(),
        tls_key: tls_paths
            .as_ref()
            .map(|(_, k)| k.display().to_string())
            .unwrap_or_default(),
        http_bind: normalize_http_bind(&args.http_bind, args.tls),
        lab_reveal_otp: args.lab_reveal_otp,
        otp_delivery_webhook_url: args.otp_delivery_webhook_url.clone(),
        otp_delivery_webhook_token: args.otp_delivery_webhook_token.clone(),
        healthz_interval_secs: settings::DEFAULT_HEALTHZ_INTERVAL_SECS,
    };
    // Overlay lab toggles saved from the admin console so restart keeps them.
    if let Some(persisted) = settings::load_persisted(&db) {
        runtime.apply_persisted(&persisted);
        tracing::info!(
            display_name = %runtime.display_name,
            public_url = %runtime.public_url,
            lab_seal = runtime.lab_seal,
            allow_dev_envelope = runtime.allow_dev_envelope,
            otp_webhook = !runtime.otp_delivery_webhook_url.is_empty(),
            "restored persisted runtime settings"
        );
    }
    // CLI / env webhook wins when provided (so deploy can inject without wiping SQLite).
    if !args.otp_delivery_webhook_url.trim().is_empty() {
        runtime.otp_delivery_webhook_url = args.otp_delivery_webhook_url.trim().to_string();
    }
    if !args.otp_delivery_webhook_token.trim().is_empty() {
        runtime.otp_delivery_webhook_token = args.otp_delivery_webhook_token.trim().to_string();
    }
    if runtime.otp_webhook_configured() {
        tracing::info!(
            url = %runtime.otp_delivery_webhook_url,
            "OTP delivery webhook configured"
        );
    }
    // Keep enrollment store in sync with the effective public URL / name.
    enrollment.set_display_name(&runtime.display_name);
    enrollment.set_public_url(&runtime.public_url);
    let public_url = runtime.public_url.clone();

    let settings = Arc::new(Mutex::new(runtime));
    let started_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let http_bind = settings.lock().unwrap().http_bind.clone();
    let webauthn = auth::build_webauthn(&public_url, &http_bind)?;
    let rate_limiter = RateLimiter::new(RateLimitConfig {
        enabled: args.rate_limit,
        general_per_min: args.rate_limit_general.max(10),
        auth_per_min: args.rate_limit_auth.max(5),
        enroll_per_min: args.rate_limit_enroll.max(5),
        upload_per_min: args.rate_limit_upload.max(60),
        device_uploads_per_hour: args.rate_limit_device_uploads.max(1),
        device_upload_window_secs: args.rate_limit_device_window_secs.max(60),
        ..RateLimitConfig::default()
    });
    let state = AppState {
        db,
        store,
        blobs,
        access,
        enrollment,
        otp,
        settings: settings.clone(),
        auth: AuthState {
            webauthn: Arc::new(webauthn),
            fail_guard: Arc::new(Mutex::new(auth::FailGuard::default())),
        },
        rate_limiter,
        build_gate: BuildGate::new(),
        data_dir: args.data_dir.clone(),
        started_at,
    };

    let max_body = args.max_body_mb.saturating_mul(1024 * 1024);
    let max_concurrent = args.max_concurrent.max(8);
    let timeout_secs = args.request_timeout_secs.max(10);
    let app = build_router(state, max_body, max_concurrent, timeout_secs);

    let addr: SocketAddr = args.bind.parse()?;
    let scheme = if args.tls { "https" } else { "http" };
    let (http_bind, tls_cert_disp) = {
        let s = settings.lock().unwrap();
        (s.http_bind.clone(), s.tls_cert.clone())
    };
    if !args.tls {
        tracing::warn!("TLS disabled — serving plain HTTP (lab only; prefer --tls true)");
    }
    tracing::info!(
        %addr,
        scheme,
        tls_cert = %if tls_cert_disp.is_empty() { "none" } else { tls_cert_disp.as_str() },
        http_bind = %if http_bind.is_empty() { "off" } else { http_bind.as_str() },
        data = %args.data_dir.display(),
        lab_seal = args.lab_seal,
        max_body_mb = args.max_body_mb,
        allow_dev_envelope = args.allow_dev_envelope,
        rate_limit = args.rate_limit,
        max_concurrent,
        request_timeout_secs = timeout_secs,
        admin = %format!("{scheme}://{addr}/admin"),
        admin_http = %if http_bind.is_empty() {
            "off".to_string()
        } else {
            format!("http://{http_bind}/admin")
        },
        "collectemoi-api listening (HTTPS-only unless --http-bind is set)"
    );

    if let Some((cert, key)) = tls_paths {
        let config = RustlsConfig::from_pem_file(&cert, &key)
            .await
            .map_err(|e| anyhow::anyhow!("load TLS PEM: {e}"))?;
        if http_bind.is_empty() {
            axum_server::bind_rustls(addr, config)
                .serve(app.into_make_service_with_connect_info::<SocketAddr>())
                .await?;
        } else {
            let http_addr: SocketAddr = http_bind.parse()?;
            let https_app = app.clone();
            let http_app = app;
            let https = async move {
                axum_server::bind_rustls(addr, config)
                    .serve(https_app.into_make_service_with_connect_info::<SocketAddr>())
                    .await
            };
            let http = async move {
                let listener = tokio::net::TcpListener::bind(http_addr).await?;
                tracing::info!(%http_addr, "plain HTTP lab listener (no TLS — local admin)");
                axum::serve(
                    listener,
                    http_app.into_make_service_with_connect_info::<SocketAddr>(),
                )
                .await?;
                Ok::<(), anyhow::Error>(())
            };
            tokio::try_join!(
                async {
                    https
                        .await
                        .map_err(|e| anyhow::anyhow!("https server: {e}"))
                },
                http
            )?;
        }
    } else {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await?;
    }
    Ok(())
}

fn build_router(
    state: AppState,
    max_body_bytes: usize,
    max_concurrent: usize,
    timeout_secs: u64,
) -> Router {
    use axum::routing::delete;

    Router::new()
        .route("/healthz", get(admin::healthz))
        .route("/v1/status", get(admin::public_status))
        .route("/admin", get(admin::console))
        .route("/admin/", get(admin::console))
        .route("/admin/logo.png", get(admin::console_logo))
        .route("/admin/favicon.png", get(admin::console_favicon))
        .route("/favicon.png", get(admin::console_favicon))
        .route("/v1/admin/auth/status", get(auth::auth_status))
        .route("/v1/admin/auth/setup/start", post(auth::setup_start))
        .route("/v1/admin/auth/setup/finish", post(auth::setup_finish))
        .route("/v1/admin/auth/login", post(auth::login))
        .route("/v1/admin/auth/mfa/totp", post(auth::mfa_totp))
        .route(
            "/v1/admin/auth/mfa/webauthn/start",
            post(auth::mfa_webauthn_start),
        )
        .route(
            "/v1/admin/auth/mfa/webauthn/finish",
            post(auth::mfa_webauthn_finish),
        )
        .route("/v1/admin/auth/logout", post(auth::logout))
        .route("/v1/admin/auth/me", get(auth::me))
        .route("/v1/admin/auth/password", post(auth::change_password))
        .route(
            "/v1/admin/auth/totp/enable/start",
            post(auth::totp_enable_start),
        )
        .route(
            "/v1/admin/auth/totp/enable/finish",
            post(auth::totp_enable_finish),
        )
        .route("/v1/admin/auth/totp/disable", post(auth::totp_disable))
        .route(
            "/v1/admin/auth/webauthn/register/start",
            post(auth::webauthn_register_start),
        )
        .route(
            "/v1/admin/auth/webauthn/register/finish",
            post(auth::webauthn_register_finish),
        )
        .route("/v1/admin/auth/webauthn", get(auth::webauthn_list))
        .route(
            "/v1/admin/auth/webauthn/{cred_id}",
            delete(auth::webauthn_delete),
        )
        .route("/v1/admin/overview", get(admin::overview))
        .route("/v1/admin/status", get(admin::status))
        .route(
            "/v1/admin/settings",
            get(admin::get_settings).patch(admin::patch_settings),
        )
        .route("/v1/admin/storage", get(admin::storage))
        .route("/v1/admin/storage/wipe", axum::routing::post(admin::wipe_storage))
        .route("/v1/admin/devices", get(admin::list_devices))
        .route("/v1/admin/devices/{id}/ban", post(admin::set_device_ban))
        .route(
            "/v1/admin/devices/{id}/rate-limit-bypass",
            post(admin::set_device_rate_limit_bypass),
        )
        .route("/v1/admin/keys", get(admin::list_client_keys))
        .route("/v1/admin/keys/ban", post(admin::ban_public_key))
        .route("/v1/admin/keys/unban", post(admin::unban_public_key))
        .route("/v1/admin/uploads", get(admin::list_uploads))
        .route(
            "/v1/admin/uploads/{id}",
            axum::routing::delete(admin::delete_upload),
        )
        .route("/v1/admin/jobs", get(admin::list_jobs))
        .route("/v1/admin/analysis", get(admin::list_analysis))
        .route("/v1/admin/blobs", get(blobs::list_blobs))
        .route(
            "/v1/admin/blobs/{id}",
            get(blobs::get_blob).delete(blobs::delete_blob),
        )
        .route("/v1/admin/blobs/{id}/content", get(blobs::get_blob_content))
        .route("/v1/admin/access-stats", get(access::access_stats))
        .route("/v1/admin/access-log", get(access::access_log))
        .route(
            "/v1/admin/tokens",
            get(admin::list_tokens).post(admin::create_token),
        )
        .route(
            "/v1/admin/companies",
            get(admin::list_companies).post(admin::upsert_company),
        )
        .route(
            "/v1/admin/companies/{id}",
            get(admin::get_company).delete(admin::delete_company),
        )
        .route(
            "/v1/admin/companies/{id}/enabled",
            axum::routing::post(admin::set_company_enabled),
        )
        .route(
            "/v1/admin/lab-keys",
            get(admin::list_lab_keys).post(admin::upsert_lab_key),
        )
        .route(
            "/v1/admin/lab-keys/{key}",
            axum::routing::delete(admin::delete_lab_key),
        )
        .route("/v1/admin/enrollment-log", get(admin::enrollment_log))
        .route("/v1/admin/logs", get(admin::process_logs))
        .route("/v1/admin/export", get(backup::export_backup))
        .route("/v1/admin/import", post(backup::import_backup))
        .route("/v1/admin/mobile", get(mobile::list_mobile))
        .route(
            "/v1/admin/mobile/{name}",
            get(mobile::download_mobile).delete(mobile::delete_mobile),
        )
        .route(
            "/v1/admin/branding",
            get(branding::get_branding).put(branding::put_branding),
        )
        .route(
            "/v1/admin/i18n/catalog",
            get(branding::get_i18n_catalog),
        )
        .route(
            "/v1/admin/branding/assets",
            post(branding::upload_branding_asset),
        )
        .route(
            "/v1/admin/branding/assets/{kind}",
            get(branding::get_branding_asset),
        )
        .route(
            "/v1/admin/branding/build",
            get(branding::get_build).post(branding::start_build),
        )
        .route("/v1/admin/branding/build/stop", post(branding::stop_build))
        .route(
            "/v1/admin/branding/versions",
            post(branding::create_version),
        )
        .route(
            "/v1/admin/branding/versions/{version}",
            axum::routing::delete(branding::delete_version),
        )
        .route("/v1/blobs", get(blobs::list_blobs))
        .route("/v1/blobs/{id}", get(blobs::get_blob))
        .route("/v1/blobs/{id}/content", get(blobs::get_blob_content))
        .route("/v1/enroll", post(enroll::enroll))
        .route("/v1/enroll/start", post(otp::enroll_start))
        .route("/v1/enroll/confirm", post(otp::enroll_confirm))
        .route("/v1/tenant/manifest", get(enroll::manifest))
        .route("/v1/requests", get(enroll::requests))
        .route(
            "/v1/client/submissions",
            post(client_history::list_submissions),
        )
        .route("/v1/messages", post(enroll::post_message))
        .route("/v1/lab/seal", post(lab::lab_seal))
        .route("/v1/uploads", post(uploads::create_upload))
        .route("/v1/uploads/{id}", get(uploads::get_upload))
        .route("/v1/uploads/{id}/chunks/{n}", put(uploads::put_chunk))
        .route("/v1/uploads/{id}/complete", post(uploads::complete_upload))
        // Lab seal posts whole files as base64 JSON; axum's default 2 MiB limit rejects them.
        .layer(DefaultBodyLimit::max(max_body_bytes))
        .layer(CorsLayer::very_permissive())
        .layer(TraceLayer::new_for_http())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_admin,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            access::middleware,
        ))
        // Outer shields: reject abusive IPs early, then cap concurrency / hung clients.
        .layer(middleware::from_fn_with_state(
            state.clone(),
            ratelimit::middleware,
        ))
        .layer(ConcurrencyLimitLayer::new(max_concurrent))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(timeout_secs),
        ))
        .with_state(state)
}

fn normalize_http_bind(raw: &str, tls: bool) -> String {
    let v = raw.trim();
    if !tls || v.is_empty() || v.eq_ignore_ascii_case("off") || v == "-" {
        return String::new();
    }
    v.to_string()
}

fn normalize_public_url(url: &str, tls: bool) -> String {
    let url = url.trim().trim_end_matches('/').to_string();
    if tls && url.starts_with("http://") {
        tracing::warn!(
            "TLS enabled but --public-url uses http:// — rewriting to https:// (pass an https URL explicitly to silence this)"
        );
        return format!("https://{}", url.trim_start_matches("http://"));
    }
    if !tls && url.starts_with("https://") {
        tracing::warn!("TLS disabled but --public-url uses https:// — rewriting to http://");
        return format!("http://{}", url.trim_start_matches("https://"));
    }
    url
}
