//! zniff: a desktop HTTP(S) capture app. The window is an nsh Rust view.
//! The proxy runs on its own tokio runtime in this process.

#![allow(clippy::too_many_arguments)]
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

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
mod har;
mod i18n;
mod icon;
#[cfg(not(target_arch = "wasm32"))]
mod mcp;
mod present;
#[cfg(not(target_arch = "wasm32"))]
mod proxy;
#[cfg(target_arch = "wasm32")]
#[path = "hub_web.rs"]
mod proxy;
#[cfg(not(target_arch = "wasm32"))]
mod relay;
mod rules;
mod session;
mod share;
#[cfg(not(target_arch = "wasm32"))]
mod sysproxy;
mod ui;
#[cfg(not(target_arch = "wasm32"))]
mod updater;
#[cfg(not(target_arch = "wasm32"))]
mod web;
mod ws;

#[cfg(not(target_arch = "wasm32"))]
pub use ca::clear_ios_simulator_proxies;
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
