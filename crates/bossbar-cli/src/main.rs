//! `bossbar`: CLI client for the bossbar daemon.
//!
//! Talks to the daemon over loopback TCP (see `bossbar-proto`), starting it
//! on demand unless `--no-spawn` is given.

use std::{
    io::{BufRead, BufReader, Write},
    net::{Shutdown, TcpStream},
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

use anyhow::{bail, Context as _, Result};
use bossbar_proto::{
    daemon_info_path, Anchor, BarInit, BarKind, BarPatch, BarSnapshot, BarState, BarStatus,
    DaemonInfo, Request, WireRequest, WireResponse, MAX_TICKS,
};
use clap::{Parser, Subcommand, ValueEnum};

const CONNECT_TIMEOUT: Duration = Duration::from_millis(400);
const IO_TIMEOUT: Duration = Duration::from_secs(10);
const DAEMON_START_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Parser, Debug)]
#[command(
    name = "bossbar",
    version,
    about = "System-wide progress bars in a floating pill",
    long_about = "Creates and updates bossbar progress bars. The daemon is started on demand;\n\
                  run `bossbar daemon stop` to shut it down."
)]
struct Cli {
    /// Print machine-readable JSON instead of text.
    #[arg(long, global = true)]
    json: bool,
    /// Never start the daemon; fail if it is not already running.
    #[arg(long, global = true)]
    no_spawn: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Create a bar and print its id.
    Create {
        /// Label shown in the pill.
        label: String,
        /// Stable id (defaults to b1, b2, ...). Required to update the bar later.
        #[arg(long)]
        id: Option<String>,
        /// Percentage 0-100 (default mode).
        #[arg(long, conflicts_with_all = ["ticks", "indeterminate"])]
        percent: Option<f64>,
        /// Minecraft-style segmented bar with N ticks.
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=MAX_TICKS as i64))]
        ticks: Option<u32>,
        /// Starting tick when using --ticks.
        #[arg(long, requires = "ticks")]
        tick: Option<u32>,
        /// Unknown-duration spinner.
        #[arg(long)]
        indeterminate: bool,
        /// Optional second line under the label.
        #[arg(long)]
        detail: Option<String>,
        /// Accent color: blue, cyan, green, amber, red, violet, pink, or #rrggbb.
        #[arg(long)]
        color: Option<String>,
    },
    /// Change a running bar.
    Update {
        /// Bar id.
        id: String,
        #[arg(long)]
        label: Option<String>,
        /// Percentage 0-100.
        #[arg(long)]
        percent: Option<f64>,
        /// Current tick (ticks bars only).
        #[arg(long)]
        tick: Option<u32>,
        #[arg(long)]
        detail: Option<String>,
        #[arg(long)]
        color: Option<String>,
        /// Lifecycle status; `done`/`failed` fill and then retire the bar.
        #[arg(long)]
        status: Option<StatusArg>,
    },
    /// Advance a ticks bar by one (or --by N / --to N).
    Tick {
        /// Bar id.
        id: String,
        #[arg(long, allow_hyphen_values = true, conflicts_with = "to")]
        by: Option<i64>,
        #[arg(long)]
        to: Option<u32>,
    },
    /// Complete a bar: fills it, flashes success, then removes it.
    Finish {
        /// Bar id.
        id: String,
        #[arg(long)]
        label: Option<String>,
        /// How long the success state stays visible.
        #[arg(long, default_value = "1600ms")]
        hold: HumanDuration,
    },
    /// Mark a bar failed: flashes the failure state, then removes it.
    Fail {
        /// Bar id.
        id: String,
        /// Error line shown under the label.
        #[arg(long)]
        message: Option<String>,
        /// How long the failure state stays visible.
        #[arg(long, default_value = "4200ms")]
        hold: HumanDuration,
    },
    /// Remove a bar immediately.
    Remove {
        /// Bar id.
        id: String,
    },
    /// List live bars.
    List,
    /// Block until a bar disappears (exit 1 if it failed, 2 on timeout).
    Wait {
        /// Bar id.
        id: String,
        #[arg(long, default_value = "30m")]
        timeout: HumanDuration,
        #[arg(long, default_value = "250ms")]
        interval: HumanDuration,
    },
    /// Remove every bar.
    Clear,
    /// Collapse the pill into one line with a progress spinner.
    Collapse {
        #[arg(value_enum)]
        value: Option<OnOffToggle>,
    },
    /// Anchor the pill on the active screen.
    Position {
        #[arg(value_enum)]
        anchor: AnchorArg,
    },
    /// Add breathing room around the pill (off = flush with the screen bounds).
    Padding {
        #[arg(value_enum)]
        value: Option<OnOffToggle>,
    },
    /// Show or hide the pill without touching the bars.
    Visible {
        #[arg(value_enum)]
        value: Option<OnOffToggle>,
    },
    /// Run a command and drive a bar from its output.
    Run {
        /// Bar label (defaults to the command name).
        #[arg(long)]
        label: Option<String>,
        /// Tick once per output line instead of spinning indefinitely.
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=MAX_TICKS as i64))]
        ticks: Option<u32>,
        #[arg(long)]
        color: Option<String>,
        /// Command and arguments, after `--`.
        #[arg(last = true, required = true, num_args = 1..)]
        command: Vec<String>,
    },
    /// Manage the background daemon.
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
    },
}

