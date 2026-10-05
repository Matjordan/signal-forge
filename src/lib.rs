//! Core serial workbench. GUI widgets never own or read serial handles.
pub mod bridge;
pub mod config;
pub mod endpoint;
pub mod presets;
pub mod repeat;
pub mod send;
pub mod serial;
pub mod traffic;
pub mod virtual_pair;

pub mod capture;
pub mod inspector;

pub mod send_history;

pub mod workspace;
