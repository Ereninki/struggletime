//! StruggleTime — desktop app binary (tray + CLI + daemon).
//!
//! The HTTP API / dashboard ships separately as `struggletime-server`
//! (`st-server`) so the app stays light.
//!
//! Modes (never ambiguous):
//! - Explicit CLI subcommands → foreground CLI (never tray, never re-exec tray)
//! - `--tray` / `tray` → tray (single-instance; optional one-shot detach from a TTY)
//! - No args → `status` (SSH / CI / headless never auto-start tray)
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use st_core::ledger::Ledger;
use st_core::verify::{verify_chain, FORMULA_VERSION};
use st_daemon::{run as run_daemon, DaemonArgs};
use st_helper::{run as run_helper, AdaptersAction, HelperCli, HelperCmd};
use st_tray::{run as run_tray, TrayArgs};
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Child tray / CLI helpers must set this so they never re-spawn another tray.
const ENV_NO_RESPAWN: &str = "STRUGGLETIME_NO_RESPAWN";
/// Internal tray child after a one-shot detach from an interactive shell.
const ENV_INTERNAL_TRAY: &str = "STRUGGLETIME_INTERNAL_TRAY";

#[derive(Parser, Debug)]
#[command(
    name = "struggletime",
    about = "StruggleTime: track struggle-weighted coding time (tray + CLI + daemon)",
    version
)]
struct Cli {
    /// Start the desktop tray (menu bar / notification area)
    #[arg(long)]
    tray: bool,

    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Watch projects and seal allowlisted metadata buckets
    Daemon(DaemonArgs),
    /// Menu-bar / headless supervisor (keeps the daemon alive)
    Tray(TrayArgs),
    /// Verify a local `.aitime-ledger.json` hash chain
    Verify {
        /// Path to ledger file (default: ./.aitime-ledger.json)
        #[arg(long)]
        ledger: Option<PathBuf>,
    },
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
        /// Fail if project id already exists at another folder
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

fn main() -> Result<()> {
    #[cfg(windows)]
    win_console::attach_if_cli();

    let cli = Cli::parse();
    let cmd = resolve_cmd(cli);

    match cmd {
        Cmd::Daemon(args) => run_daemon(args),
        Cmd::Tray(args) => run_tray_mode(args),
        Cmd::Verify { ledger } => verify_ledger(ledger),
        Cmd::Apikey { key } => run_helper(HelperCli {
            cmd: HelperCmd::Apikey { key },
        }),
        Cmd::ClearKey => run_helper(HelperCli {
            cmd: HelperCmd::ClearKey,
        }),
        Cmd::Start {
            repo,
            project,
            server,
            user,
        } => run_helper(HelperCli {
            cmd: HelperCmd::Start {
                repo,
                project,
                server,
                user,
            },
        }),
        Cmd::Stop => run_helper(HelperCli {
            cmd: HelperCmd::Stop,
        }),
        Cmd::Status => run_helper(HelperCli {
            cmd: HelperCmd::Status,
        }),
        Cmd::Pause => run_helper(HelperCli {
            cmd: HelperCmd::Pause,
        }),
        Cmd::Resume => run_helper(HelperCli {
            cmd: HelperCmd::Resume,
        }),
        Cmd::Sync => run_helper(HelperCli {
            cmd: HelperCmd::Sync,
        }),
        Cmd::Link {
            repo,
            project,
            create,
        } => run_helper(HelperCli {
            cmd: HelperCmd::Link {
                repo,
                project,
                create,
            },
        }),
        Cmd::Config => run_helper(HelperCli {
            cmd: HelperCmd::Config,
        }),
        Cmd::Logs { lines } => run_helper(HelperCli {
            cmd: HelperCmd::Logs { lines },
        }),
        Cmd::Server { url } => run_helper(HelperCli {
            cmd: HelperCmd::Server { url },
        }),
        Cmd::Adapters { action } => run_helper(HelperCli {
            cmd: HelperCmd::Adapters { action },
        }),
    }
}

fn resolve_cmd(cli: Cli) -> Cmd {
    if cli.tray {
        return Cmd::Tray(TrayArgs::default());
    }
    if let Some(cmd) = cli.cmd {
        return cmd;
    }
    // macOS .app sets LSEnvironment STRUGGLETIME_APP_BUNDLE=1 (no argv from Finder).
    if std::env::var_os("STRUGGLETIME_APP_BUNDLE").is_some()
        && std::env::var_os(ENV_NO_RESPAWN).is_none()
    {
        return Cmd::Tray(TrayArgs::default());
    }
    // No subcommand: status. Desktop .desktop / Start Menu / LaunchAgent must
    // pass --tray explicitly so SSH / CI / headless never enter the tray loop.
    Cmd::Status
}

fn run_tray_mode(args: TrayArgs) -> Result<()> {
    if std::env::var_os(ENV_NO_RESPAWN).is_some() && std::env::var_os(ENV_INTERNAL_TRAY).is_none() {
        bail!(
            "refusing to start tray under {ENV_NO_RESPAWN}=1 \
             (CLI helper/daemon path must not launch the tray)"
        );
    }
    // One-shot detach from an interactive TTY so the shell returns immediately.
    // The child is marked INTERNAL_TRAY and never detaches again.
    if should_detach_tray(&args) {
        return spawn_detached_tray(&args);
    }
    run_tray(args)
}

fn should_detach_tray(args: &TrayArgs) -> bool {
    if args.headless {
        return false;
    }
    if std::env::var_os(ENV_INTERNAL_TRAY).is_some() {
        return false;
    }
    if std::env::var_os(ENV_NO_RESPAWN).is_some() {
        return false;
    }
    // Already a GUI / LaunchAgent style launch (no TTY).
    if !std::io::stdin().is_terminal() && !std::io::stdout().is_terminal() {
        return false;
    }
    std::io::stdin().is_terminal() || std::io::stdout().is_terminal()
}

fn spawn_detached_tray(args: &TrayArgs) -> Result<()> {
    let exe = std::env::current_exe().context("current_exe")?;
    let mut cmd = Command::new(&exe);
    cmd.arg("tray");
    if args.headless {
        cmd.arg("--headless");
    }
    if args.interval != 15 {
        cmd.arg("--interval").arg(args.interval.to_string());
    }
    cmd.env(ENV_INTERNAL_TRAY, "1");
    cmd.env(ENV_NO_RESPAWN, "1");
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
    }
    let child = cmd
        .spawn()
        .with_context(|| format!("spawn {}", exe.display()))?;
    println!(
        "StruggleTime tray started (pid {}) — menu bar / system tray",
        child.id()
    );
    Ok(())
}