#[derive(Subcommand, Debug, Clone)]
enum DaemonCommand {
    /// Start the daemon if it is not already running.
    Start,
    /// Stop the daemon.
    Stop,
    /// Restart the daemon.
    Restart,
    /// Report whether the daemon is running.
    Status,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum OnOffToggle {
    On,
    Off,
    Toggle,
}

impl OnOffToggle {
    fn as_option(self) -> Option<bool> {
        match self {
            Self::On => Some(true),
            Self::Off => Some(false),
            Self::Toggle => None,
        }
    }
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum AnchorArg {
    TopCenter,
    TopRight,
}

impl From<AnchorArg> for Anchor {
    fn from(value: AnchorArg) -> Self {
        match value {
            AnchorArg::TopCenter => Anchor::TopCenter,
            AnchorArg::TopRight => Anchor::TopRight,
        }
    }
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum StatusArg {
    Running,
    Done,
    Failed,
}

impl From<StatusArg> for BarStatus {
    fn from(value: StatusArg) -> Self {
        match value {
            StatusArg::Running => BarStatus::Running,
            StatusArg::Done => BarStatus::Done,
            StatusArg::Failed => BarStatus::Failed,
        }
    }
}

/// Duration with an explicit unit: `800ms`, `2s`, `1.5s`, `3m`, `1h`.
/// A bare number is milliseconds.
#[derive(Clone, Copy, Debug)]
struct HumanDuration(Duration);

impl std::str::FromStr for HumanDuration {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = value.trim();
        let (number, multiplier) = if let Some(rest) = value.strip_suffix("ms") {
            (rest, 1.0)
        } else if let Some(rest) = value.strip_suffix('s') {
            (rest, 1000.0)
        } else if let Some(rest) = value.strip_suffix('m') {
            (rest, 60_000.0)
        } else if let Some(rest) = value.strip_suffix('h') {
            (rest, 3_600_000.0)
        } else {
            (value, 1.0)
        };
        let number: f64 = number
            .trim()
            .parse()
            .map_err(|_| format!("invalid duration '{value}' (use 500ms, 2s, 3m, 1h)"))?;
        if !number.is_finite() || number < 0.0 {
            return Err(format!("invalid duration '{value}'"));
        }
        Ok(Self(Duration::from_secs_f64(number * multiplier / 1000.0)))
    }
}

fn main() {
    let cli = Cli::parse();
    let code = match run(cli) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("bossbar: {error:#}");
            1
        }
    };
    std::process::exit(code);
}

