//! Bundled user documentation and deterministic continuous-hover timing.
use std::sync::OnceLock;

pub const GUIDE: &str = include_str!("../docs/user-guide.md");
pub const HOVER_SECONDS: f64 = 10.0;

#[derive(Debug)]
pub struct Topic {
    pub id: &'static str,
    pub title: &'static str,
    pub tooltip: &'static str,
    pub expanded: &'static str,
    pub body: &'static str,
}
impl Topic {
    pub fn anchor(&self) -> String {
        self.title.to_lowercase().replace(' ', "-")
    }
    pub fn matches(&self, query: &str) -> bool {
        let haystack =
            format!("{} {} {} {}", self.id, self.title, self.tooltip, self.body).to_lowercase();
        query
            .split_whitespace()
            .all(|word| haystack.contains(&word.to_lowercase()))
    }
}

pub fn topics() -> &'static [Topic] {
    static TOPICS: OnceLock<Vec<Topic>> = OnceLock::new();
    TOPICS.get_or_init(|| {
        GUIDE
            .split("<!-- topic:")
            .skip(1)
            .map(|section| {
                let (id, section) = section.split_once(" -->").expect("Help topic ID");
                let section = section.trim();
                let (title, section) = section
                    .strip_prefix("## ")
                    .expect("Help title")
                    .split_once('\n')
                    .unwrap();
                let (tooltip, body) = section
                    .trim_start()
                    .strip_prefix("> ")
                    .expect("Help tooltip")
                    .split_once('\n')
                    .unwrap();
                let body = body.trim();
                let expanded = body.split_once("\n\n").map_or(body, |(first, _)| first);
                Topic {
                    id,
                    title,
                    tooltip,
                    expanded,
                    body,
                }
            })
            .collect()
    })
}
pub fn topic(id: &str) -> Option<&'static Topic> {
    topics().iter().find(|topic| topic.id == id)
}
pub fn by_anchor(anchor: &str) -> Option<&'static Topic> {
    topics().iter().find(|topic| topic.anchor() == anchor)
}

/// Dismissal is sticky for this hover. Leaving or replacing the target resets it.
#[derive(Debug, Default)]
pub struct HoverTimer {
    target: Option<u64>,
    started: f64,
    dismissed: bool,
}
impl HoverTimer {
    pub fn update(&mut self, now: f64, target: Option<u64>, interacting: bool) -> bool {
        if self.target != target {
            self.target = target;
            self.started = now;
            self.dismissed = false;
        }
        if interacting {
            self.started = now;
            self.dismissed = true;
        }
        target.is_some() && !self.dismissed && now - self.started >= HOVER_SECONDS
    }
    pub fn dismiss(&mut self) {
        self.dismissed = true;
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn documentation_ids_content_and_links_are_valid() {
        let mut ids = std::collections::HashSet::new();
        let mut anchors = std::collections::HashSet::new();
        assert!(topics().len() >= 30);
        for topic in topics() {
            assert!(ids.insert(topic.id), "Duplicate {}", topic.id);
            assert!(anchors.insert(topic.anchor()));
            assert!(!topic.tooltip.is_empty() && !topic.expanded.is_empty());
            assert!(topic.body.len() > 400, "Incomplete {}", topic.id);
            for link in topic.body.split("](#").skip(1) {
                let anchor = link.split_once(')').unwrap().0;
                assert!(
                    by_anchor(anchor).is_some(),
                    "{} has broken link {}",
                    topic.id,
                    anchor
                );
            }
        }
        for id in [
            "tx.checksum.skip_first",
            "virtual_pair.timing",
            "terminal.control_characters",
            "selection.inspector",
        ] {
            assert!(topic(id).is_some());
        }
    }
    #[test]
    fn search_finds_workflows_and_diagnostics() {
        assert!(topic("troubleshoot.virtual")
            .unwrap()
            .matches("pyserial container"));
        assert!(topic("tx.checksum.skip_first")
            .unwrap()
            .matches("SKIP first"));
        assert!(!topic("tx.checksum").unwrap().matches("nonexistent-token"));
    }
    #[test]
    fn hover_requires_ten_continuous_seconds_and_resets_on_leave_or_replacement() {
        let mut timer = HoverTimer::default();
        assert!(!timer.update(0., Some(1), false));
        assert!(!timer.update(9.99, Some(1), false));
        assert!(timer.update(10., Some(1), false));
        assert!(!timer.update(11., None, false));
        assert!(!timer.update(20., Some(1), false));
        assert!(!timer.update(29., Some(2), false));
        assert!(!timer.update(38.99, Some(2), false));
        assert!(timer.update(39., Some(2), false));
    }
    #[test]
    fn dismissal_interaction_and_invalidation_do_not_reopen_stationary_help() {
        let mut timer = HoverTimer::default();
        timer.update(0., Some(1), false);
        assert!(timer.update(10., Some(1), false));
        timer.dismiss();
        assert!(!timer.update(100., Some(1), false));
        timer.update(101., None, false);
        timer.update(102., Some(1), false);
        assert!(timer.update(112., Some(1), false));
        assert!(!timer.update(113., Some(1), true));
        assert!(!timer.update(200., Some(1), false));
        timer.reset();
        assert!(!timer.update(201., Some(1), false));
    }
}
