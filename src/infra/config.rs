use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;

use crate::domain::importance::{ImportancePolicy, Tier};

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub policy: ImportancePolicy,
    pub hide_tests: bool,
    pub poll_interval: Duration,
    pub path: PathBuf,
    pub load_error: Option<String>,
}

pub fn load() -> AppConfig {
    let path = config_path();
    let defaults = default_config();
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return AppConfig {
            policy: defaults.policy,
            hide_tests: defaults.hide_tests,
            poll_interval: defaults.poll_interval,
            path,
            load_error: None,
        };
    };
    match toml::from_str::<ConfigFile>(&raw) {
        Ok(file) => AppConfig {
            policy: file
                .tiers
                .map(|tiers| ImportancePolicy::new(tiers.into_iter().map(Tier::from).collect(), file.test_markers.clone().unwrap_or_else(default_test_markers)))
                .unwrap_or(defaults.policy),
            hide_tests: file.hide_tests.unwrap_or(defaults.hide_tests),
            poll_interval: file
                .poll_interval_ms
                .map(Duration::from_millis)
                .unwrap_or(defaults.poll_interval),
            path,
            load_error: None,
        },
        Err(error) => AppConfig {
            policy: defaults.policy,
            hide_tests: defaults.hide_tests,
            poll_interval: defaults.poll_interval,
            path: path.clone(),
            load_error: Some(format!("{}: {}", path.display(), error.message())),
        },
    }
}

#[derive(Debug, Deserialize)]
struct ConfigFile {
    tiers: Option<Vec<TierConfig>>,
    test_markers: Option<Vec<String>>,
    hide_tests: Option<bool>,
    poll_interval_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct TierConfig {
    name: String,
    markers: Vec<String>,
}

impl From<TierConfig> for Tier {
    fn from(config: TierConfig) -> Self {
        Tier {
            name: config.name,
            markers: config.markers,
        }
    }
}

struct Defaults {
    policy: ImportancePolicy,
    hide_tests: bool,
    poll_interval: Duration,
}

fn default_config() -> Defaults {
    Defaults {
        policy: ImportancePolicy::new(default_tiers(), default_test_markers()),
        hide_tests: false,
        poll_interval: Duration::from_millis(DEFAULT_POLL_INTERVAL_MS),
    }
}

fn default_tiers() -> Vec<Tier> {
    vec![
        tier("Domain", &["domain"]),
        tier("Use cases", &["use_cases", "usecases", "use-cases"]),
        tier("Application", &["application", "app", "api", "presentation"]),
        tier("Infrastructure", &["infrastructure", "infra", "adapters"]),
    ]
}

fn default_test_markers() -> Vec<String> {
    ["tests", "test", "__tests__", "spec", "specs"]
        .iter()
        .map(|marker| marker.to_string())
        .collect()
}

fn tier(name: &str, markers: &[&str]) -> Tier {
    Tier {
        name: name.to_string(),
        markers: markers.iter().map(|marker| marker.to_string()).collect(),
    }
}

fn config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(CONFIG_RELATIVE_PATH)
}

const CONFIG_RELATIVE_PATH: &str = ".config/diffviewer/config.toml";
const DEFAULT_POLL_INTERVAL_MS: u64 = 1000;
