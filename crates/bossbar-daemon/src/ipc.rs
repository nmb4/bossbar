//! Loopback IPC server: newline-delimited JSON, one request per connection.

use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context as _, Result};
use bossbar_proto::{
    daemon_info_path, state_dir, DaemonInfo, WireRequest, WireResponse, PROTOCOL_VERSION,
};

use crate::{state::Daemon, waker::Waker};

/// Binds the loopback listener, publishes `daemon.json`, and serves requests
/// on background threads.
pub fn serve(shared: Arc<Mutex<Daemon>>, waker: Waker) -> Result<DaemonInfo> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).context("bind bossbar IPC listener")?;
    let port = listener
        .local_addr()
        .context("read bossbar IPC address")?
        .port();
    let info = DaemonInfo {
        pid: std::process::id(),
        port,
        token: random_token(),
        protocol: PROTOCOL_VERSION,
    };
    write_info(&info)?;

    let thread_shared = Arc::clone(&shared);
    let thread_waker = waker.clone();
    let thread_token = info.token.clone();
    std::thread::Builder::new()
        .name("bossbar-ipc".to_owned())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let shared = Arc::clone(&thread_shared);
                let waker = thread_waker.clone();
                let token = thread_token.clone();
                let _ = std::thread::Builder::new()
                    .name("bossbar-ipc-conn".to_owned())
                    .spawn(move || {
                        if let Err(error) = handle(stream, &shared, &waker, &token) {
                            tracing::debug!("ipc connection ended: {error:#}");
                        }
                    });
            }
        })
        .context("spawn bossbar IPC thread")?;

    Ok(info)
}

/// Removes `daemon.json` when it still describes this daemon.
pub fn clear_info(info: &DaemonInfo) {
    let Some(path) = daemon_info_path() else {
        return;
    };
    if let Ok(raw) = fs::read_to_string(&path) {
        if let Ok(current) = serde_json::from_str::<DaemonInfo>(&raw) {
            if current.pid == info.pid && current.port == info.port {
                let _ = fs::remove_file(&path);
            }
        }
    }
}

fn handle(
    mut stream: TcpStream,
    shared: &Arc<Mutex<Daemon>>,
    waker: &Waker,
    token: &str,
) -> Result<()> {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .context("set IPC read timeout")?;
    let mut line = String::new();
    BufReader::new(stream.try_clone().context("clone IPC stream")?).read_line(&mut line)?;

    let response = match serde_json::from_str::<WireRequest>(&line) {
        Ok(request) if request.token == token => {
            let (response, revision) = {
                let mut daemon = shared
                    .lock()
                    .map_err(|_| anyhow::anyhow!("daemon state lock poisoned"))?;
                let response = daemon.apply(&request.cmd);
                (response, daemon.revision)
            };
            tracing::debug!(
                command = request.cmd.name(),
                revision,
                "handled IPC request"
            );
            waker.wake();
            response
        }
        Ok(_) => WireResponse::error("invalid daemon token"),
        Err(error) => WireResponse::error(format!("invalid request: {error}")),
    };

    serde_json::to_writer(&mut stream, &response).context("write IPC response")?;
    stream.write_all(b"\n").context("terminate IPC response")?;
    stream.flush().context("flush IPC response")?;
    Ok(())
}

fn write_info(info: &DaemonInfo) -> Result<()> {
    let dir = state_dir().context("bossbar state directory is unavailable")?;
    fs::create_dir_all(&dir).context("create bossbar state directory")?;
    let path = daemon_info_path().context("bossbar state directory is unavailable")?;
    let temporary = path.with_extension("json.tmp");
    let payload = serde_json::to_vec_pretty(info).context("encode daemon info")?;

    let mut options = OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary).context("open daemon info file")?;
    file.write_all(&payload).context("write daemon info")?;
    file.sync_all().context("sync daemon info")?;
    drop(file);
    fs::rename(&temporary, &path).context("publish daemon info")?;
    Ok(())
}

/// Token guarding the loopback port against unrelated local processes.
///
/// The port and token live in a per-user file, so this only needs to be
/// unpredictable, not cryptographic: any process able to read the file can
/// already run commands through the CLI.
fn random_token() -> String {
    use std::hash::{BuildHasher, Hasher};

    let mut token = String::new();
    let state = std::collections::hash_map::RandomState::new();
    for round in 0u64..2 {
        let mut hasher = state.build_hasher();
        hasher.write_u64(round);
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default(),
        );
        hasher.write_u32(std::process::id());
        hasher.write_u64(round.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        token.push_str(&format!("{:016x}", hasher.finish()));
    }
    token
}
