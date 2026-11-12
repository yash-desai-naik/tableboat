//! Tableboat's core: everything that doesn't need a window. Shared by the desktop app
//! (`src/main.rs`) and the `tb` command line (`src/bin/tb.rs`).

pub mod claude;
pub mod conn;
pub mod db;
pub mod mcp;
mod mcp_app;
pub mod paths;
pub mod prefs;
