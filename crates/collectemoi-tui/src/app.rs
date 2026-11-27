//! App state, tabs, and refresh logic for the admin TUI.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::Value;

use crate::client::AdminClient;

/// Samples kept for RX/TX sparklines (≈3–5 min at default 3s poll).
pub const NET_HISTORY_LEN: usize = 72;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Pipeline,
    Devices,
    Companies,
    Access,
}

impl Tab {
    pub const ALL: [Tab; 5] = [
        Tab::Overview,
        Tab::Pipeline,
        Tab::Devices,
        Tab::Companies,
        Tab::Access,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Pipeline => "Pipeline",
            Tab::Devices => "Devices",
            Tab::Companies => "Companies",
            Tab::Access => "Access",
        }
    }

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        let i = Self::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Self::ALL[(i + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    pub fn from_digit(d: char) -> Option<Self> {
        match d {
            '1' => Some(Tab::Overview),
            '2' => Some(Tab::Pipeline),
            '3' => Some(Tab::Devices),
            '4' => Some(Tab::Companies),
            '5' => Some(Tab::Access),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub health_ok: bool,
    pub overview: Value,
    pub devices: Vec<Value>,
    pub uploads: Vec<Value>,
    pub jobs: Vec<Value>,
    pub companies: Vec<Value>,
    pub access_stats: Value,
    pub access_events: Vec<Value>,
    pub error: Option<String>,
}

pub struct App {
    pub client: AdminClient,
    pub tab: Tab,
    pub snap: Snapshot,
    pub scroll: usize,
    pub poll_every: Duration,
    pub last_poll: Instant,
    pub status_line: String,
    pub should_quit: bool,
    /// Rolling RX bytes/sec samples for sparklines.
    pub rx_history: VecDeque<u64>,
    /// Rolling TX bytes/sec samples for sparklines.
    pub tx_history: VecDeque<u64>,
    /// Peak RX seen this session (gauge scale).
    pub rx_peak: u64,
    /// Peak TX seen this session (gauge scale).
    pub tx_peak: u64,
}

impl App {
    pub fn new(client: AdminClient, poll_secs: u64) -> Self {
        Self {
            client,
            tab: Tab::Overview,
            snap: Snapshot::default(),
            scroll: 0,
            poll_every: Duration::from_secs(poll_secs.max(1)),
            last_poll: Instant::now() - Duration::from_secs(3600),
            status_line: "connecting…".into(),
            should_quit: false,
            rx_history: VecDeque::with_capacity(NET_HISTORY_LEN),
            tx_history: VecDeque::with_capacity(NET_HISTORY_LEN),
            rx_peak: 1,
            tx_peak: 1,
        }
    }

    pub fn tick_refresh(&mut self) {
        if self.last_poll.elapsed() >= self.poll_every {
            let _ = self.refresh();
        }
    }

    pub fn refresh(&mut self) -> Result<()> {
        self.status_line = "refreshing…".into();
        match self.fetch_all() {
            Ok(snap) => {
                let n_dev = snap.devices.len();
                let n_jobs = snap.jobs.len();
                let up = snap
                    .overview
                    .get("uptime_secs")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let bw = snap
                    .overview
                    .get("bandwidth")
                    .cloned()
                    .unwrap_or(Value::Null);
                let rx = n(&bw, "rx_bps");
                let tx = n(&bw, "tx_bps");
                self.push_net_sample(rx, tx);
                self.status_line = format!(
                    "ok · up {} · ↓{}/s ↑{}/s · {} devices · {} jobs · refreshed {}",
                    format_uptime(up),
                    format_bytes(rx),
                    format_bytes(tx),
                    n_dev,
                    n_jobs,
                    chrono_now()
                );
                self.snap = snap;
                self.last_poll = Instant::now();
                Ok(())
            }
            Err(e) => {
                self.snap.error = Some(e.to_string());
                self.status_line = format!("error · {e}");
                self.last_poll = Instant::now();
                Err(e)
            }
        }
    }

    fn push_net_sample(&mut self, rx: u64, tx: u64) {
        if self.rx_history.len() >= NET_HISTORY_LEN {
            self.rx_history.pop_front();
        }
        if self.tx_history.len() >= NET_HISTORY_LEN {
            self.tx_history.pop_front();
        }
        self.rx_history.push_back(rx);
        self.tx_history.push_back(tx);
        self.rx_peak = self.rx_peak.max(rx).max(1);
        self.tx_peak = self.tx_peak.max(tx).max(1);
    }

    fn fetch_all(&self) -> Result<Snapshot> {
        let (hstatus, hbody) = self.client.healthz()?;
        let overview = self.client.get_json("v1/admin/overview")?;
        let devices = arr(self.client.get_json("v1/admin/devices")?, "devices");
        let uploads = arr(self.client.get_json("v1/admin/uploads?limit=80")?, "uploads");
        let jobs = arr(self.client.get_json("v1/admin/jobs")?, "jobs");
        let companies = arr(self.client.get_json("v1/admin/companies")?, "companies");
        let access_stats = self.client.get_json("v1/admin/access-stats")?;
        let access_events = arr(
            self.client.get_json("v1/admin/access-log?limit=60")?,
            "events",
        );
        Ok(Snapshot {
            health_ok: hstatus == 200 && hbody.eq_ignore_ascii_case("ok"),
            overview,
            devices,
            uploads,
            jobs,
            companies,
            access_stats,
            access_events,
            error: None,
        })
    }

    pub fn scroll_by(&mut self, delta: isize) {
        let max = self.list_len().saturating_sub(1);
        if delta < 0 {
            self.scroll = self.scroll.saturating_sub((-delta) as usize);
        } else {
            self.scroll = (self.scroll + delta as usize).min(max);
        }
    }

    pub fn list_len(&self) -> usize {
        match self.tab {
            Tab::Overview => 1,
            Tab::Pipeline => self.snap.uploads.len().max(self.snap.jobs.len()),
            Tab::Devices => self.snap.devices.len(),
            Tab::Companies => self.snap.companies.len(),
            Tab::Access => self.snap.access_events.len(),
        }
    }

    pub fn set_tab(&mut self, tab: Tab) {
        self.tab = tab;
        self.scroll = 0;
    }
}

fn arr(v: Value, key: &str) -> Vec<Value> {
    v.get(key)
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default()
}

pub fn format_uptime(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        if m == 0 {
            format!("{h}h")
        } else {
            format!("{h}h{m}m")
        }
    }
}

fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mins = (secs / 60) % 60;
    let hours = (secs / 3600) % 24;
    format!("{hours:02}:{mins:02}")
}

pub fn s(v: &Value, key: &str) -> String {
    v.get(key)
        .map(|x| match x {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Null => "—".into(),
            other => other.to_string(),
        })
        .unwrap_or_else(|| "—".into())
}

pub fn n(v: &Value, key: &str) -> u64 {
    v.get(key)
        .and_then(|x| x.as_u64().or_else(|| x.as_i64().map(|i| i.max(0) as u64)))
        .unwrap_or(0)
}

pub fn short(s: &str, max: usize) -> String {
    let t = s.trim();
    if t.chars().count() <= max {
        t.to_string()
    } else {
        let take = max.saturating_sub(1);
        format!("{}…", t.chars().take(take).collect::<String>())
    }
}

pub fn format_bytes(n: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let x = n as f64;
    if x >= GB {
        format!("{:.1}G", x / GB)
    } else if x >= MB {
        format!("{:.1}M", x / MB)
    } else if x >= KB {
        format!("{:.0}K", x / KB)
    } else {
        format!("{n}B")
    }
}
