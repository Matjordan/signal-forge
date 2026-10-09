//! Owned Linux PTYs joined by a bounded, full-duplex internal relay.
use nix::{
    fcntl::{fcntl, FcntlArg, OFlag},
    pty::openpty,
    sys::termios::{cfmakeraw, tcgetattr, tcsetattr, SetArg},
    unistd::ttyname,
};
use std::{
    collections::VecDeque,
    fs::File,
    io::{Read, Write},
    os::fd::AsRawFd,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// Software pacing shared by both ends; directions have independent timelines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LinkTiming {
    #[default]
    Unlimited,
    Emulated(crate::terminal_display::SerialFraming),
}
impl LinkTiming {
    pub fn validate(self) -> Result<(), String> {
        if let Self::Emulated(framing) = self {
            if framing.wire_seconds(1).is_none() {
                return Err("Invalid virtual link baud/framing".into());
            }
        }
        Ok(())
    }
    pub fn label(self) -> String {
        match self {
            Self::Unlimited => "Unlimited".into(),
            Self::Emulated(framing) => format!("Emulated · {}", framing.label()),
        }
    }
}
#[derive(Clone, Debug)]
pub struct PathDiagnostic {
    pub path: String,
    pub target: String,
    pub permissions: String,
    pub access: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairState {
    Running,
    Stopped,
    Fault(String),
}

struct OwnedLink {
    path: PathBuf,
    target: PathBuf,
}
impl Drop for OwnedLink {
    fn drop(&mut self) {
        // Never unlink a path another process replaced after creation.
        if std::fs::read_link(&self.path).ok().as_ref() == Some(&self.target) {
            if let Err(error) = std::fs::remove_file(&self.path) {
                log::warn!("{}: {error}", self.path.display());
            }
        }
    }
}

enum PairCommand {
    ReleaseExclusive(usize, SyncSender<Result<(), String>>),
}
pub struct VirtualPair {
    pub name: String,
    pub paths: [String; 2],
    pub raw_paths: [String; 2],
    pub timing: LinkTiming,
    state: Arc<Mutex<PairState>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    links: Vec<OwnedLink>,
    commands: SyncSender<PairCommand>,
}
impl VirtualPair {
    pub fn create(name: &str, directory: Option<&Path>) -> Result<Self, String> {
        Self::create_with_timing(name, directory, LinkTiming::Unlimited)
    }
    pub fn create_with_timing(
        name: &str,
        directory: Option<&Path>,
        timing: LinkTiming,
    ) -> Result<Self, String> {
        timing.validate()?;
        if name.is_empty()
            || name.len() > 64
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(
                "Pair name must contain 1–64 letters, digits, hyphens, or underscores".into(),
            );
        }
        let first = openpty(None, None).map_err(|e| e.to_string())?;
        let second = openpty(None, None).map_err(|e| e.to_string())?;
        for slave in [&first.slave, &second.slave] {
            let mut settings = tcgetattr(slave).map_err(|e| e.to_string())?;
            cfmakeraw(&mut settings);
            tcsetattr(slave, SetArg::TCSANOW, &settings).map_err(|e| e.to_string())?;
        }
        let raw_paths = [
            ttyname(&first.slave)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .into_owned(),
            ttyname(&second.slave)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .into_owned(),
        ];
        let mut links = Vec::new();
        let mut paths = raw_paths.clone();
        if let Some(directory) = directory {
            std::fs::create_dir_all(directory)
                .map_err(|e| format!("{}: {e}", directory.display()))?;
            for index in 0..2 {
                let path = directory.join(format!("{name}-{}", if index == 0 { "a" } else { "b" }));
                let target = PathBuf::from(&raw_paths[index]);
                std::os::unix::fs::symlink(&target, &path)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                paths[index] = path.to_string_lossy().into_owned();
                links.push(OwnedLink { path, target });
            }
        }
        for master in [&first.master, &second.master] {
            fcntl(master.as_raw_fd(), FcntlArg::F_SETFL(OFlag::O_NONBLOCK))
                .map_err(|e| e.to_string())?;
        }
        let mut a = File::from(first.master);
        let mut b = File::from(second.master);
        let slaves = (first.slave, second.slave);
        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(PairState::Running));
        let (commands, requests) = mpsc::sync_channel(4);
        let worker_stop = stop.clone();
        let worker_state = state.clone();
        let worker = thread::Builder::new()
            .name(format!("pty-pair:{name}"))
            .spawn(move || {
                // Keep both slaves alive while clients open/close them, avoiding EIO.
                let _keepalive = slaves;
                let mut a_to_b = DirectionQueue::new(timing);
                let mut b_to_a = DirectionQueue::new(timing);
                let result = (|| -> std::io::Result<()> {
                    while !worker_stop.load(Ordering::Acquire) {
                        while let Ok(PairCommand::ReleaseExclusive(side, reply)) =
                            requests.try_recv()
                        {
                            let fd = if side == 0 {
                                _keepalive.0.as_raw_fd()
                            } else {
                                _keepalive.1.as_raw_fd()
                            };
                            let result =
                                if unsafe { nix::libc::ioctl(fd, nix::libc::TIOCNXCL) } == 0 {
                                    Ok(())
                                } else {
                                    Err(std::io::Error::last_os_error().to_string())
                                };
                            let _ = reply.send(result);
                        }
                        let moved = relay(&mut a, &mut b, &mut a_to_b)?
                            | relay(&mut b, &mut a, &mut b_to_a)?;
                        if !moved {
                            let now = Instant::now();
                            let delay = [a_to_b.next_delay(now), b_to_a.next_delay(now)]
                                .into_iter()
                                .flatten()
                                .min()
                                .unwrap_or(Duration::from_millis(2))
                                .min(Duration::from_millis(2));
                            // Ready data blocked by the receiver must not spin.
                            thread::sleep(delay.max(Duration::from_micros(100)));
                        }
                    }
                    Ok(())
                })();
                *worker_state.lock().unwrap_or_else(|e| e.into_inner()) = match result {
                    Ok(()) => PairState::Stopped,
                    Err(error) => PairState::Fault(error.to_string()),
                };
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            name: name.into(),
            paths,
            raw_paths,
            timing,
            state,
            stop,
            worker: Some(worker),
            links,
            commands,
        })
    }
    /// Explicit recovery after a client exits; never raises privileges or changes modes.
    /// Clearing a live client's TIOCEXCL allows conflicting opens, so callers must warn.
    pub fn release_exclusive(&self, side: usize) -> Result<(), String> {
        if side > 1 || self.state() != PairState::Running {
            return Err("Choose a side of a running pair".into());
        }
        let (reply, response) = mpsc::sync_channel(1);
        self.commands
            .try_send(PairCommand::ReleaseExclusive(side, reply))
            .map_err(|e| e.to_string())?;
        response
            .recv_timeout(Duration::from_millis(250))
            .map_err(|e| e.to_string())?
    }
    /// Checks access only on explicit request; never changes termios or permissions.
    pub fn diagnostics(&self) -> Vec<PathDiagnostic> {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let diagnose = |(path, target): (&String, &String)| {
            let permissions = match std::fs::metadata(path) {
                Ok(meta) => format!(
                    "mode {:04o} · uid {} · gid {}",
                    meta.mode() & 0o7777,
                    meta.uid(),
                    meta.gid()
                ),
                Err(error) => format!("Missing/inaccessible: {error}"),
            };
            let valid = std::fs::canonicalize(path).ok().as_ref() == Some(&PathBuf::from(target));
            let access = if !valid {
                "Path missing or replaced; it no longer resolves to this pair's PTY".into()
            } else {
                match std::fs::OpenOptions::new().read(true).write(true).custom_flags(nix::libc::O_NOCTTY | nix::libc::O_NONBLOCK).open(path) {
                    Ok(file)=> {
                        // Advisory locks are used by picocom and serialport as well as TIOCEXCL.
                        let locked=unsafe { nix::libc::flock(file.as_raw_fd(),nix::libc::LOCK_EX | nix::libc::LOCK_NB) };
                        if locked==0 { "Can open; external connection state is not reported".into() }
                        else { format!("Locked by another client: {}",std::io::Error::last_os_error()) }
                    },
                    Err(error)=>format!("Cannot open: {error}. If busy, disconnect the client on this side and use its peer."),
                }
            };
            PathDiagnostic {
                path: path.clone(),
                target: target.clone(),
                permissions,
                access,
            }
        };
        self.paths
            .iter()
            .zip(&self.raw_paths)
            .map(diagnose)
            .collect()
    }
    pub fn state(&self) -> PairState {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                log::error!("PTY pair {} worker panicked", self.name);
            }
        }
        self.links.clear();
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = PairState::Stopped;
    }
}
impl Drop for VirtualPair {
    fn drop(&mut self) {
        self.stop();
    }
}

