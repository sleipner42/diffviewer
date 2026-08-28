mod application;
mod domain;
mod infra;
mod ui;

use std::path::PathBuf;

use application::repository::RangeSpec;
use infra::git_cli::GitCli;
use infra::single_instance::{self, Launch, OpenRequest};
use infra::updates::CURRENT_VERSION;
use ui::app::DiffViewerApp;

fn main() -> eframe::Result {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    for argument in &arguments {
        match argument.as_str() {
            "--version" | "-V" => {
                println!("diffviewer {CURRENT_VERSION}");
                return Ok(());
            }
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(());
            }
            _ => {}
        }
    }
    let (directory, range) = match split_arguments(&arguments) {
        Ok(split) => split,
        Err(error) => {
            eprintln!("diffviewer: {error}");
            std::process::exit(1);
        }
    };
    let requested = directory.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let start_dir = std::fs::canonicalize(&requested).unwrap_or(requested);
    let request = OpenRequest { dir: start_dir, range };
    // Validate the range here, in the process the user typed into, so a typo is a
    // terminal error rather than a toast in a window that may be on another desktop.
    if let Some(spec) = &request.range {
        if let Err(error) = GitCli::discover_range(&request.dir, spec.clone()) {
            eprintln!("diffviewer: {error:#}");
            std::process::exit(1);
        }
    }
    let listener = match single_instance::claim_or_forward(&request) {
        Ok(Launch::Forwarded) => return Ok(()),
        Ok(Launch::Primary(listener)) => Some(listener),
        Err(error) => {
            eprintln!("diffviewer: {error:#}");
            None
        }
    };
    let config = infra::config::load();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_app_id(APP_NAME)
            .with_inner_size([1500.0, 950.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| Ok(Box::new(DiffViewerApp::new(cc, config, request, listener)))),
    )
}

/// Splits `dw [directory] [base..head]`. An argument is a range only when it
/// looks like one *and* cannot be meant as a path, so `dw ../sibling` opens a
/// directory even though it contains dots. Anything unusable is an error rather
/// than being dropped, which would silently open the wrong kind of review.
fn split_arguments(arguments: &[String]) -> Result<(Option<PathBuf>, Option<RangeSpec>), String> {
    let mut directory: Option<PathBuf> = None;
    let mut range: Option<RangeSpec> = None;
    let set_directory = |value: &str, directory: &mut Option<PathBuf>| {
        if directory.is_some() {
            return Err("more than one directory given".to_string());
        }
        *directory = Some(PathBuf::from(value));
        Ok(())
    };
    for argument in arguments {
        if argument.starts_with('-') {
            return Err(format!("unknown option {argument}"));
        }
        if PathBuf::from(argument).exists() || looks_like_path(argument) {
            set_directory(argument, &mut directory)?;
            continue;
        }
        if argument.contains("..") {
            if range.is_some() {
                return Err("more than one range given".to_string());
            }
            range = Some(RangeSpec::parse(argument).ok_or_else(|| format!("{argument} is not a valid range"))?);
            continue;
        }
        set_directory(argument, &mut directory)?;
    }
    Ok((directory, range))
}

/// Written as a path by a human, whether or not it exists — so a typo in a
/// relative directory is reported as a missing directory, not a bad revision.
fn looks_like_path(argument: &str) -> bool {
    argument.starts_with('/') || argument.starts_with("./") || argument.starts_with("../") || argument.starts_with('~')
}

const APP_NAME: &str = "diffviewer";
const USAGE: &str = "usage: diffviewer [directory] [base..head]

Opens a git diff review window for the repository containing <directory>
(default: the current directory). If diffviewer is already running, the
directory is opened as a new tab in the existing window.

Given a range, reviews the commits on <head> since it forked from <base>
instead of the working tree. The review is read-only: no staging, no
discarding, no committing. Both base..head and base...head mean the same
thing — the diff from the merge base — and head defaults to HEAD.

  diffviewer main..            review your branch against main
  diffviewer main..feature     review feature against main
  diffviewer ~/code/app main.. review another repo's branch

  -V, --version   print the version
  -h, --help      show this help";
