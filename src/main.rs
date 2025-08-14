use eframe::egui;

mod app;
mod disk_scanner;
mod ui;
mod utils;

use app::DiskAnalyzerApp;

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([800.0, 600.0])
            .with_decorations(true)
            .with_transparent(false),
        ..Default::default()
    };

    eframe::run_native(
        "Discope RC-0.1 | Disk Analyzer | v0.1 | @Author: Noncent",
        options,
        Box::new(|cc| {
            let fonts = egui::FontDefinitions::default();
            let font_size = 14.0;
            cc.egui_ctx.set_fonts(fonts);
            let mut style = (*cc.egui_ctx.style()).clone();
            style.text_styles.get_mut(&egui::TextStyle::Body).unwrap().size = font_size;
            style.text_styles.get_mut(&egui::TextStyle::Heading).unwrap().size = font_size * 1.5;
            style.text_styles.get_mut(&egui::TextStyle::Button).unwrap().size = font_size;
            cc.egui_ctx.set_style(style);
            Box::new(DiskAnalyzerApp::new())
        }),
    )
}
