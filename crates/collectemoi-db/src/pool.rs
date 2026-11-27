//! Small sync connection pool for SQLite (WAL).
//!
//! One `Mutex<Connection>` serialises the whole API under upload load. A pool
//! lets readers proceed while a writer holds the SQLite write lock, and lets
//! multiple writers queue on `busy_timeout` instead of a process-wide mutex.

use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use rusqlite::Connection;

const DEFAULT_BUSY_MS: u32 = 30_000;

pub(crate) fn apply_pragmas(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         PRAGMA foreign_keys=ON;
         PRAGMA temp_store=MEMORY;
         PRAGMA busy_timeout=30000;
         PRAGMA cache_size=-65536;
         PRAGMA mmap_size=268435456;
         PRAGMA wal_autocheckpoint=1000;",
    )?;
    let _ = DEFAULT_BUSY_MS;
    Ok(())
}

pub(crate) fn open_connection(path: &Path) -> Result<Connection> {
    let conn =
        Connection::open(path).with_context(|| format!("open sqlite {}", path.display()))?;
    apply_pragmas(&conn)?;
    Ok(conn)
}

struct PoolInner {
    path: PathBuf,
    idle: Mutex<Vec<Connection>>,
    available: Condvar,
    /// Live connections currently checked out or idle.
    live: AtomicUsize,
    max_size: usize,
}

/// Cloneable pool of SQLite connections sharing one DB file.
#[derive(Clone)]
pub struct SqlitePool {
    inner: Arc<PoolInner>,
}

impl SqlitePool {
    pub fn new(path: PathBuf, seed: Connection, max_size: usize) -> Self {
        let max_size = max_size.clamp(2, 128);
        Self {
            inner: Arc::new(PoolInner {
                path,
                idle: Mutex::new(vec![seed]),
                available: Condvar::new(),
                live: AtomicUsize::new(1),
                max_size,
            }),
        }
    }

    pub fn max_size(&self) -> usize {
        self.inner.max_size
    }

    pub fn acquire(&self) -> Result<PooledConnection> {
        let timeout = Duration::from_millis(u64::from(DEFAULT_BUSY_MS));
        let deadline = std::time::Instant::now() + timeout;
        let mut idle = self.inner.idle.lock().unwrap();
        loop {
            if let Some(conn) = idle.pop() {
                return Ok(PooledConnection {
                    conn: Some(conn),
                    pool: self.inner.clone(),
                });
            }
            let live = self.inner.live.load(Ordering::SeqCst);
            if live < self.inner.max_size {
                self.inner.live.fetch_add(1, Ordering::SeqCst);
                drop(idle);
                match open_connection(&self.inner.path) {
                    Ok(conn) => {
                        return Ok(PooledConnection {
                            conn: Some(conn),
                            pool: self.inner.clone(),
                        });
                    }
                    Err(e) => {
                        self.inner.live.fetch_sub(1, Ordering::SeqCst);
                        self.inner.available.notify_one();
                        return Err(e);
                    }
                }
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                bail!(
                    "sqlite pool exhausted (max {}) after {}ms",
                    self.inner.max_size,
                    DEFAULT_BUSY_MS
                );
            }
            let (guard, wait) = self
                .inner
                .available
                .wait_timeout(idle, deadline.saturating_duration_since(now))
                .unwrap();
            idle = guard;
            if wait.timed_out() && idle.is_empty() {
                bail!(
                    "sqlite pool exhausted (max {}) after {}ms",
                    self.inner.max_size,
                    DEFAULT_BUSY_MS
                );
            }
        }
    }
}

pub struct PooledConnection {
    conn: Option<Connection>,
    pool: Arc<PoolInner>,
}

impl Drop for PooledConnection {
    fn drop(&mut self) {
        if let Some(conn) = self.conn.take() {
            {
                let mut idle = self.pool.idle.lock().unwrap();
                idle.push(conn);
            }
            self.pool.available.notify_one();
        }
    }
}

impl Deref for PooledConnection {
    type Target = Connection;
    fn deref(&self) -> &Self::Target {
        self.conn.as_ref().expect("pooled connection")
    }
}

impl DerefMut for PooledConnection {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.conn.as_mut().expect("pooled connection")
    }
}
