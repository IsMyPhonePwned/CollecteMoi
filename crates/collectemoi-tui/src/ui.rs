//! Ratatui views for the admin dashboard.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Cell, Gauge, Paragraph, Row, Sparkline, Table, Tabs, Wrap,
};
use ratatui::Frame;

use crate::app::{format_bytes, format_uptime, n, s, short, App, Tab};

const ACCENT: Color = Color::Rgb(46, 196, 160);
const MUTED: Color = Color::Rgb(120, 140, 132);
const WARN: Color = Color::Rgb(224, 160, 74);
const BAD: Color = Color::Rgb(224, 96, 96);
const OK: Color = Color::Rgb(80, 200, 140);

pub fn draw(f: &mut Frame, app: &App) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(1),
        ])
        .split(f.area());

    draw_header(f, root[0], app);
    draw_tabs(f, root[1], app);
    match app.tab {
        Tab::Overview => draw_overview(f, root[2], app),
        Tab::Pipeline => draw_pipeline(f, root[2], app),
        Tab::Devices => draw_devices(f, root[2], app),
        Tab::Companies => draw_companies(f, root[2], app),
        Tab::Access => draw_access(f, root[2], app),
    }
    draw_footer(f, root[3], app);
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let health = if app.snap.health_ok {
        Span::styled("● health", Style::default().fg(OK))
    } else {
        Span::styled("● health", Style::default().fg(BAD))
    };
    let ver = s(&app.snap.overview, "version");
    let up = format_uptime(n(&app.snap.overview, "uptime_secs"));
    let bw = app
        .snap
        .overview
        .get("bandwidth")
        .cloned()
        .unwrap_or(serde_json::json!({}));
    let net = format!(
        "↓{}/s ↑{}/s",
        format_bytes(n(&bw, "rx_bps")),
        format_bytes(n(&bw, "tx_bps"))
    );
    let title = Line::from(vec![
        Span::styled(" CollecteMoi ", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::styled("lab TUI", Style::default().fg(MUTED)),
        Span::raw("  "),
        health,
        Span::raw("  "),
        Span::styled(format!("v{ver}"), Style::default().fg(MUTED)),
        Span::raw("  "),
        Span::styled(format!("up {up}"), Style::default().fg(MUTED)),
        Span::raw("  "),
        Span::styled(net, Style::default().fg(ACCENT)),
    ]);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(40, 60, 52)))
        .title(title);
    let rx_total = format_bytes(n(&bw, "rx_total"));
    let tx_total = format_bytes(n(&bw, "tx_total"));
    let inner = Paragraph::new(Line::from(vec![
        Span::styled(
            format!("  web /admin · totals RX {rx_total}  TX {tx_total} · access {}", n(&app.snap.overview, "access_total")),
            Style::default().fg(MUTED),
        ),
    ]))
    .block(block);
    f.render_widget(inner, area);
}

