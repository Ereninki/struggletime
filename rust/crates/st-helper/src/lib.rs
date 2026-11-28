//! Native `struggletime-helper` — starts the Rust daemon, manages API key / links.
mod adapters;

pub use adapters::{install_adapters, install_all_adapters, list_adapters, ADAPTER_NAMES};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use st_core::config::{
    daemon_log_path, daemon_pid_path, link_project, linked_paths, load_settings, normalize_server,
    pause_flag_path, runtime_home, save_settings, settings_path,
};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(name = "struggletime-helper", about = "StruggleTime CLI (native)")]
pub struct HelperCli {
    #[command(subcommand)]
    pub cmd: HelperCmd,
}

#[derive(Subcommand, Debug)]
pub enum HelperCmd {
    /// Save API key (st_…)
    Apikey {
        key: String,
    },
    /// Remove API key
    #[command(name = "clear-key")]
    ClearKey,
    /// Start the tracker daemon
    Start {
        #[arg(long)]
        repo: Option<PathBuf>,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        server: Option<String>,
        #[arg(long)]
        user: Option<String>,
    },
    /// Stop the tracker daemon
    Stop,
    /// Show status
    Status,
    Pause,
    Resume,
    /// Flush pending ledger uploads
    Sync,
    /// Link a local directory
    Link {
        #[arg(long)]
        repo: Option<PathBuf>,
        #[arg(long)]
        project: Option<String>,
        /// Fail if project id already exists (do not retarget / merge)
        #[arg(long, default_value_t = false)]
        create: bool,
    },
    /// Print settings path
    Config,
    /// Tail daemon log
    Logs {
        #[arg(long, default_value_t = 30)]
        lines: usize,
    },
    /// Show or set server URL
    Server {
        url: Option<String>,
    },
    /// List or install optional editor / agent adapters
    Adapters {
        #[command(subcommand)]
        action: Option<AdaptersAction>,
    },
}

#[derive(Subcommand, Debug)]
pub enum AdaptersAction {
    /// Show available adapters and install status
    List,
    /// Install adapters into editor / user paths (default: all)
    Install {
        /// Adapter names (vscode, neovim, claude-code, codex, hooks, …)
        names: Vec<String>,
    },
}

pub fn run(cli: HelperCli) -> Result<()> {
    fs::create_dir_all(runtime_home())?;
    match cli.cmd {
        HelperCmd::Apikey { key } => {
            let key = key.trim();
            if !key.starts_with("st_") {
                bail!("key should start with st_");
            }
            let mut s = load_settings()?;
            s.token = key.to_string();
            // Best-effort: ask /v1/auth/me
            if let Ok(handle) = fetch_handle(&s.server, key) {
                s.handle = handle;
            }
            save_settings(&s)?;
            println!("saved API key → {}", settings_path().display());
            if !s.handle.is_empty() {
                println!("handle @{}", s.handle);
            }
        }
        HelperCmd::ClearKey => {
            let mut s = load_settings()?;
            s.token.clear();
            save_settings(&s)?;
            println!("cleared API key");
        }
        HelperCmd::Start {
            repo,
            project,
            server,
            user,
        } => {
            let mut s = load_settings()?;
            if let Some(server) = server {
                s.server = normalize_server(&server);
            }
            if let Some(user) = user {
                s.handle = user;
            }
            if let Some(repo) = repo {
                let path = repo.canonicalize().unwrap_or(repo);
                link_project(&mut s, project.as_deref().unwrap_or(""), &path, false)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
            }
            save_settings(&s)?;
            if s.token.is_empty() && linked_empty(&s) {
                bail!("set an API key first (struggletime apikey <st_…>) or pass --repo");
            }
            start_daemon(&s)?;
        }
        HelperCmd::Stop => stop_daemon()?,
        HelperCmd::Status => print_status()?,
        HelperCmd::Pause => {
            fs::write(pause_flag_path(), b"1")?;
            println!("paused");
        }
        HelperCmd::Resume => {
            let _ = fs::remove_file(pause_flag_path());
            println!("resumed");
        }
        HelperCmd::Sync => {
            let s = load_settings()?;
            run_daemon_once(&s, true)?;
            println!("sync requested");
        }
        HelperCmd::Link {
            repo,
            project,
            create,
        } => {
            let mut s = load_settings()?;
            let path = repo
                .or_else(|| {
                    if s.repo.is_empty() {
                        None
                    } else {
                        Some(PathBuf::from(&s.repo))
                    }
                })
                .context("pass --repo PATH")?;
            let path = path.canonicalize().unwrap_or(path);
            match link_project(&mut s, project.as_deref().unwrap_or(""), &path, create) {
                Ok(()) => {
                    save_settings(&s)?;
                    println!("linked {} (id={})", path.display(), s.project);
                }
                Err(e) => {
                    bail!("{e}");
                }
            }
        }
        HelperCmd::Config => {
            println!("{}", settings_path().display());
        }
        HelperCmd::Logs { lines } => {
            let path = daemon_log_path();
            if !path.exists() {
                println!("(no log yet at {})", path.display());
                return Ok(());
            }
            let text = fs::read_to_string(path)?;
            for line in text
                .lines()
                .rev()
                .take(lines)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
            {
                println!("{line}");
            }
        }
        HelperCmd::Server { url } => {
            let mut s = load_settings()?;
            if let Some(url) = url {
                s.server = normalize_server(&url);
                save_settings(&s)?;
            }
            println!("{}", normalize_server(&s.server));
        }
        HelperCmd::Adapters { action } => match action.unwrap_or(AdaptersAction::List) {
            AdaptersAction::List => list_adapters()?,
            AdaptersAction::Install { names } => {
                let summary = install_adapters(&names)?;
                println!("{summary}");
            }
        },
    }
    Ok(())
}

