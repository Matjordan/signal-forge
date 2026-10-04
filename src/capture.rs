//! Versioned JSON Lines capture written by a dedicated bounded subscriber.
use crate::{
    bridge::Bridge,
    endpoint::EndpointId,
    inspector::{timestamp_ns, BridgeDirection},
    traffic::{TrafficEvent, TrafficSubscription},
};
use std::{
    fs::OpenOptions,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::RecvTimeoutError,
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime},
};
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureState {
    Recording,
    Finishing,
    Completed,
    Fault(String),
}
#[derive(Debug, Clone)]
pub struct CaptureStatus {
    pub state: CaptureState,
    pub events: u64,
    pub bytes: u64,
    pub dropped: u64,
}
pub struct Capture {
    pub path: PathBuf,
    stop: Arc<AtomicBool>,
    status: Arc<Mutex<CaptureStatus>>,
    worker: Option<JoinHandle<()>>,
}
fn record(writer: &mut impl Write, value: &impl serde::Serialize) -> Result<(), String> {
    serde_json::to_writer(&mut *writer, value).map_err(|e| e.to_string())?;
    writer.write_all(b"\n").map_err(|e| e.to_string())
}
impl Capture {
    pub fn start(path: &Path, bridge: &Bridge) -> Result<Self, String> {
        if bridge.state() != crate::bridge::BridgeState::Running {
            return Err("Start a running bridge before capturing".into());
        }
        let subscription = bridge.subscribe_tracked(4096);
        Self::start_subscription(path, bridge.a.clone(), bridge.b.clone(), subscription)
    }
    /// Also usable by other transport-level observers and deterministic tests.
    pub fn start_subscription(
        path: &Path,
        a: EndpointId,
        b: EndpointId,
        subscription: TrafficSubscription,
    ) -> Result<Self, String> {
        let opened = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| format!("{}: {e}", path.display()));
        let file = match opened {
            Ok(file) => file,
            Err(error) => {
                subscription.close();
                return Err(error);
            }
        };
        let mut writer = BufWriter::new(file);
        if let Err(error)=record(&mut writer,&serde_json::json!({"type":"header","format":"signal-forge-capture","version":1,"started_unix_ns":timestamp_ns(SystemTime::now()).to_string(),"endpoint_a":a.0,"endpoint_b":b.0,"queue_capacity":subscription.capacity,"semantics":"successfully forwarded RX chunks; bounded best-effort capture"})).and_then(|_|writer.flush().map_err(|e|e.to_string())) {
            subscription.close(); return Err(error);
        }
        let stop = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(CaptureStatus {
            state: CaptureState::Recording,
            events: 0,
            bytes: 0,
            dropped: 0,
        }));
        let worker_stop = stop.clone();
        let worker_status = status.clone();
        let worker=thread::Builder::new().name("bridge-capture".into()).spawn(move || {
            let result=(||->Result<(),String> {
                let mut previous:Option<(u64,i128)>=None;
                let mut flush_at=Instant::now();
                loop {
                    if worker_stop.load(Ordering::Acquire) { subscription.close(); break; }
                    match subscription.receiver.recv_timeout(Duration::from_millis(20)) {
                        Ok(event)=>write_event(&mut writer,&event,&a,&mut previous,&worker_status)?,
                        Err(RecvTimeoutError::Timeout)=>{},
                        Err(RecvTimeoutError::Disconnected)=>break,
                    }
                    if flush_at.elapsed()>=Duration::from_millis(100) { writer.flush().map_err(|e|e.to_string())?; flush_at=Instant::now(); }
                    worker_status.lock().unwrap_or_else(|e|e.into_inner()).dropped=subscription.dropped_events();
                }
                subscription.close();
                for event in subscription.receiver.try_iter() { write_event(&mut writer,&event,&a,&mut previous,&worker_status)?; }
                let summary = {
                    let mut status=worker_status.lock().unwrap_or_else(|e|e.into_inner());
                    status.dropped=subscription.dropped_events();
                    status.clone()
                };
                record(&mut writer,&serde_json::json!({"type":"footer","ended_unix_ns":timestamp_ns(SystemTime::now()).to_string(),"events":summary.events,"bytes":summary.bytes,"dropped_events":summary.dropped,"complete":summary.dropped==0}))?;
                writer.flush().map_err(|e|e.to_string())?;
                writer.get_ref().sync_all().map_err(|e|e.to_string())?;
                worker_status.lock().unwrap_or_else(|e|e.into_inner()).state=CaptureState::Completed;
                Ok(())
            })();
            subscription.close();
            if let Err(error)=result { worker_status.lock().unwrap_or_else(|e|e.into_inner()).state=CaptureState::Fault(error); }
        }).map_err(|e|e.to_string())?;
        Ok(Self {
            path: path.to_owned(),
            stop,
            status,
            worker: Some(worker),
        })
    }
    pub fn status(&self) -> CaptureStatus {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Release);
        let mut status = self.status.lock().unwrap_or_else(|e| e.into_inner());
        if status.state == CaptureState::Recording {
            status.state = CaptureState::Finishing;
        }
    }
    pub fn finish(&mut self) {
        self.request_stop();
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                self.status.lock().unwrap_or_else(|e| e.into_inner()).state =
                    CaptureState::Fault("Capture worker panicked".into());
            }
        }
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        self.finish();
    }
}
fn write_event(
    writer: &mut impl Write,
    event: &TrafficEvent,
    a: &EndpointId,
    previous: &mut Option<(u64, i128)>,
    status: &Mutex<CaptureStatus>,
) -> Result<(), String> {
    let timestamp = timestamp_ns(event.timestamp);
    let delta = previous.map(|(_, t)| (timestamp - t).to_string());
    let missed = previous
        .map(|(seq, _)| event.sequence.saturating_sub(seq + 1))
        .unwrap_or(0);
    let direction = if &event.endpoint == a {
        BridgeDirection::AToB
    } else {
        BridgeDirection::BToA
    };
    record(
        writer,
        &serde_json::json!({"type":"event","sequence":event.sequence,"timestamp_unix_ns":timestamp.to_string(),"delta_ns":delta,"direction":direction,"source_endpoint":event.endpoint.0,"raw_bytes":event.bytes.as_ref(),"missed_before":missed}),
    )?;
    *previous = Some((event.sequence, timestamp));
    let mut status = status.lock().unwrap_or_else(|e| e.into_inner());
    status.events += 1;
    status.bytes += event.bytes.len() as u64;
    Ok(())
}
