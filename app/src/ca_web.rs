//! Certificate status for the web view.
//!
//! The browser does not install a CA and does not read the system trust store.
//! The server process owns `ca.crt`.

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum CaStatus {
    Trusted,
    InstalledUntrusted,
    NotInstalled,
}

impl CaStatus {
    #[allow(dead_code)]
    pub fn is_installed(self) -> bool {
        !matches!(self, Self::NotInstalled)
    }
}

/// Placeholder. The web view never opens a certificate file.
#[derive(Clone)]
pub struct Ca;

#[allow(dead_code)]
impl Ca {
    pub fn unavailable() -> Self {
        Self
    }

    pub fn dir(&self) -> &Path {
        Path::new("")
    }

    pub fn trust_status(&self) -> CaStatus {
        CaStatus::NotInstalled
    }

    pub fn cert_path(&self) -> PathBuf {
        PathBuf::new()
    }
}

/// Not the user's data directory. The web view does not write config.
/// wasm32's `std::env::temp_dir` panics, so this is only a path string.
pub fn data_dir() -> PathBuf {
    PathBuf::from("zniff-web-view")
}
