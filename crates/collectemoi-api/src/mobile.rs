//! Serve lab mobile builds (APK / IPA / iOS zip) from known artefact directories.

use std::env;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use axum::{
    Json,
    body::Body,
    extract::{Path as AxumPath, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio_util::io::ReaderStream;

use crate::AppState;

#[derive(Debug, Clone, Serialize)]
pub struct MobileBuild {
    pub name: String,
    pub platform: String,
    /// App version baked into the binary (Flutter / aapt), when known.
    pub version: String,
    /// Company folder under `data/mobile/<company_id>/…` (empty = legacy flat publish).
    #[serde(default)]
    pub company_id: String,
    /// Branding/app version folder under the company (`data/mobile/<company>/<branding_version>/`).
    #[serde(default)]
    pub branding_version: String,
    pub sha256: String,
    pub size: u64,
    pub modified_at: String,
    pub content_type: String,
    pub path_hint: String,
}

#[derive(Debug, Deserialize)]
pub struct MobileListQuery {
    #[serde(default)]
    pub company_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MobileDownloadQuery {
    #[serde(default)]
    pub company_id: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BuildMeta {
    name: String,
    #[serde(default)]
    version: String,
    sha256: String,
    size: u64,
    #[serde(default)]
    mtime_secs: u64,
    #[serde(default)]
    platform: String,
}

fn mobile_roots(state: &AppState) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(raw) = env::var("COLLECTEMOI_MOBILE_DIR") {
        let t = raw.trim();
        if !t.is_empty() {
            roots.push(PathBuf::from(t));
        }
    }
    roots.push(state.data_dir.join("mobile"));
    // Lab default: <repo>/data → sibling dist/mobile
    if let Some(parent) = state.data_dir.parent() {
        roots.push(parent.join("dist").join("mobile"));
    }
    // CWD fallback when the API is started from the repo root
    roots.push(PathBuf::from("dist/mobile"));

    let mut out = Vec::new();
    for r in roots {
        if !out.iter().any(|x: &PathBuf| x == &r) {
            out.push(r);
        }
    }
    out
}

fn is_safe_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 180 {
        return false;
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn allowed_ext(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".apk") || lower.ends_with(".ipa") || lower.ends_with(".zip")
}

fn platform_for(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".apk") {
        "android"
    } else if lower.ends_with(".ipa") || lower.contains("ios") {
        "ios"
    } else {
        "other"
    }
}

fn content_type_for(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".apk") {
        "application/vnd.android.package-archive"
    } else if lower.ends_with(".ipa") {
        "application/octet-stream"
    } else if lower.ends_with(".zip") {
        "application/zip"
    } else {
        "application/octet-stream"
    }
}

fn modified_rfc3339(meta: &std::fs::Metadata) -> String {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| {
            time::OffsetDateTime::from_unix_timestamp(d.as_secs() as i64)
                .map(|odt| {
                    odt.format(&time::format_description::well_known::Rfc3339)
                        .unwrap_or_else(|_| odt.to_string())
                })
                .unwrap_or_else(|_| String::new())
        })
        .unwrap_or_default()
}