fn draw_tabs(f: &mut Frame, area: Rect, app: &App) {
    let titles: Vec<Line> = Tab::ALL
        .iter()
        .enumerate()
        .map(|(i, t)| {
            Line::from(format!(" {}.{} ", i + 1, t.title()))
        })
        .collect();
    let selected = Tab::ALL.iter().position(|t| *t == app.tab).unwrap_or(0);
    let tabs = Tabs::new(titles)
        .select(selected)
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Rgb(40, 60, 52))))
        .style(Style::default().fg(MUTED))
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD),
        )
        .divider("│");
    f.render_widget(tabs, area);
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let help = " Tab/←→ switch · ↑↓ scroll · r refresh · q quit ";
    let line = Line::from(vec![
        Span::styled(help, Style::default().fg(MUTED)),
        Span::raw(" "),
        Span::styled(&app.status_line, Style::default().fg(ACCENT)),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn draw_overview(f: &mut Frame, area: Rect, app: &App) {
    let o = &app.snap.overview;
    let bw = o.get("bandwidth").cloned().unwrap_or(serde_json::json!({}));
    let rx_bps = n(&bw, "rx_bps");
    let tx_bps = n(&bw, "tx_bps");
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7),
            Constraint::Length(3),
            Constraint::Length(9),
            Constraint::Min(4),
        ])
        .split(area);

    let kpis = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(14),
            Constraint::Percentage(14),
            Constraint::Percentage(14),
            Constraint::Percentage(14),
            Constraint::Percentage(14),
            Constraint::Percentage(15),
            Constraint::Percentage(15),
        ])
        .split(chunks[0]);

    kpi(f, kpis[0], "Devices", &n(o, "devices").to_string(), OK);
    kpi(f, kpis[1], "Blobs", &n(o, "blobs").to_string(), ACCENT);
    kpi(
        f,
        kpis[2],
        "Uploads",
        &format!("{}/{}", n(o, "uploads_complete"), n(o, "uploads")),
        MUTED,
    );
    let pending = n(o, "jobs_pending");
    kpi(
        f,
        kpis[3],
        "Jobs pending",
        &pending.to_string(),
        if pending > 0 { WARN } else { OK },
    );
    kpi(f, kpis[4], "Companies", &n(o, "companies").to_string(), ACCENT);
    kpi(
        f,
        kpis[5],
        "RX",
        &format!("{}/s", format_bytes(rx_bps)),
        if rx_bps > 0 { OK } else { MUTED },
    );
    kpi(
        f,
        kpis[6],
        "TX",
        &format!("{}/s", format_bytes(tx_bps)),
        if tx_bps > 0 { ACCENT } else { MUTED },
    );

    let banned = n(o, "banned_keys");
    let seal = o
        .get("settings")
        .and_then(|s| s.get("lab_seal"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let cmdev = o
        .get("settings")
        .and_then(|s| s.get("allow_dev_envelope"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let attn = Line::from(vec![
        Span::styled(" Attention  ", Style::default().fg(MUTED).add_modifier(Modifier::BOLD)),
        if pending > 0 {
            Span::styled(format!("{pending} pending jobs  "), Style::default().fg(WARN))
        } else {
            Span::styled("queue clear  ", Style::default().fg(OK))
        },
        if banned > 0 {
            Span::styled(format!("{banned} banned keys  "), Style::default().fg(WARN))
        } else {
            Span::raw("")
        },
        Span::styled(
            if seal { "lab_seal on  " } else { "lab_seal OFF  " },
            Style::default().fg(if seal { OK } else { WARN }),
        ),
        Span::styled(
            if cmdev { "CMDEV allowed" } else { "CMDEV blocked" },
            Style::default().fg(if cmdev { WARN } else { MUTED }),
        ),
    ]);
    f.render_widget(
        Paragraph::new(attn).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(40, 60, 52))),
        ),
        chunks[1],
    );

    draw_network_panel(f, chunks[2], app, &bw);

    let kinds = app
        .snap
        .access_stats
        .get("by_kind")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let max = kinds
        .iter()
        .filter_map(|k| k.get("count").and_then(|c| c.as_u64()))
        .max()
        .unwrap_or(1)
        .max(1);
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        "API traffic (by kind)",
        Style::default().fg(MUTED).add_modifier(Modifier::BOLD),
    ))];
    for k in kinds.iter().take(8) {
        let kind = s(k, "kind");
        let count = n(k, "count");
        let pct = ((count as f64 / max as f64) * 24.0).round() as usize;
        let bar = "█".repeat(pct) + &"░".repeat(24usize.saturating_sub(pct));
        lines.push(Line::from(vec![
            Span::styled(format!("{:>16} ", short(&kind, 16)), Style::default().fg(MUTED)),
            Span::styled(bar, Style::default().fg(ACCENT)),
            Span::styled(format!(" {count}"), Style::default().fg(Color::White)),
        ]));
    }
    if lines.len() == 1 {
        lines.push(Line::from(Span::styled("  no traffic yet", Style::default().fg(MUTED))));
    }
    if let Some(err) = &app.snap.error {
        lines.push(Line::from(Span::styled(
            format!("  last error: {err}"),
            Style::default().fg(BAD),
        )));
    }
    f.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Live pulse ")
                    .border_style(Style::default().fg(Color::Rgb(40, 60, 52))),
            )
            .wrap(Wrap { trim: false }),
        chunks[3],
    );
}

