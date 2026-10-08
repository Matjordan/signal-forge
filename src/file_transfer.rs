//! File snapshots are prepared off the UI thread; validated bytes use normal TX.
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FileMode {
    #[default]
    Raw,
    Ascii,
    Hex,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransferState {
    Preparing,
    Sending,
    Completed,
    Cancelled,
    Failed(String),
}
#[derive(Clone, Debug)]
pub struct TransferStatus {
    pub state: TransferState,
    pub sent: u64,
    pub total: u64,
}
#[derive(Clone)]
pub struct FileTransferHandle {
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) sent: Arc<AtomicU64>,
    status: Arc<Mutex<(TransferState, u64)>>,
}
impl FileTransferHandle {
    pub(crate) fn new() -> Self {
        Self {
            cancel: Arc::new(AtomicBool::new(false)),
            sent: Arc::new(AtomicU64::new(0)),
            status: Arc::new(Mutex::new((TransferState::Preparing, 0))),
        }
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
    pub fn status(&self) -> TransferStatus {
        let status = self.status.lock().unwrap();
        TransferStatus {
            state: status.0.clone(),
            total: status.1,
            sent: self.sent.load(Ordering::Acquire),
        }
    }
    pub fn is_active(&self) -> bool {
        matches!(
            self.status().state,
            TransferState::Preparing | TransferState::Sending
        )
    }
    pub(crate) fn set(&self, state: TransferState, total: u64) {
        *self.status.lock().unwrap() = (state, total);
    }
    pub(crate) fn finish(&self, state: TransferState) {
        self.status.lock().unwrap().0 = state;
    }
}
pub(crate) struct FileJob {
    pub file: File,
    pub handle: FileTransferHandle,
    pub remaining: u64,
    pub next: Instant,
    pub delay: Duration,
    pub chunk: usize,
}
impl Drop for FileJob {
    fn drop(&mut self) {
        if self.handle.is_active() {
            self.handle.finish(TransferState::Cancelled);
        }
    }
}

pub(crate) fn prepare(
    path: PathBuf,
    mode: FileMode,
    handle: &FileTransferHandle,
    stop: &AtomicBool,
    chunk: usize,
    delay: Duration,
) -> Result<FileJob, String> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut input = OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NONBLOCK)
        .open(&path)
        .map_err(|e| e.to_string())?;
    let metadata = input.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err("Send file requires a regular file".into());
    }
    let mut output = tempfile::tempfile().map_err(|e| e.to_string())?;
    let mut buffer = [0; 65536];
    let mut total = 0u64;
    let mut high = None;
    let mut utf8_tail = Vec::new();
    let mut offset = 0u64;
    loop {
        if handle.cancel.load(Ordering::Acquire) || stop.load(Ordering::Acquire) {
            return Err("File send cancelled".into());
        }
        let count = input.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        offset += count as u64;
        match mode {
            FileMode::Raw => {
                output
                    .write_all(&buffer[..count])
                    .map_err(|e| e.to_string())?;
                total += count as u64;
            }
            FileMode::Ascii => {
                utf8_tail.extend_from_slice(&buffer[..count]);
                match std::str::from_utf8(&utf8_tail) {
                    Ok(_) => utf8_tail.clear(),
                    Err(e) if e.error_len().is_none() => {utf8_tail.drain(..e.valid_up_to());},
                    Err(_) => return Err(format!("ASCII/Text requires UTF-8; invalid text near input byte {offset}. Use Raw for binary files.")),
                }
                output
                    .write_all(&buffer[..count])
                    .map_err(|e| e.to_string())?;
                total += count as u64;
            }
            FileMode::Hex => {
                let mut decoded = Vec::with_capacity(count / 2 + 1);
                for byte in &buffer[..count] {
                    if byte.is_ascii_whitespace() {
                        continue;
                    }
                    let digit = match byte {b'0'..=b'9' => byte-b'0', b'a'..=b'f' => byte-b'a'+10, b'A'..=b'F' => byte-b'A'+10,
                        _ => return Err(format!("Invalid Hex file near input byte {offset}: expected ASCII hex digits and whitespace"))};
                    if let Some(first) = high.take() {
                        decoded.push(first * 16 + digit);
                    } else {
                        high = Some(digit);
                    }
                }
                output.write_all(&decoded).map_err(|e| e.to_string())?;
                total += decoded.len() as u64;
            }
        }
    }
    if high.is_some() {
        return Err("Hex file has an unmatched final digit".into());
    }
    if !utf8_tail.is_empty() {
        return Err("ASCII/Text file ends with incomplete UTF-8".into());
    }
    output.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    handle.set(TransferState::Sending, total);
    Ok(FileJob {
        file: output,
        handle: handle.clone(),
        remaining: total,
        next: Instant::now(),
        delay,
        chunk,
    })
}

pub fn validate_options(chunk: usize, delay: Duration) -> Result<(), String> {
    if !(1..=65536).contains(&chunk) || delay > Duration::from_secs(60) {
        return Err(
            "Chunk size must be 1–65536 bytes; inter-chunk delay must be at most 60 seconds".into(),
        );
    }
    Ok(())
}