fn run(cli: Cli) -> Result<i32> {
    if let Command::Daemon { command } = &cli.command {
        return daemon_command(command.clone(), &cli);
    }

    let info = ensure_daemon(cli.no_spawn)?;
    let request = match cli.command {
        Command::Create {
            label,
            id,
            percent,
            ticks,
            tick,
            indeterminate,
            detail,
            color,
        } => {
            let kind = if let Some(total) = ticks {
                BarKind::Ticks { total }
            } else if indeterminate {
                BarKind::Indeterminate
            } else {
                BarKind::Percent
            };
            Request::Create {
                id,
                bar: BarInit {
                    label,
                    kind,
                    percent: percent.unwrap_or(0.0),
                    tick: tick.unwrap_or(0),
                    detail,
                    color,
                },
            }
        }
        Command::Update {
            id,
            label,
            percent,
            tick,
            detail,
            color,
            status,
        } => Request::Update {
            id,
            patch: BarPatch {
                label,
                kind: None,
                percent,
                tick,
                detail,
                color,
                status: status.map(Into::into),
            },
        },
        Command::Tick { id, by, to } => Request::Tick { id, by, to },
        Command::Finish { id, label, hold } => {
            if let Some(label) = label {
                let _ = call(
                    &info,
                    &Request::Update {
                        id: id.clone(),
                        patch: BarPatch {
                            label: Some(label),
                            ..Default::default()
                        },
                    },
                );
            }
            Request::Finish {
                id,
                hold_ms: Some(hold.0.as_millis() as u64),
            }
        }
        Command::Fail { id, message, hold } => Request::Fail {
            id,
            message,
            hold_ms: Some(hold.0.as_millis() as u64),
        },
        Command::Remove { id } => Request::Remove { id },
        Command::Clear => Request::Clear,
        Command::Collapse { value } => Request::Collapse {
            value: value.and_then(OnOffToggle::as_option),
        },
        Command::Position { anchor } => Request::SetPosition {
            anchor: anchor.into(),
        },
        Command::Padding { value } => Request::SetPadding {
            value: value.and_then(OnOffToggle::as_option),
        },
        Command::Visible { value } => Request::SetVisible {
            value: value.and_then(OnOffToggle::as_option),
        },
        Command::List => {
            let state = state_from(call(&info, &Request::List)?)?;
            print_state(&state, cli.json)?;
            return Ok(0);
        }
        Command::Wait {
            id,
            timeout,
            interval,
        } => return wait_for(&info, &id, timeout.0, interval.0),
        Command::Run {
            label,
            ticks,
            color,
            command,
        } => return run_command(&info, label, ticks, color, command),
        Command::Daemon { .. } => unreachable!("handled above"),
    };

    let response = request_raw(&info, &request)?;
    if cli.json {
        let payload = response.data.clone().unwrap_or(serde_json::Value::Null);
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        describe(&request, &response);
    }
    Ok(0)
}

fn request_raw(info: &DaemonInfo, request: &Request) -> Result<WireResponse> {
    let response = call(info, request)?;
    if !response.ok {
        bail!(
            "{}",
            response
                .error
                .clone()
                .unwrap_or_else(|| "daemon rejected the request".to_owned())
        );
    }
    Ok(response)
}