fn verify_ledger(path: Option<PathBuf>) -> Result<()> {
    let path = path.unwrap_or_else(|| PathBuf::from(".aitime-ledger.json"));
    let led = Ledger::load(&path).with_context(|| format!("load {}", path.display()))?;
    let errs = verify_chain(&led.entries);
    println!("ledger:  {}", path.display());
    println!("entries: {}", led.entries.len());
    println!("pending: {}", led.pending.len());
    println!("formula: {FORMULA_VERSION}");
    if errs.is_empty() {
        println!("chain:   ok");
        Ok(())
    } else {
        for e in &errs {
            eprintln!("chain:   {e}");
        }
        bail!("hash chain verification failed ({} error(s))", errs.len());
    }
}

#[cfg(windows)]
mod win_console {
    //! Attach to the parent console when invoked as a CLI from a terminal.
    //! Release builds use windows_subsystem = "windows" so double-click has no console.

    #[link(name = "kernel32")]
    extern "system" {
        fn AttachConsole(dw_process_id: u32) -> i32;
        fn AllocConsole() -> i32;
    }

    const ATTACH_PARENT_PROCESS: u32 = 0xFFFF_FFFF;

    fn wants_gui_tray(args: &[String]) -> bool {
        if args.is_empty() {
            // No-args is status now; attach console for CLI output.
            return false;
        }
        if args
            .iter()
            .any(|a| a == "--help" || a == "-h" || a == "--version" || a == "-V")
        {
            return false;
        }
        if args.iter().any(|a| a == "--tray") {
            return !args.iter().any(|a| a == "--headless");
        }
        if args.first().map(String::as_str) == Some("tray") {
            return !args.iter().any(|a| a == "--headless");
        }
        false
    }

    pub fn attach_if_cli() {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if wants_gui_tray(&args) {
            return;
        }
        unsafe {
            if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
                let _ = AllocConsole();
            }
        }
    }
}
