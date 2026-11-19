//! One-shot import of legacy JSON/JSONL files into SQLite when tables are empty.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::Result;
use rusqlite::{Connection, params};
use serde_json::Value;

use crate::Db;

impl Db {
    /// Import legacy on-disk catalogues if the SQL tables are empty.
    pub fn import_legacy_files(&self, data_dir: &Path) -> Result<ImportStats> {
        let conn = self.conn.lock().unwrap();
        let mut stats = ImportStats::default();

        if table_empty(&conn, "companies")? {
            stats.companies = import_json_array(
                &conn,
                &data_dir.join("companies.json"),
                |c, v| {
                    c.execute(
                        "INSERT OR IGNORE INTO companies (id, name, domains_json, case_id, enabled, updated_at, color)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        params![
                            v.get("id").and_then(|x| x.as_str()).unwrap_or(""),
                            v.get("name").and_then(|x| x.as_str()).unwrap_or(""),
                            v.get("domains").cloned().unwrap_or(Value::Array(vec![])).to_string(),
                            v.get("case_id").and_then(|x| x.as_str()).unwrap_or(""),
                            if v.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true) {
                                1
                            } else {
                                0
                            },
                            v.get("updated_at").and_then(|x| x.as_str()).unwrap_or(""),
                            v.get("color").and_then(|x| x.as_str()).unwrap_or(""),
                        ],
                    )?;
                    Ok(())
                },
            )?;
        }

        if table_empty(&conn, "lab_keys")? {
            stats.lab_keys = import_json_array(&conn, &data_dir.join("lab_keys.json"), |c, v| {
                c.execute(
                    "INSERT OR IGNORE INTO lab_keys (key, label, email, case_id, reusable, enabled, updated_at, company_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        v.get("key").and_then(|x| x.as_str()).unwrap_or(""),
                        v.get("label").and_then(|x| x.as_str()).unwrap_or(""),
                        v.get("email").and_then(|x| x.as_str()).unwrap_or(""),
                        v.get("case_id").and_then(|x| x.as_str()).unwrap_or(""),
                        if v.get("reusable").and_then(|x| x.as_bool()).unwrap_or(true) {
                            1
                        } else {
                            0
                        },
                        if v.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true) {
                            1
                        } else {
                            0
                        },
                        v.get("updated_at").and_then(|x| x.as_str()).unwrap_or(""),
                        v.get("company_id").and_then(|x| x.as_str()).unwrap_or(""),
                    ],
                )?;
                Ok(())
            })?;
        }

        if table_empty(&conn, "banned_public_keys")? {
            if let Ok(bytes) = fs::read(data_dir.join("banned_public_keys.json")) {
                if let Ok(map) = serde_json::from_slice::<serde_json::Map<String, Value>>(&bytes) {
                    for (k, v) in map {
                        let banned_at = v
                            .get("banned_at")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .to_string();
                        let reason = v
                            .get("reason")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .to_string();
                        conn.execute(
                            "INSERT OR IGNORE INTO banned_public_keys (public_key, banned_at, reason)
                             VALUES (?1, ?2, ?3)",
                            params![k, banned_at, reason],
                        )?;
                        stats.banned += 1;
                    }
                } else if let Ok(list) = serde_json::from_slice::<Vec<Value>>(&bytes) {
                    for v in list {
                        conn.execute(
                            "INSERT OR IGNORE INTO banned_public_keys (public_key, banned_at, reason)
                             VALUES (?1, ?2, ?3)",
                            params![
                                v.get("public_key").and_then(|x| x.as_str()).unwrap_or(""),
                                v.get("banned_at").and_then(|x| x.as_str()).unwrap_or(""),
                                v.get("reason").and_then(|x| x.as_str()).unwrap_or(""),
                            ],
                        )?;
                        stats.banned += 1;
                    }
                }
            }
        }

        if table_empty(&conn, "devices")? {
            let devices_dir = data_dir.join("devices");
            if devices_dir.is_dir() {
                for entry in fs::read_dir(&devices_dir)?.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|s| s.to_str()) != Some("json") {
                        continue;
                    }
                    let Ok(bytes) = fs::read(&path) else { continue };
                    let Ok(v) = serde_json::from_slice::<Value>(&bytes) else {
                        continue;
                    };
                    conn.execute(
                        "INSERT OR IGNORE INTO devices (
                            device_id, case_id, public_key, noise_key, noise_key_fingerprint,
                            platform, model, os_version, enrolled_at, banned, email, company_id
                         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                        params![
                            v.get("device_id").and_then(|x| x.as_str()).unwrap_or(""),
                            v.get("case_id").and_then(|x| x.as_str()).unwrap_or(""),
                            v.get("public_key").and_then(|x| x.as_str()).unwrap_or(""),
                            v.get("noise_key").and_then(|x| x.as_str()).unwrap_or(""),
                            v.get("noise_key_fingerprint")
                                .and_then(|x| x.as_str())
                                .unwrap_or(""),
                            v.get("platform").and_then(|x| x.as_str()).unwrap_or(""),
                            v.get("model").and_then(|x| x.as_str()).unwrap_or(""),
                            v.get("os_version").and_then(|x| x.as_str()).unwrap_or(""),
                            v.get("enrolled_at").and_then(|x| x.as_str()).unwrap_or(""),
                            if v.get("banned").and_then(|x| x.as_bool()).unwrap_or(false) {
                                1
                            } else {
                                0
                            },
                            v.get("email").and_then(|x| x.as_str()).unwrap_or(""),
                            v.get("company_id").and_then(|x| x.as_str()).unwrap_or(""),
                        ],
                    )?;
                    stats.devices += 1;
                }
            }
        }

        if table_empty(&conn, "uploads")? {
            stats.uploads = import_upload_metas(&conn, data_dir)?;
        }

        if table_empty(&conn, "blobs")? {
            stats.blobs = import_blob_metas(&conn, data_dir)?;
        }

        if table_empty(&conn, "jobs")? {
            stats.jobs = import_jobs(&conn, data_dir)?;
        }

        if table_empty(&conn, "access_events")? {
            stats.access = import_jsonl(&conn, &data_dir.join("access_log.jsonl"), |c, v| {
                c.execute(
                    "INSERT INTO access_events (
                            ts, method, path, status, duration_ms, remote, user_agent,
                            device_id, kind, resource_id
                         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                    params![
                        v.get("ts").and_then(|x| x.as_str()).unwrap_or(""),
                        v.get("method").and_then(|x| x.as_str()).unwrap_or(""),
                        v.get("path").and_then(|x| x.as_str()).unwrap_or(""),
                        v.get("status").and_then(|x| x.as_i64()).unwrap_or(0) as i32,
                        v.get("duration_ms").and_then(|x| x.as_i64()).unwrap_or(0) as i64,
                        v.get("remote").and_then(|x| x.as_str()).unwrap_or(""),
                        v.get("user_agent").and_then(|x| x.as_str()).unwrap_or(""),
                        v.get("device_id").and_then(|x| x.as_str()).unwrap_or(""),
                        v.get("kind").and_then(|x| x.as_str()).unwrap_or(""),
                        v.get("resource_id").and_then(|x| x.as_str()).unwrap_or(""),
                    ],
                )?;
                Ok(())
            })?;
        }

        if table_empty(&conn, "enrollment_events")? {
            stats.enrollment =
                import_jsonl(&conn, &data_dir.join("enrollment_log.jsonl"), |c, v| {
                    c.execute(
                        "INSERT INTO enrollment_events (
                            ts, event, email, company_id, challenge_id, device_id, lab_key, detail
                         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                        params![
                            v.get("ts").and_then(|x| x.as_i64()).unwrap_or(0),
                            v.get("event").and_then(|x| x.as_str()).unwrap_or(""),
                            v.get("email").and_then(|x| x.as_str()),
                            v.get("company_id").and_then(|x| x.as_str()),
                            v.get("challenge_id").and_then(|x| x.as_str()),
                            v.get("device_id").and_then(|x| x.as_str()),
                            v.get("lab_key").and_then(|x| x.as_str()),
                            v.get("detail").and_then(|x| x.as_str()),
                        ],
                    )?;
                    Ok(())
                })?;
        }

        Ok(stats)
    }
}