fn describe(request: &Request, response: &WireResponse) {
    match request {
        Request::Create { .. } => {
            let id = response
                .data
                .as_ref()
                .and_then(|data| data.get("id"))
                .and_then(|id| id.as_str())
                .unwrap_or("?");
            println!("created {id}");
        }
        Request::Finish { id, .. } => println!("finishing {id}"),
        Request::Fail { id, .. } => println!("failed {id}"),
        Request::Remove { id } => println!("removed {id}"),
        Request::Clear => println!("cleared all bars"),
        Request::Collapse { value } => match value {
            Some(true) => println!("collapsed"),
            Some(false) => println!("expanded"),
            None => println!("toggled collapse"),
        },
        Request::SetPosition { anchor } => println!("position: {anchor}"),
        Request::SetPadding { value } => match value {
            Some(true) => println!("padding on"),
            Some(false) => println!("padding off (flush)"),
            None => println!("toggled padding"),
        },
        Request::SetVisible { value } => match value {
            Some(true) => println!("bars visible"),
            Some(false) => println!("bars hidden"),
            None => println!("toggled visibility"),
        },
        Request::Ping => println!("pong"),
        Request::Update { id, .. } => println!("updated {id}"),
        Request::Tick { id, .. } => println!("ticked {id}"),
        Request::List | Request::Shutdown => {}
    }
}

fn print_state(state: &BarState, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(state)?);
        return Ok(());
    }
    if state.bars.is_empty() {
        println!("no bars");
    } else {
        let mode = if state.collapsed && state.bars.len() >= 2 {
            "collapsed"
        } else {
            "expanded"
        };
        println!(
            "{} bar{} · {} · {} · {}{}",
            state.bars.len(),
            if state.bars.len() == 1 { "" } else { "s" },
            state.anchor,
            mode,
            if state.visible { "visible" } else { "hidden" },
            if state.padding { " · padded" } else { "" }
        );
        for bar in &state.bars {
            print_bar_row(bar);
        }
    }
    Ok(())
}

fn print_bar_row(bar: &BarSnapshot) {
    let value = match bar.kind {
        BarKind::Indeterminate => "  -- ".to_owned(),
        _ => format!("{:>5}", bar.value_text()),
    };
    let status = match bar.status {
        BarStatus::Running => "",
        BarStatus::Done => "  done",
        BarStatus::Failed => "  failed",
    };
    println!("  {} {value}  {}{status}", bar.id, bar.label);
    if let Some(detail) = bar.detail.as_deref().filter(|detail| !detail.is_empty()) {
        println!("  {}        └ {detail}", " ".repeat(bar.id.len()));
    }
}

fn wait_for(info: &DaemonInfo, id: &str, timeout: Duration, interval: Duration) -> Result<i32> {
    let deadline = Instant::now() + timeout;
    let mut last_status = BarStatus::Running;
    let mut seen = false;
    loop {
        let state = state_from(call(info, &Request::List)?)?;
        match state.bars.iter().find(|bar| bar.id == id) {
            Some(bar) => {
                seen = true;
                last_status = bar.status;
            }
            None if seen => {
                return match last_status {
                    BarStatus::Failed => {
                        eprintln!("bossbar: bar '{id}' failed");
                        Ok(1)
                    }
                    _ => {
                        println!("bar '{id}' finished");
                        Ok(0)
                    }
                };
            }
            None if Instant::now() >= deadline => {
                bail!("no bar with id '{id}' appeared within the timeout");
            }
            None => {}
        }
        if Instant::now() >= deadline {
            bail!("timed out waiting for bar '{id}'");
        }
        std::thread::sleep(interval.max(Duration::from_millis(20)));
    }
}

