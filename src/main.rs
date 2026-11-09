use eframe::egui;
use std::path::{Path, PathBuf};

mod app;
mod chart;
mod disk_scanner;
mod export;
mod ui;

use app::DiskAnalyzerApp;

fn main() -> Result<(), eframe::Error> {
    let mut args = std::env::args_os().skip(1);
    let first = args.next();
    if first.as_deref() == Some(std::ffi::OsStr::new("--scan")) {
        let Some(path) = args.next() else {
            eprintln!("Usage: disk-analyzer --scan <folder>");
            std::process::exit(2);
        };
        match disk_scanner::scan(Path::new(&path), &disk_scanner::ScanControl::default()) {
            Ok(usage) => {
                println!(
                "{} bytes · {} files · {} folders · {} unreadable · {} separate volumes · {:.3} s",
                usage.size(), usage.files, usage.folders, usage.unreadable, usage.skipped_volumes,
                usage.elapsed.as_secs_f64(),
            )
            }
            Err(error) => {
                eprintln!("Scan failed: {error}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    let initial = first.map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([800.0, 560.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };

    eframe::run_native(
        "Discope",
        options,
        Box::new(move |cc| Box::new(DiskAnalyzerApp::new(&cc.egui_ctx, initial))),
    )
}
