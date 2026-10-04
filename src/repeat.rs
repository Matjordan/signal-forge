//! Scheduling is driven by transport workers, independently of GUI repainting.
use crate::endpoint::EndpointError;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
pub struct RepeatSpec {
    pub interval: Duration,
    /// Includes the initial send. None repeats until explicitly stopped.
    pub count: Option<u64>,
}

impl RepeatSpec {
    pub fn validate(&self) -> Result<(), EndpointError> {
        if self.interval < Duration::from_millis(1) || self.interval > Duration::from_secs(86400) {
            return Err(EndpointError::Io(
                "Repeat interval must be between 1 ms and 24 hours".into(),
            ));
        }
        if self.count == Some(0) {
            return Err(EndpointError::Io(
                "Repeat count must be positive, or continuous".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepeatState {
    Pending,
    Running,
    Completed,
    Cancelled,
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct RepeatStatus {
    pub state: RepeatState,
    /// Number of fully written payloads, not merely queued send requests.
    pub sent: u64,
    pub count: Option<u64>,
}

struct Shared {
    cancelled: AtomicBool,
    status: Mutex<RepeatStatus>,
}

/// Clonable observer/cancellation token. Dropping an observer does not stop a job.
#[derive(Clone)]
pub struct RepeatHandle(Arc<Shared>);
impl RepeatHandle {
    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::Release);
        let mut status = self.0.status.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(status.state, RepeatState::Pending | RepeatState::Running) {
            status.state = RepeatState::Cancelled;
        }
    }
    pub fn status(&self) -> RepeatStatus {
        self.0
            .status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub fn is_active(&self) -> bool {
        matches!(
            self.status().state,
            RepeatState::Pending | RepeatState::Running
        )
    }
}

/// Owned by a transport worker or its command queue. Dropping queued/active jobs
/// cancels them, including when an endpoint faults before consuming the command.
pub struct RepeatJob {
    payload: Vec<u8>,
    spec: RepeatSpec,
    next: Instant,
    handle: RepeatHandle,
}
impl RepeatJob {
    pub fn new(payload: Vec<u8>, spec: RepeatSpec) -> Result<(Self, RepeatHandle), EndpointError> {
        if payload.is_empty() || payload.len() > 65536 {
            return Err(EndpointError::Io(
                "Repeat payload must contain 1–65536 bytes".into(),
            ));
        }
        spec.validate()?;
        let handle = RepeatHandle(Arc::new(Shared {
            cancelled: AtomicBool::new(false),
            status: Mutex::new(RepeatStatus {
                state: RepeatState::Pending,
                sent: 0,
                count: spec.count,
            }),
        }));
        Ok((
            Self {
                payload,
                spec,
                next: Instant::now(),
                handle: handle.clone(),
            },
            handle,
        ))
    }

    pub fn time_until_next(&self, now: Instant) -> Duration {
        self.next.saturating_duration_since(now)
    }
    pub fn is_active(&self) -> bool {
        self.handle.is_active()
    }
    pub fn fail(&self, error: &str) {
        let mut status = self
            .handle
            .0
            .status
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if matches!(status.state, RepeatState::Pending | RepeatState::Running) {
            status.state = RepeatState::Failed(error.to_owned());
        }
    }

    /// At most one send per poll. The next interval starts after a complete
    /// write, so delayed workers do not produce catch-up bursts. A writer must
    /// inspect the cancellation flag between partial writes. False indicates
    /// cancellation during a write; already accepted bytes cannot be recalled.
    pub fn poll<F>(&mut self, now: Instant, mut write: F) -> Result<(), EndpointError>
    where
        F: FnMut(&[u8], &AtomicBool) -> Result<bool, EndpointError>,
    {
        if !self.is_active() || self.handle.0.cancelled.load(Ordering::Acquire) || now < self.next {
            return Ok(());
        }
        {
            let mut status = self
                .handle
                .0
                .status
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if status.state == RepeatState::Cancelled {
                return Ok(());
            }
            status.state = RepeatState::Running;
        }
        match write(&self.payload, &self.handle.0.cancelled) {
            Ok(true) => {
                let mut status = self
                    .handle
                    .0
                    .status
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                status.sent += 1;
                if status.state != RepeatState::Cancelled {
                    status.state = if self.spec.count.is_some_and(|count| status.sent >= count) {
                        RepeatState::Completed
                    } else {
                        RepeatState::Running
                    };
                }
                self.next = Instant::now() + self.spec.interval;
                Ok(())
            }
            Ok(false) => {
                self.handle.cancel();
                Ok(())
            }
            Err(error) => {
                self.fail(&error.to_string());
                Err(error)
            }
        }
    }
}
impl Drop for RepeatJob {
    fn drop(&mut self) {
        self.handle.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn count_includes_initial_send_and_delays_do_not_catch_up() {
        let (mut job, handle) = RepeatJob::new(
            vec![0xff],
            RepeatSpec {
                interval: Duration::from_millis(10),
                count: Some(3),
            },
        )
        .unwrap();
        let now = Instant::now();
        let mut writes = 0;
        job.poll(now, |_, _| {
            writes += 1;
            Ok(true)
        })
        .unwrap();
        job.poll(now, |_, _| {
            writes += 1;
            Ok(true)
        })
        .unwrap();
        assert_eq!(writes, 1);
        job.poll(now + Duration::from_secs(5), |_, _| {
            writes += 1;
            Ok(true)
        })
        .unwrap();
        assert_eq!(writes, 2);
        job.poll(Instant::now() + Duration::from_secs(10), |_, _| {
            writes += 1;
            Ok(true)
        })
        .unwrap();
        assert_eq!(handle.status().state, RepeatState::Completed);
        job.poll(Instant::now() + Duration::from_secs(20), |_, _| {
            writes += 1;
            Ok(true)
        })
        .unwrap();
        assert_eq!(writes, 3);
    }
    #[test]
    fn queued_cancel_drop_and_writer_fault_are_reported() {
        let spec = RepeatSpec {
            interval: Duration::from_secs(1),
            count: None,
        };
        let (mut job, handle) = RepeatJob::new(vec![1], spec).unwrap();
        handle.cancel();
        job.poll(Instant::now(), |_, _| panic!("cancelled job wrote bytes"))
            .unwrap();
        assert_eq!(handle.status().sent, 0);
        let (job, handle) = RepeatJob::new(vec![1], spec).unwrap();
        drop(job);
        assert_eq!(handle.status().state, RepeatState::Cancelled);
        let (mut job, handle) = RepeatJob::new(vec![1], spec).unwrap();
        assert!(job
            .poll(Instant::now(), |_, _| Err(EndpointError::Io(
                "unplugged".into()
            )))
            .is_err());
        assert!(matches!(handle.status().state, RepeatState::Failed(_)));
    }
    #[test]
    fn invalid_schedules_are_rejected_before_queueing() {
        for spec in [
            RepeatSpec {
                interval: Duration::ZERO,
                count: None,
            },
            RepeatSpec {
                interval: Duration::from_millis(10),
                count: Some(0),
            },
        ] {
            assert!(RepeatJob::new(vec![1], spec).is_err());
        }
    }
}
