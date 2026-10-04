//! Bridges use direct lossless transport writes, never the lossy monitor bus.
use crate::{
    endpoint::{ConnectionState, EndpointError, EndpointId},
    traffic::{Direction, TrafficBus, TrafficEvent},
};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc::Receiver,
    Arc, Mutex,
};

pub trait BridgeWriter: Send + Sync {
    fn state(&self) -> ConnectionState;
    /// Must serialize with other writes to this endpoint and inspect cancellation
    /// between partial writes. True means the complete payload was accepted.
    fn write(&self, bytes: &[u8], cancelled: &AtomicBool) -> Result<bool, EndpointError>;
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeState {
    Running,
    Stopped,
    Fault(String),
}
struct Shared {
    id: u64,
    stopped: AtomicBool,
    state: Mutex<BridgeState>,
    monitor: TrafficBus,
}
#[derive(Clone)]
struct Route {
    shared: Arc<Shared>,
    destination: Arc<dyn BridgeWriter>,
    source: EndpointId,
    destination_id: EndpointId,
}
impl Route {
    fn fault(&self, error: String) {
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if !self.shared.stopped.swap(true, Ordering::AcqRel) {
            *state = BridgeState::Fault(error);
        }
    }
    fn forward(&self, bytes: &[u8]) {
        if self.shared.stopped.load(Ordering::Acquire) {
            return;
        }
        if self.destination.state() != ConnectionState::Connected {
            self.fault(format!("{} disconnected", self.destination_id.0));
            return;
        }
        match self.destination.write(bytes, &self.shared.stopped) {
            Ok(true) => self
                .shared
                .monitor
                .publish(self.source.clone(), Direction::Rx, bytes),
            Ok(false) => {}
            Err(error) => self.fault(format!(
                "{} -> {}: {error}",
                self.source.0, self.destination_id.0
            )),
        }
    }
}

/// Transport-neutral attachment point. The transport calls receive() on its
/// RX path and endpoint_closed() on disconnect/fault, outside monitor delivery.
#[derive(Clone)]
pub struct BridgePort {
    pub id: EndpointId,
    writer: Arc<dyn BridgeWriter>,
    route: Arc<Mutex<Option<Route>>>,
}
impl BridgePort {
    pub fn new(id: EndpointId, writer: Arc<dyn BridgeWriter>) -> Self {
        Self {
            id,
            writer,
            route: Arc::new(Mutex::new(None)),
        }
    }
    pub fn receive(&self, bytes: &[u8]) {
        let route = self.route.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(route) = route {
            route.forward(bytes);
        }
    }
    pub fn endpoint_closed(&self, reason: &str) {
        let route = self.route.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(route) = route {
            route.fault(format!("{}: {reason}", self.id.0));
        }
    }
    pub fn is_bridged(&self) -> bool {
        self.route
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|r| !r.shared.stopped.load(Ordering::Acquire))
    }
}

pub struct Bridge {
    pub a: EndpointId,
    pub b: EndpointId,
    ports: [BridgePort; 2],
    shared: Arc<Shared>,
}
impl Bridge {
    pub fn start(a: BridgePort, b: BridgePort) -> Result<Self, EndpointError> {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        if a.id == b.id || Arc::ptr_eq(&a.route, &b.route) {
            return Err(EndpointError::Io("Select two different endpoints".into()));
        }
        if a.writer.state() != ConnectionState::Connected
            || b.writer.state() != ConnectionState::Connected
        {
            return Err(EndpointError::Disconnected);
        }
        let shared = Arc::new(Shared {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            stopped: AtomicBool::new(true),
            state: Mutex::new(BridgeState::Running),
            monitor: TrafficBus::default(),
        });
        // Stable lock ordering also makes simultaneous bridge creation safe.
        let (first, second) = if a.id.0 < b.id.0 { (&a, &b) } else { (&b, &a) };
        let mut first_route = first.route.lock().unwrap_or_else(|e| e.into_inner());
        let mut second_route = second.route.lock().unwrap_or_else(|e| e.into_inner());
        if [&*first_route, &*second_route].iter().any(|route| {
            route
                .as_ref()
                .is_some_and(|r| !r.shared.stopped.load(Ordering::Acquire))
        }) {
            return Err(EndpointError::Io(
                "An endpoint already belongs to a running bridge".into(),
            ));
        }
        if a.writer.state() != ConnectionState::Connected
            || b.writer.state() != ConnectionState::Connected
        {
            return Err(EndpointError::Disconnected);
        }
        *first_route = Some(Route {
            shared: shared.clone(),
            destination: second.writer.clone(),
            source: first.id.clone(),
            destination_id: second.id.clone(),
        });
        *second_route = Some(Route {
            shared: shared.clone(),
            destination: first.writer.clone(),
            source: second.id.clone(),
            destination_id: first.id.clone(),
        });
        shared.stopped.store(false, Ordering::Release);
        drop(first_route);
        drop(second_route);
        Ok(Self {
            a: a.id.clone(),
            b: b.id.clone(),
            ports: [a, b],
            shared,
        })
    }
    pub fn subscribe_tracked(&self, capacity: usize) -> crate::traffic::TrafficSubscription { self.shared.monitor.subscribe_tracked(capacity) }
    pub fn subscribe(&self, capacity: usize) -> Receiver<Arc<TrafficEvent>> {
        self.shared.monitor.subscribe(capacity)
    }
    pub fn state(&self) -> BridgeState {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub fn dropped_events(&self) -> u64 {
        self.shared.monitor.dropped_events()
    }
    pub fn stop(&mut self) {
        self.shared.stopped.store(true, Ordering::Release);
        *self.shared.state.lock().unwrap_or_else(|e| e.into_inner()) = BridgeState::Stopped;
        for port in &self.ports {
            let mut route = port.route.lock().unwrap_or_else(|e| e.into_inner());
            if route
                .as_ref()
                .is_some_and(|r| r.shared.id == self.shared.id)
            {
                *route = None;
            }
        }
    }
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.stop();
    }
}
