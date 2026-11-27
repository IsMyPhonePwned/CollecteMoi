//! CollecteMoi admin terminal dashboard (Ratatui).
//!
//! Runs as a second client alongside the web UI at `/admin` — same API, same cookie login.

mod app;
mod client;
mod ui;

use std::fs::OpenOptions;
use std::io::{self, BufRead, BufReader, Write};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use app::{App, Tab};
use client::AdminClient;

#[derive(Parser, Debug)]
#[command(
    name = "collectemoi-tui",
    about = "Terminal admin dashboard for CollecteMoi (runs alongside the web UI)"
)]
struct Cli {
    /// API base URL (HTTPS lab default).
    #[arg(long, default_value = "https://127.0.0.1:8080")]
    server: String,

    /// Admin password (prompted if omitted). Prefer env handoff from collectemoictl.
    #[arg(long, allow_hyphen_values = true)]
    password: Option<String>,

    /// TOTP code when MFA is enabled (prompted if needed).
    #[arg(long, allow_hyphen_values = true)]
    totp: Option<String>,

    /// Accept invalid TLS certificates (lab self-signed).
    #[arg(long, default_value_t = true)]
    insecure: bool,

    /// Poll interval in seconds.
    #[arg(long, default_value_t = 3)]
    poll_secs: u64,

    /// Login, fetch one snapshot, print a one-line summary, and exit (no TTY needed).
    #[arg(long, default_value_t = false)]
    check: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let client = AdminClient::connect(&cli.server, cli.insecure)?;

    // Pre-flight health (works even before login).
    match client.healthz() {
        Ok((200, body)) if body.eq_ignore_ascii_case("ok") => {}
        Ok((code, body)) => bail!("API not healthy: HTTP {code} body={body:?} at {}", cli.server),
        Err(e) => bail!("cannot reach {} ({e}) — start the lab with ./scripts/collectemoictl start", cli.server),
    }

    let password = resolve_password(cli.password)?;
    let totp = resolve_totp(cli.totp);

    if let Err(e) = client.login(&password, totp.as_deref()) {
        // Drop the password from the error chain; add a recovery hint.
        let msg = e.to_string();
        if msg.contains("invalid username or password") {
            bail!(
                "invalid username or password\n\
                 Hint: the lab password may have been reset during recovery.\n\
                 Set a new one with:  ./scripts/collectemoictl reset-password\n\
                 Then retry:          ./scripts/collectemoictl tui"
            );
        }
        return Err(e).context("admin login");
    }

    let mut app = App::new(client, cli.poll_secs);
    app.refresh().context("initial refresh")?;

    if cli.check {
        let o = &app.snap.overview;
        println!(
            "collectemoi-tui check ok · health={} · devices={} · jobs_pending={} · companies={} · uploads={}",
            if app.snap.health_ok { "ok" } else { "bad" },
            app::n(o, "devices"),
            app::n(o, "jobs_pending"),
            app::n(o, "companies"),
            app.snap.uploads.len(),
        );
        return Ok(());
    }

    enable_raw_mode().context("enable raw mode (need a real terminal)")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_loop(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

/// Resolve password: `--password` → `COLLECTEMOI_TUI_PASSWORD` env → interactive `/dev/tty`.
fn resolve_password(cli: Option<String>) -> Result<String> {
    if let Some(p) = cli.filter(|s| !s.is_empty()) {
        return Ok(p);
    }
    if let Ok(p) = std::env::var("COLLECTEMOI_TUI_PASSWORD") {
        // Swallow after read so child tools / ps dumps are less likely to retain it.
        std::env::remove_var("COLLECTEMOI_TUI_PASSWORD");
        if !p.is_empty() {
            return Ok(p);
        }
    }
    read_password_interactive()
}

fn resolve_totp(cli: Option<String>) -> Option<String> {
    if let Some(t) = cli.filter(|s| !s.is_empty()) {
        return Some(t);
    }
    if let Ok(t) = std::env::var("COLLECTEMOI_TUI_TOTP") {
        std::env::remove_var("COLLECTEMOI_TUI_TOTP");
        if !t.is_empty() {
            return Some(t);
        }
    }
    None
}

/// Read the admin password from the controlling terminal (`/dev/tty`), not stdin.
/// Stdin is often empty/EOF after `collectemoictl` build steps or non-TTY wrappers.
fn read_password_interactive() -> Result<String> {
    // Preferred: rpassword opens /dev/tty and disables echo.
    match rpassword::prompt_password("Admin password: ") {
        Ok(p) if !p.is_empty() => return Ok(p),
        Ok(_) => {}
        Err(e) => {
            eprintln!("note: password prompt via rpassword failed ({e}); trying /dev/tty");
        }
    }

    let mut tty = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .context("open /dev/tty for password (use --password if no TTY)")?;
    write!(tty, "Admin password: ").context("write password prompt")?;
    tty.flush()?;
    // Echo remains on for this fallback — better than a silent empty read.
    let mut line = String::new();
    BufReader::new(&mut tty)
        .read_line(&mut line)
        .context("read password from /dev/tty")?;
    let _ = writeln!(tty);
    let password = line.trim_end_matches(['\r', '\n']).to_string();
    if password.is_empty() {
        bail!("password required — re-run with: collectemoictl tui --password '…'");
    }
    Ok(password)
}

fn run_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|f| ui::draw(f, app))?;

        let timeout = Duration::from_millis(200);
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => {
                        app.should_quit = true;
                    }
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.should_quit = true;
                    }
                    KeyCode::Char('r') => {
                        let _ = app.refresh();
                    }
                    KeyCode::Tab | KeyCode::Right => app.set_tab(app.tab.next()),
                    KeyCode::BackTab | KeyCode::Left => app.set_tab(app.tab.prev()),
                    KeyCode::Up | KeyCode::Char('k') => app.scroll_by(-1),
                    KeyCode::Down | KeyCode::Char('j') => app.scroll_by(1),
                    KeyCode::PageUp => app.scroll_by(-10),
                    KeyCode::PageDown => app.scroll_by(10),
                    KeyCode::Char(c) => {
                        if let Some(tab) = Tab::from_digit(c) {
                            app.set_tab(tab);
                        }
                    }
                    _ => {}
                }
            }
        }

        if app.should_quit {
            break;
        }
        app.tick_refresh();
    }
    Ok(())
}