fn draw_network_panel(f: &mut Frame, area: Rect, app: &App, bw: &serde_json::Value) {
    let rx_bps = n(bw, "rx_bps");
    let tx_bps = n(bw, "tx_bps");
    let rx_total = n(bw, "rx_total");
    let tx_total = n(bw, "tx_total");
    let window = n(bw, "window_secs").max(1);

    let outer = Block::default()
        .borders(Borders::ALL)
        .title(format!(
            " Network  ↓{}/s  ↑{}/s  · totals ↓{} ↑{}  · {}s window · {} samples ",
            format_bytes(rx_bps),
            format_bytes(tx_bps),
            format_bytes(rx_total),
            format_bytes(tx_total),
            window,
            app.rx_history.len().max(app.tx_history.len()),
        ))
        .border_style(Style::default().fg(Color::Rgb(40, 60, 52)));
    let inner = outer.inner(area);
    f.render_widget(outer, area);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(inner);

    draw_net_channel(
        f,
        cols[0],
        "RX ↓",
        &app.rx_history,
        rx_bps,
        app.rx_peak,
        OK,
        Color::Rgb(30, 90, 70),
    );
    draw_net_channel(
        f,
        cols[1],
        "TX ↑",
        &app.tx_history,
        tx_bps,
        app.tx_peak,
        ACCENT,
        Color::Rgb(28, 70, 88),
    );
}

fn draw_net_channel(
    f: &mut Frame,
    area: Rect,
    label: &str,
    history: &std::collections::VecDeque<u64>,
    current: u64,
    peak: u64,
    color: Color,
    gauge_bg: Color,
) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(2), Constraint::Length(1)])
        .split(area);

    let peak = peak.max(1);
    let ratio = (current as f64 / peak as f64).clamp(0.0, 1.0);
    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(color).bg(gauge_bg))
        .ratio(ratio)
        .label(format!(
            "{label}  {}/s  (peak {}/s)",
            format_bytes(current),
            format_bytes(peak)
        ))
        .style(Style::default().fg(MUTED));
    f.render_widget(gauge, rows[0]);

    let data: Vec<u64> = if history.is_empty() {
        vec![0]
    } else {
        history.iter().copied().collect()
    };
    let spark = Sparkline::default()
        .data(&data)
        .style(Style::default().fg(color))
        .max(peak);
    f.render_widget(spark, rows[1]);

    // Mini ASCII density strip under the sparkline for terminals that flatten sparklines.
    let width = rows[2].width.max(1) as usize;
    let strip = net_density_strip(history, peak, width);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(strip, Style::default().fg(color)))),
        rows[2],
    );
}

fn net_density_strip(history: &std::collections::VecDeque<u64>, peak: u64, width: usize) -> String {
    const BLOCKS: &[char] = &[' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    if history.is_empty() || width == 0 {
        return "· waiting for samples…".to_string();
    }
    let peak = peak.max(1) as f64;
    let n = history.len();
    let mut out = String::with_capacity(width);
    for i in 0..width {
        let idx = if width == 1 {
            n - 1
        } else {
            (i * (n - 1)) / (width - 1)
        };
        let v = *history.get(idx).unwrap_or(&0) as f64;
        let level = ((v / peak) * (BLOCKS.len() - 1) as f64).round() as usize;
        out.push(BLOCKS[level.min(BLOCKS.len() - 1)]);
    }
    out
}

fn kpi(f: &mut Frame, area: Rect, label: &str, value: &str, color: Color) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(40, 60, 52)))
        .title(Span::styled(format!(" {label} "), Style::default().fg(MUTED)));
    let p = Paragraph::new(Line::from(Span::styled(
        format!("\n {value}"),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )))
    .block(block);
    f.render_widget(p, area);
}