#[derive(Debug, Default)]
pub struct ImportStats {
    pub companies: usize,
    pub lab_keys: usize,
    pub banned: usize,
    pub devices: usize,
    pub uploads: usize,
    pub blobs: usize,
    pub jobs: usize,
    pub access: usize,
    pub enrollment: usize,
}

fn table_empty(conn: &Connection, table: &str) -> Result<bool> {
    // table name is an internal constant only — never user input.
    let sql = format!("SELECT COUNT(*) FROM {table}");
    let n: i64 = conn.query_row(&sql, [], |r| r.get(0))?;
    Ok(n == 0)
}

fn import_json_array(
    conn: &Connection,
    path: &Path,
    mut insert: impl FnMut(&Connection, &Value) -> Result<()>,
) -> Result<usize> {
    let Ok(bytes) = fs::read(path) else {
        return Ok(0);
    };
    let Ok(list) = serde_json::from_slice::<Vec<Value>>(&bytes) else {
        return Ok(0);
    };
    let mut n = 0;
    for v in &list {
        insert(conn, v)?;
        n += 1;
    }
    Ok(n)
}

fn import_jsonl(
    conn: &Connection,
    path: &Path,
    mut insert: impl FnMut(&Connection, &Value) -> Result<()>,
) -> Result<usize> {
    let Ok(f) = fs::File::open(path) else {
        return Ok(0);
    };
    let mut n = 0;
    for line in BufReader::new(f).lines().flatten() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        insert(conn, &v)?;
        n += 1;
    }
    Ok(n)
}