fn run_command(
    info: &DaemonInfo,
    label: Option<String>,
    ticks: Option<u32>,
    color: Option<String>,
    command: Vec<String>,
) -> Result<i32> {
    // Let Ctrl+C reach the child only; the CLI keeps streaming long enough to
    // mark the bar failed instead of leaving it spinning forever.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGINT, libc::SIG_IGN);
    }

    let label = label.unwrap_or_else(|| {
        Path::new(&command[0])
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| command[0].clone())
    });
    let kind = match ticks {
        Some(total) => BarKind::Ticks { total },
        None => BarKind::Indeterminate,
    };
    let response = request_raw(
        info,
        &Request::Create {
            id: None,
            bar: BarInit {
                label,
                kind,
                percent: 0.0,
                tick: 0,
                detail: None,
                color,
            },
        },
    )?;
    let bar_id = response
        .data
        .as_ref()
        .and_then(|data| data.get("id"))
        .and_then(|id| id.as_str())
        .context("daemon did not return a bar id")?
        .to_owned();

    let mut child = ProcessCommand::new(&command[0])
        .args(&command[1..])
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawn '{}'", command.join(" ")))?;

    enum Line {
        Out(String),
        Err(String),
    }
    let (tx, rx) = mpsc::channel::<Line>();
    let mut readers = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                println!("{line}");
                if tx.send(Line::Out(line)).is_err() {
                    break;
                }
            }
        }));
    }
    if let Some(stderr) = child.stderr.take() {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                eprintln!("{line}");
                if tx.send(Line::Err(line)).is_err() {
                    break;
                }
            }
        }));
    }
    drop(tx);

    // Stream output into the bar, throttled so a chatty command cannot flood
    // the daemon (ticks accumulate and flush together).
    let mut last_detail = String::new();
    let mut last_flush = Instant::now();
    let mut pending_ticks: u32 = 0;
    let flush = |info: &DaemonInfo,
                 detail: &str,
                 pending: &mut u32,
                 last_flush: &mut Instant,
                 force: bool| {
        let due = force || last_flush.elapsed() >= Duration::from_millis(120);
        if !due {
            return;
        }
        *last_flush = Instant::now();
        if ticks.is_some() && *pending > 0 {
            let _ = call(
                info,
                &Request::Tick {
                    id: bar_id.clone(),
                    by: Some(i64::from(*pending)),
                    to: None,
                },
            );
            *pending = 0;
        }
        if !detail.is_empty() {
            let _ = call(
                info,
                &Request::Update {
                    id: bar_id.clone(),
                    patch: BarPatch {
                        detail: Some(detail.to_owned()),
                        ..Default::default()
                    },
                },
            );
        }
    };

    for line in rx {
        let text = match &line {
            Line::Out(text) | Line::Err(text) => text.trim(),
        };
        if text.is_empty() {
            continue;
        }
        if ticks.is_some() {
            pending_ticks = pending_ticks.saturating_add(1);
        }
        if text != last_detail {
            last_detail = text.chars().take(120).collect();
        }
        flush(
            info,
            &last_detail,
            &mut pending_ticks,
            &mut last_flush,
            false,
        );
    }
    for reader in readers {
        let _ = reader.join();
    }
    flush(
        info,
        &last_detail,
        &mut pending_ticks,
        &mut last_flush,
        true,
    );

    let status = child.wait().context("wait for child process")?;
    if status.success() {
        let _ = call(
            info,
            &Request::Finish {
                id: bar_id.clone(),
                hold_ms: None,
            },
        );
        Ok(0)
    } else {
        let message = match status.code() {
            Some(code) => format!("exit code {code}"),
            None => "terminated by signal".to_owned(),
        };
        let _ = call(
            info,
            &Request::Fail {
                id: bar_id.clone(),
                message: Some(message),
                hold_ms: None,
            },
        );
        Ok(status.code().unwrap_or(1))
    }
}