fn mtime_secs(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn meta_path(path: &Path) -> PathBuf {
    let mut p = path.as_os_str().to_owned();
    p.push(".meta.json");
    PathBuf::from(p)
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let file = File::open(path)?;
    let mut reader = BufReader::with_capacity(1024 * 1024, file);
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 1024 * 64];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn load_sidecar(path: &Path, size: u64, mtime: u64) -> Option<BuildMeta> {
    let raw = std::fs::read_to_string(meta_path(path)).ok()?;
    let meta: BuildMeta = serde_json::from_str(&raw).ok()?;
    if meta.size != size {
        return None;
    }
    // Allow missing mtime in older sidecars; if present it must match.
    if meta.mtime_secs != 0 && meta.mtime_secs != mtime {
        return None;
    }
    if meta.sha256.len() != 64 || !meta.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(meta)
}

fn write_sidecar(path: &Path, meta: &BuildMeta) {
    let Ok(json) = serde_json::to_string_pretty(meta) else {
        return;
    };
    let _ = std::fs::write(meta_path(path), format!("{json}\n"));
}

fn plist_string_after_key(xml: &str, key: &str) -> Option<String> {
    let needle = format!("<key>{key}</key>");
    let idx = xml.find(&needle)?;
    let after = &xml[idx + needle.len()..];
    let start = after.find("<string>")? + "<string>".len();
    let end = after[start..].find("</string>")? + start;
    let val = after[start..end].trim();
    if val.is_empty() {
        None
    } else {
        Some(val.to_string())
    }
}

fn version_from_ipa_or_zip(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let mut info_idx = None;
    for i in 0..archive.len() {
        let name = archive.by_index(i).ok()?.name().to_string();
        // Payload/Runner.app/Info.plist  or  Runner.app/Info.plist
        if name.ends_with("Info.plist")
            && (name.contains(".app/") || name.contains(".app\\"))
            && !name.contains("Frameworks/")
            && !name.contains("PlugIns/")
        {
            info_idx = Some(i);
            break;
        }
    }
    let i = info_idx?;
    let mut entry = archive.by_index(i).ok()?;
    let mut buf = String::new();
    // Binary plists won't parse; skip quietly.
    entry.read_to_string(&mut buf).ok()?;
    if !buf.contains("<plist") {
        return None;
    }
    let short = plist_string_after_key(&buf, "CFBundleShortVersionString");
    let build = plist_string_after_key(&buf, "CFBundleVersion");
    match (short, build) {
        (Some(s), Some(b)) if !b.is_empty() && b != s => Some(format!("{s}+{b}")),
        (Some(s), _) => Some(s),
        (None, Some(b)) => Some(b),
        _ => None,
    }
}

fn version_from_aapt(path: &Path) -> Option<String> {
    let aapt = ["aapt", "aapt2"]
        .into_iter()
        .find(|bin| Command::new(bin).arg("version").output().is_ok())?;
    let output = Command::new(aapt)
        .args(["dump", "badging"])
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    // package: name='…' versionCode='1' versionName='0.1.0'
    let name = text
        .lines()
        .find(|l| l.starts_with("package:"))
        .and_then(|l| {
            l.split("versionName='")
                .nth(1)
                .and_then(|rest| rest.split('\'').next())
                .map(str::to_string)
        })?;
    let code = text
        .lines()
        .find(|l| l.starts_with("package:"))
        .and_then(|l| {
            l.split("versionCode='")
                .nth(1)
                .and_then(|rest| rest.split('\'').next())
                .map(str::to_string)
        });
    match code {
        Some(c) if !c.is_empty() && c != name => Some(format!("{name}+{c}")),
        _ => Some(name),
    }
}

fn version_from_pubspec_near(path: &Path) -> Option<String> {
    // …/dist/mobile/foo.apk → …/app/pubspec.yaml
    // …/data/mobile/foo.apk → …/app/pubspec.yaml
    let mut dir = path.parent()?.to_path_buf();
    for _ in 0..4 {
        let candidate = dir.join("app").join("pubspec.yaml");
        if candidate.is_file() {
            let text = std::fs::read_to_string(candidate).ok()?;
            for line in text.lines() {
                let t = line.trim();
                if let Some(rest) = t.strip_prefix("version:") {
                    let v = rest.trim();
                    if !v.is_empty() {
                        return Some(v.to_string());
                    }
                }
            }
        }
        if !dir.pop() {
            break;
        }
    }
    None
}

fn detect_version(path: &Path, name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".ipa") || lower.ends_with(".zip") {
        if let Some(v) = version_from_ipa_or_zip(path) {
            return v;
        }
    }
    if lower.ends_with(".apk") {
        if let Some(v) = version_from_aapt(path) {
            return v;
        }
    }
    version_from_pubspec_near(path).unwrap_or_default()
}

fn company_scope_from_path(root: &Path, path: &Path) -> (String, String) {
    // data/mobile/<company>/<branding_version>/file.apk
    // data/mobile/file.apk → legacy flat (treat as _default)
    let Ok(rel) = path.strip_prefix(root) else {
        return (String::new(), String::new());
    };
    let parts: Vec<_> = rel
        .components()
        .filter_map(|c| c.as_os_str().to_str().map(str::to_string))
        .collect();
    match parts.as_slice() {
        [_file] => ("_default".into(), String::new()),
        [company, file] if !file.contains('.') || Path::new(file).extension().is_some() => {
            // company/file.apk (no version folder)
            (company.clone(), String::new())
        }
        [company, version, _file, ..] => (company.clone(), version.clone()),
        _ => (String::new(), String::new()),
    }
}

