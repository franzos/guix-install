use std::fmt;

use serde::{Deserialize, Serialize};

pub const CI_GUIX_URL: &str = "https://ci.guix.gnu.org";
pub const NONGUIX_URL: &str = "https://substitutes.nonguix.org";
pub const GOFRANZ_URL: &str = "https://substitutes.guix.gofranz.com";

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

    pub fn substitute_urls(&self) -> Vec<String> {
        let mut urls = vec![CI_GUIX_URL.to_string()];
        match self {
            InstallMode::Guix => {}
            InstallMode::Nonguix => urls.push(NONGUIX_URL.into()),
            InstallMode::Panther | InstallMode::Enterprise { .. } => {
                urls.push(NONGUIX_URL.into());
                urls.push(GOFRANZ_URL.into());
            }
        }
        urls
    }
}