fn daemon_command(command: DaemonCommand, cli: &Cli) -> Result<i32> {
    match command {
        DaemonCommand::Start => {
            let info = spawn_and_wait()?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&info)?);
            } else {
                println!("daemon running (pid {}, port {})", info.pid, info.port);
            }
            Ok(0)
        }
        DaemonCommand::Stop => {
            let Some(info) = running_daemon() else {
                println!("daemon is not running");
                return Ok(0);
            };
            match call(&info, &Request::Shutdown) {
                Ok(_) => println!("daemon stopping (pid {})", info.pid),
                Err(error) => bail!("could not stop daemon: {error:#}"),
            }
            Ok(0)
        }
        DaemonCommand::Restart => {
            if let Some(info) = running_daemon() {
                let _ = call(&info, &Request::Shutdown);
                let deadline = Instant::now() + Duration::from_secs(5);
                while Instant::now() < deadline && running_daemon().is_some() {
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
            let info = spawn_and_wait()?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&info)?);
            } else {
                println!("daemon restarted (pid {}, port {})", info.pid, info.port);
            }
            Ok(0)
        }
        DaemonCommand::Status => match running_daemon() {
            Some(info) => {
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&info)?);
                } else {
                    println!("daemon running (pid {}, port {})", info.pid, info.port);
                }
                Ok(0)
            }
            None => {
                if cli.json {
                    println!("null");
                } else {
                    println!("daemon is not running");
                }
                Ok(1)
            }
        },
    }
}

fn state_from(response: WireResponse) -> Result<BarState> {
    response.into_data().map_err(anyhow::Error::msg)
}

/// Sends one request and reads one response.
fn call(info: &DaemonInfo, request: &Request) -> Result<WireResponse> {
    let mut stream = TcpStream::connect_timeout(
        &format!("127.0.0.1:{}", info.port)
            .parse()
            .context("build daemon address")?,
        CONNECT_TIMEOUT,
    )
    .context("connect to bossbar daemon")?;
    stream.set_nodelay(true).ok();
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .context("set read timeout")?;
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .context("set write timeout")?;

    let envelope = WireRequest {
        token: info.token.clone(),
        cmd: request.clone(),
    };
    serde_json::to_writer(&mut stream, &envelope).context("encode request")?;
    stream.write_all(b"\n").context("terminate request")?;
    stream.flush().context("flush request")?;
    stream
        .shutdown(Shutdown::Write)
        .context("half-close request stream")?;

    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .context("read daemon response")?;
    if line.trim().is_empty() {
        bail!("daemon closed the connection without a response");
    }
    serde_json::from_str(&line).context("decode daemon response")
}

fn daemon_info() -> Option<DaemonInfo> {
    let raw = std::fs::read_to_string(daemon_info_path()?).ok()?;
    serde_json::from_str(&raw).ok()
}

fn running_daemon() -> Option<DaemonInfo> {
    let info = daemon_info()?;
    call(&info, &Request::Ping).ok()?;
    Some(info)
}

fn ensure_daemon(no_spawn: bool) -> Result<DaemonInfo> {
    if let Some(info) = running_daemon() {
        return Ok(info);
    }
    if no_spawn {
        bail!("bossbar daemon is not running (start it with `bossbar daemon start`)");
    }
    spawn_and_wait()
}

fn spawn_and_wait() -> Result<DaemonInfo> {
    let binary = daemon_binary();
    let mut command = ProcessCommand::new(&binary);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        command.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        unsafe {
            command.pre_exec(|| {
                // Detach from the controlling terminal so the daemon survives
                // the shell that started it.
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let child = command
        .spawn()
        .with_context(|| format!("start daemon at {}", binary.display()))?;
    drop(child);

    let deadline = Instant::now() + DAEMON_START_TIMEOUT;
    while Instant::now() < deadline {
        if let Some(info) = running_daemon() {
            return Ok(info);
        }
        std::thread::sleep(Duration::from_millis(120));
    }
    bail!(
        "daemon did not become ready within {}s; see the log in the bossbar state directory",
        DAEMON_START_TIMEOUT.as_secs()
    )
}

fn daemon_binary() -> PathBuf {
    if let Some(explicit) = std::env::var_os("BOSSBAR_DAEMON") {
        return PathBuf::from(explicit);
    }
    if let Ok(current) = std::env::current_exe() {
        let sibling = current.with_file_name(if cfg!(windows) {
            "bossbar-daemon.exe"
        } else {
            "bossbar-daemon"
        });
        if sibling.is_file() {
            return sibling;
        }
    }
    PathBuf::from("bossbar-daemon")
}