fn import_upload_metas(conn: &Connection, data_dir: &Path) -> Result<usize> {
    let mut n = 0;
    walk_meta_json(&data_dir.join("uploads"), &mut |v| {
        let received = v
            .get("received")
            .cloned()
            .unwrap_or(Value::Array(vec![]))
            .to_string();
        conn.execute(
            "INSERT OR IGNORE INTO uploads (
                upload_id, case_id, artefact, total_size, chunk_size, chunk_count,
                ciphertext_sha256, received_json, complete, created_at, completed_at
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                v.get("upload_id").and_then(|x| x.as_str()).unwrap_or(""),
                v.get("case_id").and_then(|x| x.as_str()).unwrap_or(""),
                artefact_str(v),
                v.get("total_size").and_then(|x| x.as_u64()).unwrap_or(0) as i64,
                v.get("chunk_size").and_then(|x| x.as_u64()).unwrap_or(0) as i64,
                v.get("chunk_count").and_then(|x| x.as_u64()).unwrap_or(0) as i64,
                v.get("ciphertext_sha256")
                    .and_then(|x| x.as_str())
                    .unwrap_or(""),
                received,
                if v.get("complete").and_then(|x| x.as_bool()).unwrap_or(false) {
                    1
                } else {
                    0
                },
                v.get("created_at").and_then(|x| x.as_str()).unwrap_or(""),
                v.get("completed_at").and_then(|x| x.as_str()).unwrap_or(""),
            ],
        )?;
        n += 1;
        Ok(())
    })?;
    Ok(n)
}