fn enrich_build(
    path: &Path,
    name: String,
    meta_fs: &std::fs::Metadata,
    root: &Path,
) -> MobileBuild {
    let size = meta_fs.len();
    let mtime = mtime_secs(meta_fs);
    let platform = platform_for(&name).to_string();
    let (company_id, branding_version) = company_scope_from_path(root, path);

    let (version, sha256) = if let Some(side) = load_sidecar(path, size, mtime) {
        let version = if side.version.is_empty() {
            detect_version(path, &name)
        } else {
            side.version
        };
        (version, side.sha256)
    } else {
        let sha256 = sha256_file(path).unwrap_or_default();
        let version = detect_version(path, &name);
        if !sha256.is_empty() {
            write_sidecar(
                path,
                &BuildMeta {
                    name: name.clone(),
                    version: version.clone(),
                    sha256: sha256.clone(),
                    size,
                    mtime_secs: mtime,
                    platform: platform.clone(),
                },
            );
        }
        (version, sha256)
    };

    MobileBuild {
        name,
        platform,
        version,
        company_id,
        branding_version,
        sha256,
        size,
        modified_at: modified_rfc3339(meta_fs),
        content_type: content_type_for(
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default(),
        )
        .to_string(),
        path_hint: path
            .strip_prefix(root)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| path.display().to_string()),
    }
}

fn find_build(
    state: &AppState,
    name: &str,
    company_id: Option<&str>,
    branding_version: Option<&str>,
) -> Option<PathBuf> {
    if !is_safe_name(name) || !allowed_ext(name) {
        return None;
    }
    let company = company_id.map(str::trim).filter(|s| !s.is_empty());
    let version = branding_version.map(str::trim).filter(|s| !s.is_empty());

    for root in mobile_roots(state) {
        // Prefer scoped path when company is known.
        if let Some(cid) = company {
            let scoped = match version {
                Some(ver) => root.join(cid).join(ver).join(name),
                None => root.join(cid).join(name),
            };
            if scoped.is_file() {
                return Some(scoped);
            }
            // Fall back: any version under that company.
            if version.is_none() {
                if let Ok(entries) = std::fs::read_dir(root.join(cid)) {
                    for e in entries.flatten() {
                        let p = e.path().join(name);
                        if p.is_file() {
                            return Some(p);
                        }
                        if e.path().is_dir() {
                            if let Ok(inner) = std::fs::read_dir(e.path()) {
                                for f in inner.flatten() {
                                    let fp = f.path();
                                    if fp.is_file()
                                        && fp.file_name().and_then(|n| n.to_str()) == Some(name)
                                    {
                                        return Some(fp);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let candidate = root.join(name);
        if candidate.is_file()
            && candidate
                .file_name()
                .and_then(|f| f.to_str())
                .map(|f| f == name)
                .unwrap_or(false)
        {
            // Flat legacy publish — only when no company filter, or _default.
            if company.is_none() || company == Some("_default") {
                return Some(candidate);
            }
        }
    }
    None
}

fn scan_builds(state: &AppState) -> Vec<MobileBuild> {
    let mut builds = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for root in mobile_roots(state) {
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let Some(name) = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(str::to_string)
                else {
                    continue;
                };
                if name.ends_with(".meta.json") {
                    continue;
                }
                if !is_safe_name(&name) || !allowed_ext(&name) {
                    continue;
                }
                let Ok(meta) = entry.metadata() else {
                    continue;
                };
                let (company_id, branding_version) = company_scope_from_path(&root, &path);
                let dedup = format!("{company_id}|{branding_version}|{name}|{}", path.display());
                if !seen.insert(dedup) {
                    continue;
                }
                builds.push(enrich_build(&path, name, &meta, &root));
            }
        }
    }

    builds.sort_by(|a, b| {
        b.modified_at
            .cmp(&a.modified_at)
            .then_with(|| a.company_id.cmp(&b.company_id))
            .then_with(|| a.name.cmp(&b.name))
    });
    builds
}

pub async fn list_mobile(
    State(state): State<AppState>,
    Query(q): Query<MobileListQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let filter = q
        .company_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let builds = tokio::task::spawn_blocking(move || {
        let mut builds = scan_builds(&state);
        if let Some(cid) = filter {
            builds
                .retain(|b| b.company_id == cid || (cid == "_default" && b.company_id.is_empty()));
        }
        builds
    })
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
    })?;
    Ok(Json(json!({
        "ok": true,
        "builds": builds,
        "company_id": q.company_id,
        "hint": "Generate from Client apps for a company — artefacts land in data/mobile/<company>/<version>/",
    })))
}

/// Delete one published APK/IPA (and its `.meta.json` sidecar when present).
pub async fn delete_mobile(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
    Query(q): Query<MobileDownloadQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    if !is_safe_name(&name) || !allowed_ext(&name) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "invalid build name" })),
        ));
    }
    let path = find_build(&state, &name, q.company_id.as_deref(), q.version.as_deref())
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "build not found" })),
            )
        })?;

    // Only allow deletes under known mobile roots.
    let roots = mobile_roots(&state);
    let canon = path.canonicalize().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
    })?;
    let allowed = roots.iter().any(|r| {
        r.canonicalize()
            .map(|root| canon.starts_with(&root))
            .unwrap_or(false)
    });
    if !allowed {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "refusing to delete build outside mobile dirs" })),
        ));
    }

    let sidecar = meta_path(&path);

    tokio::fs::remove_file(&path).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("delete failed: {e}") })),
        )
    })?;
    let _ = tokio::fs::remove_file(&sidecar).await;

    // Clean empty version / company dirs under data/mobile.
    if let Some(parent) = path.parent() {
        if let Ok(mut rd) = tokio::fs::read_dir(parent).await {
            let mut empty = true;
            while let Ok(Some(_)) = rd.next_entry().await {
                empty = false;
                break;
            }
            if empty {
                let _ = tokio::fs::remove_dir(parent).await;
                if let Some(grand) = parent.parent() {
                    if let Ok(mut rd2) = tokio::fs::read_dir(grand).await {
                        let mut gempty = true;
                        while let Ok(Some(_)) = rd2.next_entry().await {
                            gempty = false;
                            break;
                        }
                        if gempty {
                            let _ = tokio::fs::remove_dir(grand).await;
                        }
                    }
                }
            }
        }
    }

    tracing::info!(
        name = %name,
        company_id = q.company_id.as_deref().unwrap_or(""),
        version = q.version.as_deref().unwrap_or(""),
        path = %path.display(),
        "mobile build deleted"
    );

    Ok(Json(json!({
        "ok": true,
        "deleted": name,
        "company_id": q.company_id,
        "version": q.version,
        "path": path.display().to_string(),
    })))
}

