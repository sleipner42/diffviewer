use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::{Receiver, channel};
use std::thread;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseInfo {
    pub version: String,
    pub url: String,
}

pub fn spawn_check(wake: Box<dyn Fn() + Send>) -> Receiver<Result<Option<ReleaseInfo>, String>> {
    let (tx, rx) = channel();
    thread::spawn(move || {
        let result = fetch_latest()
            .map(|release| is_newer(&release.version, CURRENT_VERSION).then_some(release))
            .map_err(|error| format!("{error:#}"));
        let _ = tx.send(result);
        wake();
    });
    rx
}

pub fn spawn_install(wake: Box<dyn Fn() + Send>) -> Receiver<Result<(), String>> {
    let (tx, rx) = channel();
    thread::spawn(move || {
        let result = run_installer().map_err(|error| format!("{error:#}"));
        let _ = tx.send(result);
        wake();
    });
    rx
}

pub fn managed_by_homebrew() -> bool {
    std::env::current_exe()
        .map(|exe| exe.to_string_lossy().contains("/Cellar/"))
        .unwrap_or(false)
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

fn fetch_latest() -> Result<ReleaseInfo> {
    let output = Command::new("curl")
        .args(["-fsSL", "-H", "Accept: application/vnd.github+json", "-H", USER_AGENT_HEADER, LATEST_RELEASE_API])
        .output()
        .context("running curl")?;
    if !output.status.success() {
        bail!("GitHub release lookup failed: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    let release: ReleasePayload = serde_json::from_slice(&output.stdout).context("parsing GitHub release")?;
    Ok(ReleaseInfo {
        version: release.tag_name.trim_start_matches('v').to_string(),
        url: release.html_url,
    })
}

fn run_installer() -> Result<()> {
    let install_dir = install_dir();
    let mut command = Command::new("sh");
    command.args(["-c", INSTALL_COMMAND]);
    if let Some(dir) = install_dir {
        command.env("DIFFVIEWER_INSTALL_DIR", dir);
    }
    let output = command.output().context("running installer")?;
    if !output.status.success() {
        bail!("installer failed: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(())
}

fn install_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(|dir| dir.to_path_buf())
}

fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.trim().trim_start_matches('v').split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.split('-').next()?.parse().ok()?;
    Some((major, minor, patch))
}

#[derive(Debug, Deserialize)]
struct ReleasePayload {
    tag_name: String,
    html_url: String,
}

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const LATEST_RELEASE_API: &str = "https://api.github.com/repos/sleipner42/diffviewer/releases/latest";
const INSTALL_COMMAND: &str = "curl -fsSL https://raw.githubusercontent.com/sleipner42/diffviewer/main/install.sh | sh";
const USER_AGENT_HEADER: &str = "User-Agent: diffviewer";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_semver_tags() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(!is_newer("nightly", "0.1.0"));
    }
}
