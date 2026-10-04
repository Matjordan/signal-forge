use crate::{
    config::{FlowControl, Parity, SerialSettings},
    endpoint::{ConnectionState, Endpoint, EndpointError, EndpointId},
    traffic::{Direction, TrafficBus},
};
use std::{
    io::{Read, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender, TryRecvError, TrySendError},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

pub fn discover() -> Result<Vec<String>, EndpointError> {
    let mut ports: Vec<_> = serialport::available_ports()
        .map_err(|e| EndpointError::Io(e.to_string()))?
        .into_iter()
        .map(|p| p.port_name)
        .collect();
    ports.sort();
    Ok(ports)
}

pub struct SerialEndpoint {
    id: EndpointId,
    name: String,
    state: Arc<Mutex<ConnectionState>>,
    tx: SyncSender<Vec<u8>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl SerialEndpoint {
    pub fn open(settings: &SerialSettings, bus: TrafficBus) -> Result<Self, EndpointError> {
        if settings.baud == 0 {
            return Err(EndpointError::Io("Baud rate must be positive".into()));
        }
        let data_bits = match settings.data_bits {
            5 => serialport::DataBits::Five,
            6 => serialport::DataBits::Six,
            7 => serialport::DataBits::Seven,
            8 => serialport::DataBits::Eight,
            _ => return Err(EndpointError::Io("Data bits must be 5, 6, 7, or 8".into())),
        };
        let stop_bits = match settings.stop_bits {
            1 => serialport::StopBits::One,
            2 => serialport::StopBits::Two,
            _ => return Err(EndpointError::Io("Stop bits must be 1 or 2".into())),
        };
        let parity = match settings.parity {
            Parity::None => serialport::Parity::None,
            Parity::Odd => serialport::Parity::Odd,
            Parity::Even => serialport::Parity::Even,
        };
        let flow = match settings.flow {
            FlowControl::None => serialport::FlowControl::None,
            FlowControl::Hardware => serialport::FlowControl::Hardware,
            FlowControl::Software => serialport::FlowControl::Software,
        };
        let mut port = serialport::new(&settings.path, settings.baud)
            .data_bits(data_bits)
            .stop_bits(stop_bits)
            .parity(parity)
            .flow_control(flow)
            .timeout(Duration::from_millis(20))
            .open()
            .map_err(|e| EndpointError::Io(format!("{}: {e}", settings.path)))?;
        let id = EndpointId(format!("serial:{}", settings.path));
        let state = Arc::new(Mutex::new(ConnectionState::Connected));
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(64);
        let worker_state = state.clone();
        let worker_stop = stop.clone();
        let worker_id = id.clone();
        let worker = thread::Builder::new()
            .name(format!("serial:{}", settings.path))
            .spawn(move || {
                let mut buffer = [0; 4096];
                let run = || -> Result<(), String> {
                    while !worker_stop.load(Ordering::Acquire) {
                        // Limit TX per iteration so a busy sender cannot starve RX.
                        for _ in 0..8 {
                            if worker_stop.load(Ordering::Acquire) {
                                break;
                            }
                            match rx.try_recv() {
                                Ok(bytes) => {
                                    // Report only the bytes actually accepted, including
                                    // a partial write before a device fails.
                                    let mut offset = 0;
                                    while offset < bytes.len() {
                                        if worker_stop.load(Ordering::Acquire) {
                                            return Ok(());
                                        }
                                        match port.write(&bytes[offset..]) {
                                            Ok(0) => {
                                                return Err(
                                                    "serial write returned zero bytes".into()
                                                )
                                            }
                                            Ok(count) => {
                                                bus.publish(
                                                    worker_id.clone(),
                                                    Direction::Tx,
                                                    &bytes[offset..offset + count],
                                                );
                                                offset += count;
                                            }
                                            Err(e)
                                                if e.kind() == std::io::ErrorKind::Interrupted =>
                                            {
                                                continue
                                            }
                                            Err(e) => return Err(e.to_string()),
                                        }
                                    }
                                }
                                Err(TryRecvError::Empty) => break,
                                Err(TryRecvError::Disconnected) => return Ok(()),
                            }
                        }
                        if worker_stop.load(Ordering::Acquire) {
                            break;
                        }
                        match port.read(&mut buffer) {
                            Ok(0) => return Err("serial device closed".into()),
                            Ok(count) => {
                                bus.publish(worker_id.clone(), Direction::Rx, &buffer[..count])
                            }
                            Err(e)
                                if matches!(
                                    e.kind(),
                                    std::io::ErrorKind::TimedOut
                                        | std::io::ErrorKind::WouldBlock
                                        | std::io::ErrorKind::Interrupted
                                ) => {}
                            Err(e) => return Err(e.to_string()),
                        }
                    }
                    Ok(())
                };
                let result = {
                    let mut run = run;
                    run()
                };
                let next = match result {
                    Ok(()) => ConnectionState::Disconnected,
                    Err(error) => {
                        log::error!("{}: {error}", worker_id.0);
                        ConnectionState::Fault(error)
                    }
                };
                *worker_state.lock().unwrap_or_else(|e| e.into_inner()) = next;
            })
            .map_err(|e| EndpointError::Io(e.to_string()))?;
        log::info!("Opened {} at {} baud", settings.path, settings.baud);
        Ok(Self {
            id,
            name: settings.path.clone(),
            state,
            tx,
            stop,
            worker: Some(worker),
        })
    }
}
impl Endpoint for SerialEndpoint {
    fn id(&self) -> &EndpointId {
        &self.id
    }
    fn display_name(&self) -> &str {
        &self.name
    }
    fn state(&self) -> ConnectionState {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    fn send(&self, bytes: Vec<u8>) -> Result<(), EndpointError> {
        if self.state() != ConnectionState::Connected || self.stop.load(Ordering::Acquire) {
            return Err(EndpointError::Disconnected);
        }
        if bytes.len() > 65536 {
            return Err(EndpointError::Io(
                "A single message cannot exceed 64 KiB".into(),
            ));
        }
        self.tx.try_send(bytes).map_err(|e| match e {
            TrySendError::Full(_) => EndpointError::QueueFull,
            TrySendError::Disconnected(_) => EndpointError::Disconnected,
        })
    }
    fn disconnect(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                log::error!("{} worker panicked", self.name);
            }
        }
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = ConnectionState::Disconnected;
    }
}
impl Drop for SerialEndpoint {
    fn drop(&mut self) {
        self.disconnect();
    }
}