pub async fn download_mobile(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
    Query(q): Query<MobileDownloadQuery>,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    if !is_safe_name(&name) || !allowed_ext(&name) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "invalid build name" })),
        ));
    }

    let path = find_build(
        &state,
        &name,
        q.company_id.as_deref(),
        q.version.as_deref(),
    )
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "build not found",
                "hint": "Generate from Client apps for this company, or run ./scripts/collectemoi-app apk",
            })),
        )
    })?;

    let meta = tokio::fs::metadata(&path).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
    })?;

    // Best-effort digest for clients that want to verify the download.
    let sha_header = {
        let path_c = path.clone();
        let size = meta.len();
        let mtime = mtime_secs(&meta);
        tokio::task::spawn_blocking(move || {
            load_sidecar(&path_c, size, mtime)
                .map(|m| m.sha256)
                .or_else(|| sha256_file(&path_c).ok())
        })
        .await
        .ok()
        .flatten()
    };

    let file = tokio::fs::File::open(&path).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
    })?;
    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(content_type_for(&name)),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"{name}\"")
            .parse()
            .unwrap_or(HeaderValue::from_static("attachment")),
    );
    if meta.len() > 0 {
        headers.insert(
            header::CONTENT_LENGTH,
            HeaderValue::from_str(&meta.len().to_string()).unwrap_or(HeaderValue::from_static("0")),
        );
    }
    if let Some(sha) = sha_header.as_deref() {
        if let Ok(v) = HeaderValue::from_str(&format!("sha-256={sha}")) {
            headers.insert(header::HeaderName::from_static("digest"), v);
        }
        if let Ok(v) = HeaderValue::from_str(sha) {
            headers.insert(header::HeaderName::from_static("x-content-sha256"), v);
        }
    }

    tracing::info!(
        name = %name,
        size = meta.len(),
        sha256 = sha_header.as_deref().unwrap_or(""),
        path = %path.display(),
        "mobile build download"
    );

    Ok((StatusCode::OK, headers, body).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_path_traversal_names() {
        assert!(!is_safe_name("../etc/passwd"));
        assert!(!is_safe_name("foo/bar.apk"));
        assert!(!is_safe_name(""));
        assert!(is_safe_name("collectemoi-debug.apk"));
        assert!(is_safe_name("collectemoi-ios-simulator.ipa"));
        assert!(allowed_ext("x.apk"));
        assert!(allowed_ext("x.IPA"));
        assert!(!allowed_ext("x.txt"));
        assert_eq!(platform_for("a.apk"), "android");
        assert_eq!(platform_for("a.ipa"), "ios");
    }

    #[test]
    fn parses_plist_version_keys() {
        let xml = r#"<?xml version="1.0"?>
        <plist><dict>
          <key>CFBundleShortVersionString</key>
          <string>0.1.0</string>
          <key>CFBundleVersion</key>
          <string>1</string>
        </dict></plist>"#;
        assert_eq!(
            plist_string_after_key(xml, "CFBundleShortVersionString").as_deref(),
            Some("0.1.0")
        );
        assert_eq!(
            plist_string_after_key(xml, "CFBundleVersion").as_deref(),
            Some("1")
        );
    }
}
