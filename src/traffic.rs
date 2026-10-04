use crate::endpoint::EndpointId;
use std::sync::{
    mpsc::{self, Receiver, SyncSender, TrySendError},
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Rx,
    Tx,
}

#[derive(Debug, Clone)]
pub struct TrafficEvent {
    pub sequence: u64,
    pub timestamp: SystemTime,
    pub endpoint: EndpointId,
    pub direction: Direction,
    pub bytes: Arc<[u8]>,
}

struct Subscriber {
    sender: SyncSender<Arc<TrafficEvent>>,
    dropped: Arc<AtomicU64>,
    id: u64,
}
#[derive(Default)]
struct Inner {
    sequence: u64,
    next_subscriber: u64,
    subscribers: Vec<Subscriber>,
}

/// Bounded, best-effort monitoring bus. Publishing never waits for a consumer.
/// Sequence gaps identify dropped events; the forwarding path must use the
/// endpoint byte stream directly, never a lossy monitoring subscription.
#[derive(Clone, Default)]
pub struct TrafficBus(Arc<Mutex<Inner>>);

impl TrafficBus {
    pub fn subscribe(&self, capacity: usize) -> Receiver<Arc<TrafficEvent>> {
        self.subscribe_tracked(capacity).receiver
    }
    pub fn subscribe_tracked(&self, capacity: usize) -> TrafficSubscription {
        let (sender, receiver) = mpsc::sync_channel(capacity.max(1));
        let dropped = Arc::new(AtomicU64::new(0));
        let mut inner = self.0.lock().unwrap_or_else(|e| e.into_inner());
        inner.next_subscriber += 1;
        let id = inner.next_subscriber;
        inner.subscribers.push(Subscriber { sender, dropped: dropped.clone(), id });
        TrafficSubscription { receiver, dropped, id, capacity: capacity.max(1), bus: self.clone() }
    }

    pub fn publish(&self, endpoint: EndpointId, direction: Direction, bytes: &[u8]) {
        // Serialize sequence allocation and fan-out so every subscriber observes
        // the same ordering, even with independent serial worker threads.
        let mut inner = self.0.lock().unwrap_or_else(|e| e.into_inner());
        inner.sequence += 1;
        let event = Arc::new(TrafficEvent {
            sequence: inner.sequence,
            timestamp: SystemTime::now(),
            endpoint,
            direction,
            bytes: Arc::from(bytes),
        });
        inner.subscribers.retain_mut(|subscriber| {
            match subscriber.sender.try_send(event.clone()) {
                Ok(()) => true,
                Err(TrySendError::Full(_)) => {
                    subscriber.dropped.fetch_add(1, Ordering::Relaxed);
                    true
                }
                Err(TrySendError::Disconnected(_)) => false,
            }
        });
    }

    pub fn dropped_events(&self) -> u64 {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .subscribers
            .iter()
            .map(|s| s.dropped.load(Ordering::Relaxed))
            .sum()
    }
}

/// A dedicated consumer's exact drop count and a synchronized capture cutoff.
/// Closing detaches the sender under the publish lock; queued events can then
/// be drained without newly arriving traffic extending the capture indefinitely.
pub struct TrafficSubscription {
    pub receiver: Receiver<Arc<TrafficEvent>>,
    pub capacity: usize,
    dropped: Arc<AtomicU64>,
    id: u64,
    bus: TrafficBus,
}
impl TrafficSubscription {
    pub fn dropped_events(&self) -> u64 { self.dropped.load(Ordering::Relaxed) }
    pub fn close(&self) {
        self.bus.0.lock().unwrap_or_else(|e| e.into_inner()).subscribers.retain(|s| s.id != self.id);
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Keep arbitrary binary data readable without lossy UTF-8 conversion.
pub fn ascii(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| match b {
            b'\r' => "\\r".to_owned(),
            b'\n' => "\\n".to_owned(),
            b'\t' => "\\t".to_owned(),
            0x20..=0x7e => char::from(*b).to_string(),
            _ => format!("\\x{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fan_out_preserves_order_and_does_not_wait_for_slow_consumers() {
        let bus = TrafficBus::default();
        let slow = bus.subscribe(1);
        let fast = bus.subscribe(8);
        for byte in 0..4 {
            bus.publish(EndpointId("test".into()), Direction::Rx, &[byte]);
        }
        assert_eq!(bus.dropped_events(), 3);
        assert_eq!(slow.try_recv().unwrap().bytes.as_ref(), &[0]);
        let events: Vec<_> = fast.try_iter().collect();
        assert_eq!(
            events.iter().map(|e| e.sequence).collect::<Vec<_>>(),
            vec![1, 2, 3, 4]
        );
        assert_eq!(
            events.iter().map(|e| e.bytes[0]).collect::<Vec<_>>(),
            vec![0, 1, 2, 3]
        );
    }
    #[test]
    fn concurrent_publishers_have_one_global_order() {
        let bus = TrafficBus::default();
        let receiver = bus.subscribe(200);
        let handles: Vec<_> = (0..2)
            .map(|index| {
                let bus = bus.clone();
                std::thread::spawn(move || {
                    for _ in 0..100 {
                        bus.publish(EndpointId(index.to_string()), Direction::Tx, &[0xff]);
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(
            receiver.try_iter().map(|e| e.sequence).collect::<Vec<_>>(),
            (1..=200).collect::<Vec<_>>()
        );
    }
    #[test]
    fn binary_display_is_lossless() {
        assert_eq!(ascii(&[0, 255, b'\r', b'\n']), "\\x00\\xFF\\r\\n");
        assert_eq!(hex(&[0, 255]), "00 FF");
    }
}
