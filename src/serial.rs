use crate::{
    bridge::{BridgePort, BridgeWriter},
    config::{FlowControl, Parity, SerialSettings},
    endpoint::{ConnectionState, Endpoint, EndpointError, EndpointId},
    repeat::{RepeatHandle, RepeatJob, RepeatSpec},
    traffic::{Direction, TrafficBus},
};
use std::{
    io::Read,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender, TryRecvError, TrySendError},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
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

enum Command {
    Send(Vec<u8>),
    Repeat(RepeatJob),
}

pub struct SerialEndpoint {
    id: EndpointId,
    name: String,
    state: Arc<Mutex<ConnectionState>>,
    tx: SyncSender<Command>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    bridge: BridgePort,
    writer: Arc<SerialWriter>,
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
        let port = serialport::new(&settings.path, settings.baud)
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
        let mut reader = port.try_clone().map_err(|e| EndpointError::Io(e.to_string()))?;
        let writer = Arc::new(SerialWriter { port: Mutex::new(Some(port)), state: state.clone(), stop: stop.clone(), bus: bus.clone(), id: id.clone() });
        let bridge = BridgePort::new(id.clone(), writer.clone());
        let worker_bridge = bridge.clone();
        let worker_writer = writer.clone();
        let (tx, rx) = mpsc::sync_channel::<Command>(64);
        let worker_state = state.clone();
        let worker_stop = stop.clone();
        let worker_id = id.clone();
        let worker = thread::Builder::new()
            .name(format!("serial:{}", settings.path))
            .spawn(move || {
                let mut buffer = [0; 4096];
                let mut repeat: Option<RepeatJob> = None;
                let result = (|| -> Result<(), EndpointError> {
                    while !worker_stop.load(Ordering::Acquire) {
                        // Bound command processing so manual sends cannot starve RX or timers.
                        for _ in 0..8 {
                            if worker_stop.load(Ordering::Acquire) {
                                break;
                            }
                            match rx.try_recv() {
                                Ok(Command::Send(bytes)) => {
                                    if !worker_writer.write(&bytes, &worker_stop)? {
                                        return Ok(());
                                    }
                                }
                                Ok(Command::Repeat(job)) => {
                                    if job.is_active() {
                                        repeat = Some(job);
                                    }
                                }
                                Err(TryRecvError::Empty) => break,
                                Err(TryRecvError::Disconnected) => return Ok(()),
                            }
                        }
                        if worker_stop.load(Ordering::Acquire) {
                            break;
                        }
                        if let Some(job) = &mut repeat {
                            job.poll(Instant::now(), |bytes, cancelled| {
                                worker_writer.write(bytes, cancelled)
                            })?;
                        }
                        if repeat.as_ref().is_some_and(|job| !job.is_active()) {
                            repeat = None;
                        }
                        // Shorten RX waits to the next timer deadline. Serial reads remain
                        // bounded when a job is stopped, even for very long intervals.
                        let timeout = repeat
                            .as_ref()
                            .map(|job| job.time_until_next(Instant::now()))
                            .unwrap_or(Duration::from_millis(20))
                            .clamp(Duration::from_millis(1), Duration::from_millis(20));
                        reader.set_timeout(timeout)
                            .map_err(|e| EndpointError::Io(e.to_string()))?;
                        match reader.read(&mut buffer) {
                            Ok(0) => return Err(EndpointError::Io("serial device closed".into())),
                            Ok(count) => {
                                bus.publish(worker_id.clone(), Direction::Rx, &buffer[..count]);
                                worker_bridge.receive(&buffer[..count]);
                            }
                            Err(e)
                                if matches!(
                                    e.kind(),
                                    std::io::ErrorKind::TimedOut
                                        | std::io::ErrorKind::WouldBlock
                                        | std::io::ErrorKind::Interrupted
                                ) => {}
                            Err(e) => return Err(EndpointError::Io(e.to_string())),
                        }
                    }
                    Ok(())
                })();
                if let (Some(job), Err(error)) = (&repeat, &result) {
                    job.fail(&error.to_string());
                }
                // Active and still-queued repeat commands cancel when their owners drop.
                drop(repeat);
                let next = match result {
                    Ok(()) => {
                        let state = worker_state.lock().unwrap_or_else(|e| e.into_inner()).clone();
                        if matches!(state, ConnectionState::Fault(_)) { state } else { ConnectionState::Disconnected }
                    },
                    Err(error) => {
                        log::error!("{}: {error}", worker_id.0);
                        ConnectionState::Fault(error.to_string())
                    }
                };
                worker_writer.close();
                worker_bridge.endpoint_closed(&format!("{next:?}"));
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
            bridge,
            writer,
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
        self.tx.try_send(Command::Send(bytes)).map_err(|e| match e {
            TrySendError::Full(_) => EndpointError::QueueFull,
            TrySendError::Disconnected(_) => EndpointError::Disconnected,
        })
    }
    fn start_repeat(
        &self,
        bytes: Vec<u8>,
        spec: RepeatSpec,
    ) -> Result<RepeatHandle, EndpointError> {
        if self.state() != ConnectionState::Connected || self.stop.load(Ordering::Acquire) {
            return Err(EndpointError::Disconnected);
        }
        let (job, handle) = RepeatJob::new(bytes, spec)?;
        self.tx
            .try_send(Command::Repeat(job))
            .map_err(|e| match e {
                TrySendError::Full(_) => EndpointError::QueueFull,
                TrySendError::Disconnected(_) => EndpointError::Disconnected,
            })?;
        Ok(handle)
    }
    fn bridge_port(&self) -> Result<BridgePort, EndpointError> {
        if self.state() != ConnectionState::Connected { return Err(EndpointError::Disconnected); }
        Ok(self.bridge.clone())
    }
    fn disconnect(&mut self) {
        self.bridge.endpoint_closed("disconnected");
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                log::error!("{} worker panicked", self.name);
            }
        }
        self.writer.close();
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = ConnectionState::Disconnected;
    }
}
impl Drop for SerialEndpoint {
    fn drop(&mut self) {
        self.disconnect();
    }
}

