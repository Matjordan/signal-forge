//! RX payload files with no display metadata; writers run independently of GUI.
use crate::{
    endpoint::EndpointId,
    file_transfer::FileMode,
    traffic::{Direction, TrafficBus, TrafficControl},
};
use std::{
    fs::OpenOptions,
    io::{BufWriter, Write},
    path::Path,
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordingState {
    Recording,
    Finishing,
    Completed,
    Incomplete,
    Failed(String),
}
#[derive(Clone, Debug)]
pub struct RecordingStatus {
    pub state: RecordingState,
    pub received: u64,
    pub written: u64,
    pub dropped: u64,
}
pub struct RawRecording {
    control: TrafficControl,
    status: Arc<Mutex<RecordingStatus>>,
    worker: Option<JoinHandle<()>>,
}
impl RawRecording {
    pub fn start(path: &Path, endpoint: EndpointId, bus: &TrafficBus) -> Result<Self, String> {
        Self::start_mode(path, endpoint, bus, FileMode::Raw)
    }
    pub fn start_mode(
        path: &Path,
        endpoint: EndpointId,
        bus: &TrafficBus,
        mode: FileMode,
    ) -> Result<Self, String> {
        Self::start_subscription(
            path,
            endpoint.clone(),
            mode,
            bus.subscribe_endpoint_rx(4096, endpoint),
        )
    }
    pub fn start_subscription(
        path: &Path,
        endpoint: EndpointId,
        mode: FileMode,
        subscription: crate::traffic::TrafficSubscription,
    ) -> Result<Self, String> {
        let opened = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| e.to_string());
        let file = match opened {
            Ok(file) => file,
            Err(error) => {
                subscription.close();
                return Err(error);
            }
        };
        let control = subscription.control();
        let status = Arc::new(Mutex::new(RecordingStatus {
            state: RecordingState::Recording,
            received: 0,
            written: 0,
            dropped: 0,
        }));
        let progress = status.clone();
        let worker = thread::spawn(move || {
            let mut writer = BufWriter::new(file);
            let mut column = 0;
            let result = (|| -> Result<(), std::io::Error> {
                while let Ok(event) = subscription.receiver.recv() {
                    if event.endpoint != endpoint || event.direction != Direction::Rx {
                        continue;
                    }
                    progress.lock().unwrap().received += event.bytes.len() as u64;
                    let encoded = match mode {
                        FileMode::Raw => event.bytes.to_vec(),
                        FileMode::Ascii => {
                            let mut output = Vec::new();
                            for byte in event.bytes.iter() {
                                match byte {
                                    b'\r' | b'\n' | b'\t' | 32..=126 => output.push(*byte),
                                    _ => output
                                        .extend_from_slice(format!("\\x{byte:02X}").as_bytes()),
                                }
                            }
                            output
                        }
                        FileMode::Hex => {
                            let mut output = Vec::new();
                            for byte in event.bytes.iter() {
                                output.extend_from_slice(format!("{byte:02X}").as_bytes());
                                column += 1;
                                if column == 16 {
                                    output.push(b'\n');
                                    column = 0;
                                } else {
                                    output.push(b' ');
                                }
                            }
                            output
                        }
                    };
                    writer.write_all(&encoded)?;
                    writer.flush()?;
                    let mut status = progress.lock().unwrap();
                    status.written += encoded.len() as u64;
                    status.dropped = subscription.dropped_events();
                }
                if mode == FileMode::Hex && column != 0 {
                    writer.write_all(b"\n")?;
                }
                writer.flush()?;
                writer.get_ref().sync_all()
            })();
            subscription.close();
            let mut status = progress.lock().unwrap();
            status.dropped = subscription.dropped_events();
            if let Ok(metadata) = writer.get_ref().metadata() {
                status.written = metadata.len();
            }
            status.state = match result {
                Err(e) => RecordingState::Failed(e.to_string()),
                Ok(()) if status.dropped > 0 => RecordingState::Incomplete,
                Ok(()) => RecordingState::Completed,
            };
        });
        Ok(Self {
            control,
            status,
            worker: Some(worker),
        })
    }
    pub fn stop(&self) {
        self.control.close();
        let mut status = self.status.lock().unwrap();
        if status.state == RecordingState::Recording {
            status.state = RecordingState::Finishing;
        }
    }
    pub fn is_active(&self) -> bool {
        self.worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
    }
    pub fn progress(&self) -> RecordingStatus {
        self.status.lock().unwrap().clone()
    }
    pub fn status(&self) -> String {
        let status = self.progress();
        format!(
            "{:?}: {} bytes received / {} bytes written; {} dropped events",
            status.state, status.received, status.written, status.dropped
        )
    }
    pub fn finish(&mut self) {
        self.stop();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for RawRecording {
    fn drop(&mut self) {
        self.finish();
    }
}
