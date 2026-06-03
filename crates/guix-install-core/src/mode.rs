use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum InstallMode {
    Guix,
    Nonguix,
    #[default]
    Panther,
    Enterprise {
        config_id: String,
        config_url: String,
    },
}

impl fmt::Display for InstallMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstallMode::Guix => write!(f, "guix"),
            InstallMode::Nonguix => write!(f, "nonguix"),
            InstallMode::Panther => write!(f, "panther"),
            InstallMode::Enterprise { config_id, .. } => {
                write!(f, "enterprise ({config_id})")
            }
        }
    }
}

impl InstallMode {
    pub fn label(&self) -> &str {
        match self {
            InstallMode::Guix => "guix",
            InstallMode::Nonguix => "nonguix",
            InstallMode::Panther => "panther",
            InstallMode::Enterprise { .. } => "enterprise",
        }
    }
}