fn import_blob_metas(conn: &Connection, data_dir: &Path) -> Result<usize> {
    let mut n = 0;
    // Prefer index.jsonl
    let index = data_dir.join("blobs").join("index.jsonl");
    if index.is_file() {
        n = import_jsonl(conn, &index, |c, v| {
            insert_blob(c, v)?;
            Ok(())
        })?;
        if n > 0 {
            return Ok(n);
        }
    }
    walk_meta_json(&data_dir.join("blobs"), &mut |v| {
        insert_blob(conn, v)?;
        n += 1;
        Ok(())
    })?;
    Ok(n)
}

fn insert_blob(conn: &Connection, v: &Value) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO blobs (
            blob_id, upload_id, case_id, artefact, size, sha256, created_at, queued_at,
            device_public_key, filename, content_type
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![
            v.get("blob_id").and_then(|x| x.as_str()).unwrap_or(""),
            v.get("upload_id").and_then(|x| x.as_str()).unwrap_or(""),
            v.get("case_id").and_then(|x| x.as_str()).unwrap_or(""),
            v.get("artefact").and_then(|x| x.as_str()).unwrap_or(""),
            v.get("size").and_then(|x| x.as_u64()).unwrap_or(0) as i64,
            v.get("sha256").and_then(|x| x.as_str()).unwrap_or(""),
            v.get("created_at").and_then(|x| x.as_str()).unwrap_or(""),
            v.get("queued_at").and_then(|x| x.as_str()).unwrap_or(""),
            v.get("device_public_key")
                .and_then(|x| x.as_str())
                .unwrap_or(""),
            v.get("filename").and_then(|x| x.as_str()).unwrap_or(""),
            v.get("content_type")
                .and_then(|x| x.as_str())
                .unwrap_or("application/octet-stream"),
        ],
    )?;
    Ok(())
}

fn import_jobs(conn: &Connection, data_dir: &Path) -> Result<usize> {
    let dir = data_dir.join("jobs");
    if !dir.is_dir() {
        return Ok(0);
    }
    let mut n = 0;
    for entry in fs::read_dir(&dir)?.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let (status, read_path) = if name.ends_with(".json") {
            ("pending", path.clone())
        } else if name.ends_with(".json.claimed") {
            ("claimed", path.clone())
        } else if name.ends_with(".json.done") {
            ("done", path.clone())
        } else if name.ends_with(".json.failed") {
            ("failed", path.clone())
        } else {
            continue;
        };
        let Ok(bytes) = fs::read(&read_path) else {
            continue;
        };
        let Ok(v) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        // failed wrapper may nest job
        let job = if v.get("job").is_some() && v.get("upload_id").is_none() {
            continue;
        } else {
            v
        };
        let upload_id = job
            .get("upload_id")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if upload_id.is_empty() {
            continue;
        }
        conn.execute(
            "INSERT OR IGNORE INTO jobs (
                upload_id, case_id, artefact, envelope_path, device_public_key, queued_at, status
             ) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                upload_id,
                job.get("case_id").and_then(|x| x.as_str()).unwrap_or(""),
                job.get("artefact").and_then(|x| x.as_str()).unwrap_or(""),
                job.get("envelope_path")
                    .and_then(|x| x.as_str())
                    .unwrap_or(""),
                job.get("device_public_key")
                    .and_then(|x| x.as_str())
                    .unwrap_or(""),
                job.get("queued_at").and_then(|x| x.as_str()).unwrap_or(""),
                status,
            ],
        )?;
        n += 1;
    }
    Ok(n)
}

fn artefact_str(v: &Value) -> String {
    match v.get("artefact") {
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string().trim_matches('"').to_string(),
        None => String::new(),
    }
}

fn walk_meta_json(dir: &Path, visit: &mut dyn FnMut(&Value) -> Result<()>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let meta = path.join("meta.json");
            if meta.is_file() {
                if let Ok(bytes) = fs::read(&meta) {
                    if let Ok(v) = serde_json::from_slice::<Value>(&bytes) {
                        visit(&v)?;
                        continue;
                    }
                }
            }
            walk_meta_json(&path, visit)?;
        }
    }
    Ok(())
}
