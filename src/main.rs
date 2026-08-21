mod application;
mod domain;
mod infra;
mod ui;

use std::path::PathBuf;

use ui::app::DiffViewerApp;

fn main() -> eframe::Result {
    let start_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
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
        Box::new(move |cc| Ok(Box::new(DiffViewerApp::new(cc, config, start_dir)))),
    )
}

const APP_NAME: &str = "diffviewer";
