//! Core serial workbench. GUI widgets never own or read serial handles.
pub mod bridge;
pub mod config;
pub mod endpoint;
pub mod file_transfer;
pub mod presets;
pub mod raw_recording;
pub mod repeat;
pub mod send;
pub mod serial;
pub mod ssh_serial;
pub mod traffic;
pub mod virtual_pair;

pub mod capture;
pub mod inspector;

pub mod send_history;

pub mod workspace;

pub mod terminal_display;

pub mod updater;

pub mod desktop;

pub mod terminal_selection;

pub mod traffic_analysis;

pub mod replay;
pub mod triggered_capture;

pub mod session;
