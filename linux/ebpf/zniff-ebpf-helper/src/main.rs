#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

mod bridge;
#[cfg(target_os = "linux")]
mod ebpf_loader;
mod proc_scan;
mod protocol;

use std::io::{self, BufRead, Write};

fn emit_event(evt: &protocol::HelperEvent) {
    if let Ok(line) = serde_json::to_string(evt) {
        let mut out = io::stdout().lock();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }
}

#[cfg(target_os = "linux")]
#[tokio::main]
async fn main() {
    use std::sync::{Arc, Mutex, RwLock};
    use std::time::Duration;
    use tokio::sync::mpsc;

    let ctrl = match ebpf_loader::EbpfController::load_and_attach() {
        Ok(c) => c,
        Err(err) => {
            emit_event(&protocol::HelperEvent::Error {
                message: format!("{err:#}"),
            });
            std::process::exit(1);
        }
    };
    let ctrl = Arc::new(Mutex::new(ctrl));
    let runtime_cfg = Arc::new(RwLock::new(bridge::BridgeRuntimeConfig::default()));

    let lookup_ctrl = ctrl.clone();
    let lookup: bridge::LookupFn = Arc::new(move |family, src_port| {
        lookup_ctrl
            .lock()
            .ok()?
            .lookup_and_remove_src_port(family, src_port)
    });

    let label_ctrl = ctrl.clone();
    let labeler: bridge::LabelFn = Arc::new(move |pid, comm| {
        label_ctrl
            .lock()
            .map(|c| c.resolve_label(pid, comm))
            .unwrap_or_default()
    });

    let bridge = match bridge::BridgeServer::start(runtime_cfg.clone(), lookup, labeler).await {
        Ok(b) => b,
        Err(err) => {
            emit_event(&protocol::HelperEvent::Error {
                message: format!("failed to bind local bridge: {err:#}"),
            });
            std::process::exit(1);
        }
    };
    let port_v4 = bridge.port_v4;
    let port_v6 = bridge.port_v6;

    emit_event(&protocol::HelperEvent::Ready {
        bridge_port_v4: port_v4,
        bridge_port_v6: port_v6,
    });

    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<Option<protocol::HelperCommand>>();
    std::thread::spawn(move || {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            match line {
                Ok(raw) => {
                    let trimmed = raw.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<protocol::HelperCommand>(trimmed) {
                        Ok(cmd) => {
                            let is_stop = matches!(cmd, protocol::HelperCommand::Stop);
                            let _ = cmd_tx.send(Some(cmd));
                            if is_stop {
                                break;
                            }
                        }
                        Err(err) => {
                            emit_event(&protocol::HelperEvent::Error {
                                message: format!("invalid helper command: {err}"),
                            });
                        }
                    }
                }
                Err(_) => break,
            }
        }
        let _ = cmd_tx.send(None);
    });

    let mut active_cfg: Option<protocol::HelperConfig> = None;
    let mut tick = tokio::time::interval(Duration::from_secs(5));

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = tick.tick() => {
                if let Some(ref cfg) = active_cfg {
                    if let Ok(mut guard) = ctrl.lock() {
                        let _ = guard.refresh_proc_pids(cfg);
                    }
                }
            }
            msg = cmd_rx.recv() => {
                match msg {
                    Some(Some(protocol::HelperCommand::Update(cfg))) => {
                        {
                            let mut r = runtime_cfg.write().unwrap_or_else(|e| e.into_inner());
                            r.proxy_port = cfg.proxy_port;
                            r.proxy_auth = cfg.proxy_auth.clone();
                        }
                        match ctrl.lock() {
                            Ok(mut guard) => {
                                if let Err(err) = guard.apply_config(&cfg, port_v4, port_v6) {
                                    emit_event(&protocol::HelperEvent::Error {
                                        message: format!("{err:#}"),
                                    });
                                } else {
                                    active_cfg = Some(cfg);
                                    emit_event(&protocol::HelperEvent::Updated);
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    Some(Some(protocol::HelperCommand::Ping)) => {
                        emit_event(&protocol::HelperEvent::Pong);
                    }
                    Some(Some(protocol::HelperCommand::Stop)) | Some(None) | None => break,
                }
            }
        }
    }

    bridge.stop();
    if let Ok(mut guard) = ctrl.lock() {
        guard.disable();
    }
    drop(ctrl);
}

#[cfg(not(target_os = "linux"))]
fn main() {
    let _ = (bridge::format_orig_authority, proc_scan::load_desktop_map);
    emit_event(&protocol::HelperEvent::Error {
        message: "zniff-ebpf-helper only runs on Linux".into(),
    });
    let stdin = io::stdin();
    let _ = stdin.lock().lines().next();
    std::process::exit(1);
}
