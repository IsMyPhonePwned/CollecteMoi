//! Blocking HTTPS admin client (cookie session, lab self-signed TLS).

use anyhow::{bail, Context, Result};
use reqwest::blocking::{Client, Response};
use reqwest::cookie::Jar;
use reqwest::header::{HeaderMap, SET_COOKIE};
use reqwest::Url;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct AdminClient {
    base: Url,
    http: Client,
    jar: Arc<Jar>,
}

impl AdminClient {
    pub fn connect(server: &str, insecure: bool) -> Result<Self> {
        let base = Url::parse(server.trim_end_matches('/'))
            .with_context(|| format!("invalid server URL: {server}"))?;
        let jar = Arc::new(Jar::default());
        let mut builder = Client::builder()
            .cookie_provider(jar.clone())
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(8))
            .user_agent(concat!("collectemoi-tui/", env!("CARGO_PKG_VERSION")));
        if insecure || base.scheme() == "https" {
            // Lab default: accept auto-generated self-signed certs.
            builder = builder.danger_accept_invalid_certs(true);
        }
        let http = builder.build().context("build HTTP client")?;
        Ok(Self { base, http, jar })
    }

    fn url(&self, path: &str) -> Result<Url> {
        self.base
            .join(path.trim_start_matches('/'))
            .context("join URL")
    }

    pub fn healthz(&self) -> Result<(u16, String)> {
        let res = self.http.get(self.url("healthz")?).send()?;
        let status = res.status().as_u16();
        let body = res.text().unwrap_or_default().trim().to_string();
        Ok((status, body))
    }

    pub fn auth_status(&self) -> Result<Value> {
        self.get_json("v1/admin/auth/status")
    }

    pub fn login(&self, password: &str, totp: Option<&str>) -> Result<()> {
        let status = self.auth_status()?;
        if status
            .get("setup_required")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            bail!(
                "admin setup required — open {}/admin or run: ./scripts/collectemoictl reset-password --force-setup --yes",
                self.base
            );
        }

        let mut res = self
            .http
            .post(self.url("v1/admin/auth/login")?)
            .json(&serde_json::json!({ "password": password }))
            .send()
            .context("login request")?;
        self.capture_set_cookie(&mut res)?;
        if !res.status().is_success() {
            let err = res
                .json::<Value>()
                .ok()
                .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
                .unwrap_or_else(|| "login failed".into());
            bail!("{err}");
        }
        let body: Value = res.json().context("login response")?;
        let stage = body
            .get("stage")
            .and_then(|v| v.as_str())
            .unwrap_or("full");
        if stage == "full" || body.get("authenticated").and_then(|v| v.as_bool()) == Some(true) {
            return Ok(());
        }
        if stage == "mfa" {
            let code = match totp.map(str::trim).filter(|s| !s.is_empty()) {
                Some(c) => c.to_string(),
                None => rpassword::prompt_password("TOTP code: ").context("read TOTP")?,
            };
            let mut mfa = self
                .http
                .post(self.url("v1/admin/auth/mfa/totp")?)
                .json(&serde_json::json!({ "code": code.trim() }))
                .send()
                .context("mfa request")?;
            self.capture_set_cookie(&mut mfa)?;
            if !mfa.status().is_success() {
                let err = mfa
                    .json::<Value>()
                    .ok()
                    .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
                    .unwrap_or_else(|| "mfa failed".into());
                bail!("{err}");
            }
            return Ok(());
        }
        bail!("unexpected auth stage: {stage}");
    }

    pub fn get_json(&self, path: &str) -> Result<Value> {
        let res = self
            .http
            .get(self.url(path)?)
            .send()
            .with_context(|| format!("GET {path}"))?;
        let status = res.status();
        let text = res.text().unwrap_or_default();
        if !status.is_success() {
            bail!("GET {path} → HTTP {status}: {}", text.chars().take(160).collect::<String>());
        }
        serde_json::from_str(&text).with_context(|| format!("parse {path}"))
    }

    /// reqwest cookie jar sometimes misses Secure cookies on https://127.0.0.1 — stash manually.
    fn capture_set_cookie(&self, res: &mut Response) -> Result<()> {
        let headers: HeaderMap = res.headers().clone();
        for value in headers.get_all(SET_COOKIE) {
            if let Ok(s) = value.to_str() {
                self.jar.add_cookie_str(s, &self.base);
                // Also force a non-Secure variant into the jar for local probes.
                if let Some(name_val) = s.split(';').next() {
                    let patched = format!("{name_val}; Path=/");
                    self.jar.add_cookie_str(&patched, &self.base);
                }
            }
        }
        Ok(())
    }
}