fn draw_pipeline(f: &mut Frame, area: Rect, app: &App) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

    let upload_rows = app.snap.uploads.iter().enumerate().map(|(i, u)| {
        let complete = u.get("complete").and_then(|v| v.as_bool()).unwrap_or(false);
        let status = if complete { "done" } else { "stream" };
        let style = if i == app.scroll {
            Style::default().bg(Color::Rgb(28, 48, 42)).fg(Color::White)
        } else if complete {
            Style::default().fg(OK)
        } else {
            Style::default().fg(WARN)
        };
        Row::new(vec![
            Cell::from(short(&s(u, "artefact"), 18)),
            Cell::from(short(&s(u, "upload_id"), 12)),
            Cell::from(status),
            Cell::from(format_bytes(n(u, "total_size"))),
        ])
        .style(style)
    });
    let uploads = Table::new(
        upload_rows,
        [
            Constraint::Percentage(35),
            Constraint::Percentage(30),
            Constraint::Length(8),
            Constraint::Length(10),
        ],
    )
    .header(
        Row::new(["Artefact", "Upload", "State", "Size"]).style(
            Style::default()
                .fg(MUTED)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Uploads ({}) ", app.snap.uploads.len()))
            .border_style(Style::default().fg(Color::Rgb(40, 60, 52))),
    );

    let job_rows = app.snap.jobs.iter().enumerate().map(|(i, j)| {
        let st = s(j, "status").to_ascii_lowercase();
        let style = if i == app.scroll {
            Style::default().bg(Color::Rgb(28, 48, 42)).fg(Color::White)
        } else if st == "failed" {
            Style::default().fg(BAD)
        } else if st == "done" {
            Style::default().fg(OK)
        } else {
            Style::default().fg(WARN)
        };
        Row::new(vec![
            Cell::from(short(&s(j, "artefact"), 16)),
            Cell::from(short(&st, 10)),
            Cell::from(short(&s(j, "upload_id"), 12)),
        ])
        .style(style)
    });
    let jobs = Table::new(
        job_rows,
        [
            Constraint::Percentage(40),
            Constraint::Percentage(25),
            Constraint::Percentage(35),
        ],
    )
    .header(
        Row::new(["Artefact", "Status", "Upload"]).style(
            Style::default()
                .fg(MUTED)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Jobs ({}) ", app.snap.jobs.len()))
            .border_style(Style::default().fg(Color::Rgb(40, 60, 52))),
    );

    f.render_widget(uploads, cols[0]);
    f.render_widget(jobs, cols[1]);
}

fn draw_devices(f: &mut Frame, area: Rect, app: &App) {
    let rows = app.snap.devices.iter().enumerate().map(|(i, d)| {
        let banned = d.get("banned").and_then(|v| v.as_bool()).unwrap_or(false)
            || d.get("public_key_banned")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
        let style = if i == app.scroll {
            Style::default().bg(Color::Rgb(28, 48, 42)).fg(Color::White)
        } else if banned {
            Style::default().fg(BAD)
        } else {
            Style::default().fg(Color::White)
        };
        Row::new(vec![
            Cell::from(short(&s(d, "email"), 28)),
            Cell::from(short(&s(d, "company_id"), 12)),
            Cell::from(short(&s(d, "platform"), 10)),
            Cell::from(short(&s(d, "model"), 14)),
            Cell::from(if banned { "banned" } else { "ok" }),
            Cell::from(short(&s(d, "device_id"), 14)),
        ])
        .style(style)
    });
    let table = Table::new(
        rows,
        [
            Constraint::Percentage(28),
            Constraint::Percentage(14),
            Constraint::Percentage(12),
            Constraint::Percentage(16),
            Constraint::Length(8),
            Constraint::Percentage(18),
        ],
    )
    .header(
        Row::new(["Email", "Company", "Platform", "Model", "State", "Device"]).style(
            Style::default()
                .fg(MUTED)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Devices ({}) ", app.snap.devices.len()))
            .border_style(Style::default().fg(Color::Rgb(40, 60, 52))),
    );
    f.render_widget(table, area);
}

fn draw_companies(f: &mut Frame, area: Rect, app: &App) {
    let rows = app.snap.companies.iter().enumerate().map(|(i, c)| {
        let enabled = c.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
        let style = if i == app.scroll {
            Style::default().bg(Color::Rgb(28, 48, 42)).fg(Color::White)
        } else if !enabled {
            Style::default().fg(WARN)
        } else {
            Style::default().fg(Color::White)
        };
        let domains = c
            .get("domains")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str())
                    .take(3)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let last_build = short(&s(c, "last_build_at"), 16);
        Row::new(vec![
            Cell::from(short(&s(c, "name"), 18)),
            Cell::from(short(&s(c, "id"), 10)),
            Cell::from(short(&domains, 22)),
            Cell::from(n(c, "device_count").to_string()),
            Cell::from(n(c, "build_count").to_string()),
            Cell::from(if last_build == "—" {
                "—".into()
            } else {
                last_build
            }),
            Cell::from(if enabled { "active" } else { "paused" }),
            Cell::from(short(&s(c, "color"), 8)),
        ])
        .style(style)
    });
    let table = Table::new(
        rows,
        [
            Constraint::Percentage(18),
            Constraint::Percentage(12),
            Constraint::Percentage(22),
            Constraint::Length(5),
            Constraint::Length(6),
            Constraint::Length(17),
            Constraint::Length(8),
            Constraint::Length(8),
        ],
    )
    .header(
        Row::new(["Name", "ID", "Domains", "Dev", "Build", "Generated", "State", "Color"]).style(
            Style::default()
                .fg(MUTED)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Companies ({}) ", app.snap.companies.len()))
            .border_style(Style::default().fg(Color::Rgb(40, 60, 52))),
    );
    f.render_widget(table, area);
}

fn draw_access(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(9), Constraint::Length(4), Constraint::Min(5)])
        .split(area);

    let bw = app
        .snap
        .overview
        .get("bandwidth")
        .cloned()
        .unwrap_or(serde_json::json!({}));
    draw_network_panel(f, chunks[0], app, &bw);

    let remotes = app
        .snap
        .access_stats
        .get("by_remote")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut data: Vec<u64> = remotes
        .iter()
        .rev()
        .take(48)
        .filter_map(|r| r.get("count").and_then(|c| c.as_u64()))
        .collect();
    if data.is_empty() {
        data.push(0);
    }
    let spark = Sparkline::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Remotes (hit counts) ")
                .border_style(Style::default().fg(Color::Rgb(40, 60, 52))),
        )
        .data(&data)
        .style(Style::default().fg(WARN));
    f.render_widget(spark, chunks[1]);

    let rows = app.snap.access_events.iter().enumerate().map(|(i, e)| {
        let status = n(e, "status");
        let style = if i == app.scroll {
            Style::default().bg(Color::Rgb(28, 48, 42)).fg(Color::White)
        } else if status >= 500 {
            Style::default().fg(BAD)
        } else if status >= 400 {
            Style::default().fg(WARN)
        } else {
            Style::default().fg(Color::White)
        };
        Row::new(vec![
            Cell::from(short(&s(e, "kind"), 14)),
            Cell::from(short(&format!("{} {}", s(e, "method"), s(e, "path")), 36)),
            Cell::from(status.to_string()),
            Cell::from(format!("{}ms", n(e, "duration_ms"))),
            Cell::from(short(&s(e, "remote"), 16)),
        ])
        .style(style)
    });
    let table = Table::new(
        rows,
        [
            Constraint::Percentage(16),
            Constraint::Percentage(42),
            Constraint::Length(6),
            Constraint::Length(8),
            Constraint::Percentage(18),
        ],
    )
    .header(
        Row::new(["Kind", "Request", "Code", "ms", "Remote"]).style(
            Style::default()
                .fg(MUTED)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Recent access ({}) ", app.snap.access_events.len()))
            .border_style(Style::default().fg(Color::Rgb(40, 60, 52))),
    );
    f.render_widget(table, chunks[2]);
}
