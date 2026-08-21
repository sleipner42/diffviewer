mod application;
mod domain;
mod infra;
mod ui;

use std::path::PathBuf;

use infra::single_instance::{self, Launch};
use infra::updates::CURRENT_VERSION;
use ui::app::DiffViewerApp;

fn main() -> eframe::Result {
    let argument = std::env::args().nth(1);
    match argument.as_deref() {
        Some("--version") | Some("-V") => {
            println!("diffviewer {CURRENT_VERSION}");
            return Ok(());
        }
        Some("--help") | Some("-h") => {
            println!("{USAGE}");
            return Ok(());
        }
        _ => {}
    }
    let requested = argument
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let start_dir = std::fs::canonicalize(&requested).unwrap_or(requested);
    let listener = match single_instance::claim_or_forward(&start_dir) {
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
        Box::new(move |cc| Ok(Box::new(DiffViewerApp::new(cc, config, start_dir, listener)))),
    )
}

const APP_NAME: &str = "diffviewer";
const USAGE: &str = "usage: diffviewer [directory]

Opens a git diff review window for the repository containing <directory>
(default: the current directory). If diffviewer is already running, the
directory is opened as a new tab in the existing window.

  -V, --version   print the version
  -h, --help      show this help";
