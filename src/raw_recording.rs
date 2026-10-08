//! Raw RX bytes recorded by a dedicated subscriber, independent of display state.
use crate::{
    endpoint::EndpointId,
    traffic::{Direction, TrafficBus},
};
use std::{
    fs::OpenOptions,
    io::{BufWriter, Write},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

pub struct RawRecording {
    stop: Arc<AtomicBool>,
    status: Arc<Mutex<String>>,
    worker: Option<JoinHandle<()>>,
}
impl RawRecording {
    pub fn start(path: &Path, endpoint: EndpointId, bus: &TrafficBus) -> Result<Self, String> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| e.to_string())?;
        let subscription = bus.subscribe_tracked(4096);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let status = Arc::new(Mutex::new("Recording".to_string()));
        let progress = status.clone();
        let worker = thread::spawn(move || {
            let mut writer = BufWriter::new(file);
            let mut bytes = 0u64;
            let mut closed = false;
            let result = (|| -> Result<(), std::io::Error> {
                loop {
                    if stopping.load(Ordering::Acquire) && !closed {
                        subscription.close();
                        closed = true;
                    }
                    match subscription
                        .receiver
                        .recv_timeout(Duration::from_millis(20))
                    {
                        Ok(event)
                            if event.endpoint == endpoint && event.direction == Direction::Rx =>
                        {
                            writer.write_all(&event.bytes)?;
                            bytes += event.bytes.len() as u64;
                        }
                        Ok(_) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
                writer.flush()?;
                writer.get_ref().sync_all()
            })();
            subscription.close();
            let drops = subscription.dropped_events();
            *progress.lock().unwrap() = match result {
                Err(e) => format!("Recording failed: {e}"),
                Ok(()) if drops > 0 => {
                    format!("Incomplete: {bytes} bytes, {drops} dropped monitoring events")
                }
                Ok(()) => format!("Completed: {bytes} bytes"),
            };
        });
        Ok(Self {
            stop,
            status,
            worker: Some(worker),
        })
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        let mut status = self.status.lock().unwrap();
        if *status == "Recording" {
            *status = "Finishing".into();
        }
    }
    pub fn is_active(&self) -> bool {
        self.worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
    }
    pub fn status(&self) -> String {
        self.status.lock().unwrap().clone()
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