fn relay(
    source: &mut File,
    destination: &mut File,
    pending: &mut DirectionQueue,
) -> std::io::Result<bool> {
    const LIMIT: usize = 65536;
    let mut moved = false;
    if pending.bytes.len() < LIMIT {
        let mut buffer = [0; 4096];
        let capacity = (LIMIT - pending.bytes.len()).min(buffer.len());
        match source.read(&mut buffer[..capacity]) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "PTY closed",
                ))
            }
            Ok(count) => {
                if pending.bytes.is_empty() {
                    pending.restart(Instant::now());
                }
                pending.bytes.extend(&buffer[..count]);
                moved = true;
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) => {}
            Err(e) => return Err(e),
        }
    }
    let now = Instant::now();
    if let Some(blocked) = pending.blocked_since.take() {
        pending.origin += now.saturating_duration_since(blocked);
    }
    let ready = pending.ready(now);
    if ready > 0 {
        let (bytes, _) = pending.bytes.as_slices();
        match destination.write(&bytes[..ready.min(bytes.len())]) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "PTY write returned zero",
                ))
            }
            Ok(count) => {
                pending.bytes.drain(..count);
                pending.delivered += count as u64;
                moved = true;
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                if e.kind() == std::io::ErrorKind::WouldBlock {
                    pending.blocked_since = Some(Instant::now());
                }
            }
            Err(e) => return Err(e),
        }
    }
    Ok(moved)
}