fn write_payload(
    port: &mut dyn serialport::SerialPort,
    bytes: &[u8],
    stop: &AtomicBool,
    cancelled: Option<&AtomicBool>,
    bus: &TrafficBus,
    id: &EndpointId,
) -> Result<bool, EndpointError> {
    let mut offset = 0;
    while offset < bytes.len() {
        if stop.load(Ordering::Acquire)
            || cancelled.is_some_and(|flag| flag.load(Ordering::Acquire))
        {
            return Ok(false);
        }
        match port.write(&bytes[offset..]) {
            Ok(0) => return Err(EndpointError::Io("serial write returned zero bytes".into())),
            Ok(count) => {
                bus.publish(id.clone(), Direction::Tx, &bytes[offset..offset + count]);
                offset += count;
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(EndpointError::Io(e.to_string())),
        }
    }
    Ok(true)
}

struct SerialWriter {
    port: Mutex<Option<Box<dyn serialport::SerialPort>>>,
    state: Arc<Mutex<ConnectionState>>,
    stop: Arc<AtomicBool>,
    bus: TrafficBus,
    id: EndpointId,
}
impl BridgeWriter for SerialWriter {
    fn state(&self) -> ConnectionState { self.state.lock().unwrap_or_else(|e| e.into_inner()).clone() }
    fn write(&self, bytes: &[u8], cancelled: &AtomicBool) -> Result<bool, EndpointError> {
        if self.stop.load(Ordering::Acquire) { return Ok(false); }
        let mut port = self.port.lock().unwrap_or_else(|e| e.into_inner());
        let Some(port) = port.as_mut() else { return Err(EndpointError::Disconnected); };
        let result = write_payload(&mut **port, bytes, &self.stop, Some(cancelled), &self.bus, &self.id);
        if let Err(error) = &result {
            *self.state.lock().unwrap_or_else(|e| e.into_inner()) = ConnectionState::Fault(error.to_string());
            self.stop.store(true, Ordering::Release);
        }
        result
    }
}

impl SerialWriter {
    fn close(&self) { self.port.lock().unwrap_or_else(|e| e.into_inner()).take(); }
}