fn linked_empty(s: &st_core::Settings) -> bool {
    s.projects.is_empty() && s.repo.is_empty()
}

fn fetch_handle(server: &str, token: &str) -> Result<String> {
    let url = format!("{}/v1/auth/me", server.trim_end_matches('/'));
    let client = reqwest::blocking::Client::new();
    let res = client
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .send()?;
    let v: serde_json::Value = res.json()?;
    Ok(v.get("handle")
        .and_then(|h| h.as_str())
        .unwrap_or("")
        .to_string())
}

fn daemon_bin() -> PathBuf {
    if let Ok(p) = std::env::var("STRUGGLETIME_DAEMON") {
        return PathBuf::from(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        // Unified binary: re-exec self with `daemon` subcommand.
        let name = exe.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name == "struggletime" || name == "struggletime.exe" {
            return exe;
        }
        if let Some(dir) = exe.parent() {
            for name in [
                "struggletime",
                "struggletime.exe",
                "struggletime-daemon",
                "struggletime-daemon.exe",
            ] {
                let cand = dir.join(name);
                if cand.exists() {
                    return cand;
                }
            }
        }
        return exe;
    }
    PathBuf::from("struggletime")
}

fn daemon_cmd(bin: &PathBuf) -> Command {
    let mut cmd = Command::new(bin);
    let name = bin.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if name == "struggletime" || name == "struggletime.exe" {
        cmd.arg("daemon");
    }
    // Daemon must never bubble into tray / re-exec loops.
    cmd.env("STRUGGLETIME_NO_RESPAWN", "1");
    cmd
}

fn start_daemon(s: &st_core::Settings) -> Result<()> {
    if daemon_alive() {
        println!("already running (pid {})", read_pid().unwrap_or_default());
        return Ok(());
    }
    let mut watch_paths: Vec<(PathBuf, String)> = Vec::new();
    for p in &s.projects {
        let path = PathBuf::from(&p.path);
        if path.is_dir() {
            watch_paths.push((path, p.id.clone()));
        }
    }
    if watch_paths.is_empty() && !s.repo.is_empty() {
        let path = PathBuf::from(&s.repo);
        if path.is_dir() {
            watch_paths.push((path, s.project.clone()));
        }
    }
    if watch_paths.is_empty() {
        bail!("no project folders to watch — pass --repo or link a project first");
    }

    let bin = daemon_bin();
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(daemon_log_path())?;
    let mut cmd = daemon_cmd(&bin);
    cmd.arg("--server")
        .arg(normalize_server(&s.server))
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .stdin(Stdio::null());
    if !s.token.is_empty() {
        cmd.arg("--token").arg(&s.token);
    }
    if !s.handle.is_empty() {
        cmd.arg("--user").arg(&s.handle);
    }
    for (path, id) in &watch_paths {
        cmd.arg("--repo").arg(path);
        if !id.is_empty() {
            if s.projects.is_empty() {
                cmd.arg("--project").arg(id);
            } else {
                cmd.arg("--project-map")
                    .arg(format!("{}={id}", path.display()));
            }
        }
    }
    let child = cmd.spawn().with_context(|| {
        format!(
            "failed to start {} — build with `cargo build -p st-daemon --release` or set STRUGGLETIME_DAEMON",
            bin.display()
        )
    })?;
    let pid = child.id();
    fs::write(daemon_pid_path(), pid.to_string())?;
    // Brief settle: if the child exits immediately, clear the stale pid so
    // supervisors don't restart-spam a crashing daemon.
    thread::sleep(Duration::from_millis(200));
    if !daemon_alive() {
        let _ = fs::remove_file(daemon_pid_path());
        bail!(
            "daemon exited immediately — check {}",
            daemon_log_path().display()
        );
    }
    println!("started (pid {pid})");
    println!("logging to {}", daemon_log_path().display());
    Ok(())
}

fn stop_daemon() -> Result<()> {
    if let Some(pid) = read_pid() {
        #[cfg(unix)]
        {
            let _ = Command::new("kill").arg(pid.to_string()).status();
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;

            let _ = Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/F"])
                .creation_flags(CREATE_NO_WINDOW)
                .status();
        }
        let _ = fs::remove_file(daemon_pid_path());
        println!("stopped (pid {pid})");
    } else {
        println!("not running");
    }
    Ok(())
}

fn run_daemon_once(s: &st_core::Settings, flush_only: bool) -> Result<()> {
    let bin = daemon_bin();
    let mut cmd = daemon_cmd(&bin);
    cmd.arg("--server").arg(normalize_server(&s.server));
    if flush_only {
        cmd.arg("--flush-only");
    } else {
        cmd.arg("--once");
    }
    if !s.token.is_empty() {
        cmd.arg("--token").arg(&s.token);
    }
    for p in &s.projects {
        let path = PathBuf::from(&p.path);
        if path.is_dir() {
            cmd.arg("--repo").arg(&path);
        }
    }
    if s.projects.is_empty() && !s.repo.is_empty() {
        cmd.arg("--repo").arg(&s.repo);
    }
    let status = cmd.status()?;
    if !status.success() {
        bail!("daemon exited with {status}");
    }
    Ok(())
}

fn read_pid() -> Option<u32> {
    fs::read_to_string(daemon_pid_path())
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

fn daemon_alive() -> bool {
    let Some(pid) = read_pid() else { return false };
    #[cfg(unix)]
    {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;

        Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}")])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
            .unwrap_or(false)
    }
}

fn print_status() -> Result<()> {
    let s = load_settings()?;
    let running = daemon_alive();
    let paused = pause_flag_path().exists();
    let events = runtime_home().join("agent_events.jsonl");
    let (ev_lines, ev_bytes) = match fs::read_to_string(&events) {
        Ok(raw) => (
            raw.lines().filter(|l| !l.trim().is_empty()).count(),
            raw.len(),
        ),
        Err(_) => (0usize, 0usize),
    };
    let mut pending = 0usize;
    for (_id, path) in linked_paths(&s) {
        let ledger = path.join(".aitime-ledger.json");
        if let Ok(raw) = fs::read_to_string(ledger) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                pending += v
                    .get("pending")
                    .and_then(|p| p.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0);
            }
        }
    }
    println!("server:  {}", normalize_server(&s.server));
    println!(
        "handle:  {}",
        if s.handle.is_empty() {
            "(none)"
        } else {
            &s.handle
        }
    );
    println!(
        "api key: {}",
        if s.token.is_empty() { "missing" } else { "set" }
    );
    println!(
        "daemon:  {}",
        if running {
            format!("running (pid {})", read_pid().unwrap_or(0))
        } else {
            "stopped".into()
        }
    );
    println!("paused:  {paused}");
    println!(
        "adapter: {} ({} lines, {} bytes queued)",
        events.display(),
        ev_lines,
        ev_bytes
    );
    println!("pending uploads: {pending}");
    if s.token.is_empty() && pending > 0 {
        println!("hint:    set API key — struggletime apikey <st_…> then struggletime sync");
    }
    if !running {
        println!("hint:    start daemon — struggletime start (or tray)");
    }
    println!("projects:");
    if s.projects.is_empty() {
        if s.repo.is_empty() {
            println!("  (none linked)");
        } else {
            println!("  {} ({})", s.repo, s.project);
        }
    } else {
        for p in &s.projects {
            println!("  {}: {}", p.id, p.path);
        }
    }
    Ok(())
}