/// Bounded byte queue with an absolute rational schedule, avoiding sleep/rounding drift.
struct DirectionQueue {
    bytes: VecDeque<u8>,
    origin: Instant,
    delivered: u64,
    framing: Option<crate::terminal_display::SerialFraming>,
    blocked_since: Option<Instant>,
}
impl DirectionQueue {
    fn new(timing: LinkTiming) -> Self {
        let framing = match timing {
            LinkTiming::Unlimited => None,
            LinkTiming::Emulated(framing) => Some(framing),
        };
        Self {
            bytes: VecDeque::new(),
            origin: Instant::now(),
            delivered: 0,
            framing,
            blocked_since: None,
        }
    }
    fn restart(&mut self, now: Instant) {
        self.origin = now;
        self.delivered = 0;
        self.blocked_since = None;
    }
    fn ready(&self, now: Instant) -> usize {
        match self.framing {
            None => self.bytes.len(),
            Some(framing) => {
                let completed = now
                    .saturating_duration_since(self.origin)
                    .as_nanos()
                    .saturating_mul(framing.baud as u128)
                    / (framing.frame_bits().unwrap() as u128 * 1_000_000_000);
                completed
                    .saturating_sub(self.delivered as u128)
                    .min(self.bytes.len() as u128) as usize
            }
        }
    }
    fn next_delay(&self, now: Instant) -> Option<Duration> {
        if self.bytes.is_empty() {
            return None;
        }
        self.framing.map(|framing| {
            let numerator = (framing.frame_bits().unwrap() as u128 * 1_000_000_000)
                .saturating_mul(self.delivered as u128 + 1);
            let target = numerator.div_ceil(framing.baud as u128);
            let elapsed = now.saturating_duration_since(self.origin).as_nanos();
            Duration::from_nanos(target.saturating_sub(elapsed).min(u64::MAX as u128) as u64)
        })
    }
}
