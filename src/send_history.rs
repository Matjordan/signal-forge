//! Per-terminal command recall, including the settings needed to resend bytes.
use crate::send::{Encoding,LineEnding};
use std::collections::VecDeque;
pub const HISTORY_LIMIT:usize=100;
#[derive(Debug,Clone,PartialEq,Eq)]
pub struct SendEntry { pub input:String,pub encoding:Encoding,pub escapes:bool,pub ending:LineEnding }
#[derive(Default)]
pub struct SendHistory { entries:VecDeque<SendEntry>,cursor:Option<usize>,draft:Option<SendEntry> }
impl SendHistory {
    /// Call only after the endpoint accepts a manual, preset, or repeat command.
    /// Consecutive identical commands occupy one slot; repeats occupy one slot.
    pub fn remember(&mut self,entry:SendEntry) {
        self.edited();
        if self.entries.back()!=Some(&entry) { self.entries.push_back(entry); }
        while self.entries.len()>HISTORY_LIMIT { self.entries.pop_front(); }
    }
    pub fn older(&mut self,current:SendEntry)->Option<SendEntry> {
        if self.entries.is_empty() { return None; }
        let index=match self.cursor {
            None=>{ self.draft=Some(current); self.entries.len()-1 },
            Some(0)=>self.entries.len()-1,
            Some(index)=>index-1,
        };
        self.cursor=Some(index); self.entries.get(index).cloned()
    }
    pub fn newer(&mut self)->Option<SendEntry> {
        let index=self.cursor?;
        if index+1<self.entries.len() { self.cursor=Some(index+1); self.entries.get(index+1).cloned() }
        else { self.cursor=None; self.draft.take() }
    }
    pub fn edited(&mut self) { self.cursor=None; self.draft=None; }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn entry(input:&str)->SendEntry { SendEntry { input:input.into(),encoding:Encoding::Text,escapes:true,ending:LineEnding::None } }
    #[test]
    fn up_cycles_newest_first_and_down_restores_the_original_draft() {
        let mut history=SendHistory::default();
        assert_eq!(history.older(entry("draft")),None);
        history.remember(entry("first")); history.remember(entry("second"));
        assert_eq!(history.older(entry("draft")),Some(entry("second")));
        assert_eq!(history.older(entry("second")),Some(entry("first")));
        assert_eq!(history.older(entry("first")),Some(entry("second")));
        assert_eq!(history.newer(),Some(entry("draft"))); assert_eq!(history.newer(),None);
    }
    #[test]
    fn recall_restores_encoding_and_line_endings_and_editing_starts_new_navigation() {
        let mut history=SendHistory::default(); let binary=SendEntry { input:"00 FF".into(),encoding:Encoding::Hex,escapes:false,ending:LineEnding::CrLf };
        history.remember(binary.clone()); history.remember(entry("text"));
        history.older(entry("draft")); assert_eq!(history.older(entry("text")),Some(binary));
        history.edited(); assert_eq!(history.older(entry("edited")),Some(entry("text"))); assert_eq!(history.newer(),Some(entry("edited")));
    }
    #[test]
    fn histories_are_independent_bounded_and_deduplicate_consecutive_commands() {
        let mut a=SendHistory::default(); let mut b=SendHistory::default();
        for i in 0..150 { a.remember(entry(&i.to_string())); }
        a.remember(entry("149")); assert_eq!(a.entries.len(),HISTORY_LIMIT);
        b.remember(entry("other")); assert_eq!(b.older(entry("draft")),Some(entry("other")));
        assert_eq!(a.older(entry("draft")),Some(entry("149")));
        for _ in 1..HISTORY_LIMIT { a.older(entry("unused")); }
        assert_eq!(a.entries[a.cursor.unwrap()],entry("50"));
    }
}
