//! Bounded display history; filters and pause affect presentation only.
use crate::{endpoint::EndpointId, traffic::TrafficEvent};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
pub const HISTORY_LIMIT: usize = 1000;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeDirection {
    AToB,
    BToA,
}
impl BridgeDirection {
    pub fn label(self) -> &'static str {
        match self {
            Self::AToB => "A → B",
            Self::BToA => "B → A",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DirectionFilter {
    #[default]
    Both,
    AToB,
    BToA,
}
impl DirectionFilter {
    pub fn accepts(self, direction: BridgeDirection) -> bool {
        match self {
            Self::Both => true,
            Self::AToB => direction == BridgeDirection::AToB,
            Self::BToA => direction == BridgeDirection::BToA,
        }
    }
}
pub fn timestamp_ns(timestamp: SystemTime) -> i128 {
    match timestamp.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_nanos() as i128,
        Err(e) => -(e.duration().as_nanos() as i128),
    }
}
/// Full UTC date and time without adding a clock/locale dependency.
pub fn timestamp_utc(timestamp: SystemTime) -> String {
    let ns = timestamp_ns(timestamp);
    let seconds = ns.div_euclid(1_000_000_000);
    let days = seconds.div_euclid(86400);
    let daytime = seconds.rem_euclid(86400);
    // Civil calendar conversion from days since Unix epoch (Gregorian).
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i128::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}.{:09} UTC",
        daytime / 3600,
        daytime / 60 % 60,
        daytime % 60,
        ns.rem_euclid(1_000_000_000)
    )
}
pub struct InspectorRow {
    pub event: Arc<TrafficEvent>,
    pub direction: BridgeDirection,
    pub delta_ns: Option<i128>,
    pub missed_before: u64,
}
#[derive(Default)]
pub struct Inspector {
    pub rows: VecDeque<InspectorRow>,
    pub paused: bool,
    pub filter: DirectionFilter,
    pub deltas: bool,
    pub auto_scroll: bool,
    previous: Option<(u64, i128)>,
}
impl Inspector {
    pub fn new() -> Self {
        Self {
            deltas: true,
            auto_scroll: true,
            ..Default::default()
        }
    }
    pub fn receive(&mut self, event: Arc<TrafficEvent>, a: &EndpointId) {
        let time = timestamp_ns(event.timestamp);
        let delta_ns = self.previous.map(|(_, previous)| time - previous);
        let missed_before = self
            .previous
            .map(|(seq, _)| event.sequence.saturating_sub(seq + 1))
            .unwrap_or(0);
        self.previous = Some((event.sequence, time));
        if self.paused {
            return;
        }
        let direction = if &event.endpoint == a {
            BridgeDirection::AToB
        } else {
            BridgeDirection::BToA
        };
        self.rows.push_back(InspectorRow {
            event,
            direction,
            delta_ns,
            missed_before,
        });
        while self.rows.len() > HISTORY_LIMIT {
            self.rows.pop_front();
        }
    }
    pub fn clear(&mut self) {
        self.rows.clear();
    }
}
