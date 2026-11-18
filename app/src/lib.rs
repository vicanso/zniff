//! zniff: a desktop HTTP(S) capture app. The window is an nsh Rust view.
//! The proxy runs on its own tokio runtime in this process.

#![allow(clippy::too_many_arguments)]
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

#[cfg(not(target_arch = "wasm32"))]
mod bypass;
#[cfg(not(target_arch = "wasm32"))]
mod ca;
#[cfg(target_arch = "wasm32")]
#[path = "ca_web.rs"]
mod ca;
#[cfg(not(target_arch = "wasm32"))]
mod client_hello;
mod clock;
mod config;
#[cfg(not(target_arch = "wasm32"))]
mod decompress;
#[cfg(not(target_arch = "wasm32"))]
mod har;
mod i18n;
mod icon;
#[cfg(not(target_arch = "wasm32"))]
#[cfg_attr(not(feature = "desktop"), allow(dead_code))]
mod launcher;
#[cfg(not(target_arch = "wasm32"))]
mod local_proc;
#[cfg(not(target_arch = "wasm32"))]
mod mcp;
#[cfg(not(target_arch = "wasm32"))]
#[cfg_attr(not(feature = "desktop"), allow(dead_code))]
mod mock_gen;
mod openapi;
mod present;
#[cfg(not(target_arch = "wasm32"))]
#[cfg_attr(not(feature = "desktop"), allow(dead_code))]
mod proto_schema;
#[cfg(not(target_arch = "wasm32"))]
mod proxy;
#[cfg(target_arch = "wasm32")]
#[path = "hub_web.rs"]
mod proxy;
mod redact;
#[cfg(not(target_arch = "wasm32"))]
mod relay;
mod rule_vars;
mod rules;
mod session;
#[cfg(not(target_arch = "wasm32"))]
mod share;
#[cfg(not(target_arch = "wasm32"))]
mod sysproxy;
mod ui;
#[cfg(not(target_arch = "wasm32"))]
mod updater;
#[cfg(not(target_arch = "wasm32"))]
mod web;
#[cfg(not(target_arch = "wasm32"))]
mod webhook;
#[cfg(not(target_arch = "wasm32"))]
mod ws;

#[cfg(not(target_arch = "wasm32"))]
pub use ca::clear_ios_simulator_proxies;
#[cfg(not(target_arch = "wasm32"))]
pub use launcher::cleanup_on_exit as cleanup_launchers;
use nsh_core::prelude::{Registry, native_module};
#[cfg(not(target_arch = "wasm32"))]
pub use relay::shutdown_relay_push;
#[cfg(not(target_arch = "wasm32"))]
pub use sysproxy::restore_if_dirty;
#[cfg(not(target_arch = "wasm32"))]
pub use ui::maybe_run_cli_mode;
pub use ui::{App, initial_window_size};

/// The app's UI set (`nsh.json` `"rust"`). The web package calls this entry
/// even though `rust_app!` also sets the root.
#[native_module("zniff/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
}
