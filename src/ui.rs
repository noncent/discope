use eframe::egui;
use crate::app::DiskAnalyzerApp;
use crate::disk_scanner::{DiskUsage, FolderInfo};
use rfd::FileDialog;
use std::path::PathBuf;

pub struct DiskAnalyzerUI;

impl DiskAnalyzerUI {
    pub fn render(app: &mut DiskAnalyzerApp, ctx: &egui::Context) {
        ctx.set_visuals(egui::Visuals::dark());

        // Header (top) with breadcrumbs, toggles, select button
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.add_space(16.0); // Top padding
            Self::render_header(ui, app);
            ui.add_space(16.0); // Bottom padding
        });

        // Left sidebar: dedicated side panel that takes full height
        egui::SidePanel::left("folders_panel")
            .resizable(true)
            .default_width(360.0)
            .min_width(280.0)
            .max_width(520.0)
            .show(ctx, |ui| {
                ui.add_space(16.0);
                if let Some(du) = app.get_disk_usage().cloned() {
                    ui.heading("Folders");
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(8.0);
                    egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                        for folder in &du.folders { Self::render_tree_row(ui, app, folder, 0, du.total_size); }
                    });
                } else {
                    ui.heading("Folders");
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(8.0);
                    ui.label("Select a directory to analyze");
                }
                ui.add_space(16.0);
            });

        // Main content: sunburst chart fills remaining space
        egui::CentralPanel::default().show(ctx, |ui| {
            if app.is_scanning() {
                Self::render_scanning_progress(ui, app);
            } else if let Some(du) = app.get_disk_usage().cloned() {
                Self::render_sunburst(ui, app, du);
            } else {
                let main_rect = ui.available_rect_before_wrap();
                let text_height = 60.0;
                let center_y = main_rect.center().y;
                let text_y = center_y - (text_height / 2.0);
                ui.add_space(text_y - main_rect.min.y);
                ui.horizontal(|ui| {
                    ui.add_space(main_rect.width() / 2.0 - 150.0);
                    ui.vertical(|ui| {
                        ui.heading("Select a directory to analyze");
                        ui.label("Click 'Select Directory' to get started");
                    });
                });
            }
        });
    }

    fn render_header(ui: &mut egui::Ui, app: &mut DiskAnalyzerApp) {
        // Navigation + breadcrumb
        ui.horizontal(|ui| {
            ui.add_space(16.0); // Left header margin
            
            let back_enabled = app.can_go_back();
            if ui.add_enabled(back_enabled, egui::Button::new("◀")).clicked() { app.go_back(); }
            
            let fwd_enabled = app.can_go_forward();
            if ui.add_enabled(fwd_enabled, egui::Button::new("▶")).clicked() { app.go_forward(); }
            
            ui.separator();
            
            if let Some(current_path) = app.get_current_path().cloned() {
                let mut acc = PathBuf::new();
                for (i, part) in current_path.components().enumerate() {
                    match part { 
                        std::path::Component::RootDir => acc.push(std::path::MAIN_SEPARATOR.to_string()), 
                        _ => acc.push(part.as_os_str()) 
                    }
                    if i > 0 { ui.label("›"); }
                    let display = acc.file_name().and_then(|s| s.to_str()).unwrap_or(acc.to_string_lossy().as_ref()).to_string();
                    let acc_clone = acc.clone(); 
                    if ui.link(display).clicked() { app.scan_directory(acc_clone); }
                }
            } else { 
                ui.label("Disks and Folders"); 
            }
            
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Enhanced Select Directory button with animation
                let button_response = ui.add(egui::Button::new("📁 Select Directory")
                    .min_size(egui::vec2(140.0, 32.0))
                    .fill(egui::Color32::from_rgb(60, 120, 200))
                    .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(80, 140, 220))));
                
                if button_response.clicked() { 
                    if let Some(path) = FileDialog::new().pick_folder() { 
                        app.scan_directory(path); 
                    } 
                }
            });
            
            ui.add_space(16.0); // Right header margin
        });
        
        ui.add_space(12.0); // Space between navigation and controls
        
        // Safety toggles row
        ui.horizontal(|ui| {
            ui.add_space(16.0); // Left margin
            
            let mut dry = app.dry_run(); 
            if ui.checkbox(&mut dry, "Dry run").changed() { app.set_dry_run(dry); }
            
            ui.add_space(16.0); // Space between controls
            
            let mut confirm = app.require_confirm(); 
            if ui.checkbox(&mut confirm, "Confirm before delete").changed() { app.set_require_confirm(confirm); }
            
            ui.add_space(16.0); // Space between controls
            
            // Depth slider bound to app state - update app state immediately when slider changes
            let mut depth: i32 = app.max_depth().try_into().unwrap();
            ui.label("Depth:");
            ui.add_space(8.0);
            if ui.add(egui::Slider::new(&mut depth, 2..=20).text("")).changed() {
                app.set_max_depth(depth as usize);
            }
            ui.add_space(8.0);
            ui.label(format!("{}", depth));
            
            ui.add_space(16.0); // Space between slider and button
            
            if ui.button("Apply Depth").clicked() {
                if let Some(path) = app.get_current_path().cloned() { 
                    app.scan_directory(path); 
                }
            }
            
            ui.add_space(16.0); // Right margin
        });
    }

    fn render_scanning_progress(ui: &mut egui::Ui, app: &DiskAnalyzerApp) {
        // Get the available area for proper centering
        let available_rect = ui.available_rect_before_wrap();
        let center = available_rect.center();
        
        // Center the entire progress section
        let progress_height = 200.0; // Approximate height of progress section
        let progress_y = center.y - (progress_height / 2.0);
        
        ui.add_space(progress_y - available_rect.min.y);
        
        ui.horizontal(|ui| {
            ui.add_space(available_rect.width() / 2.0 - 200.0); // Center horizontally
            
            ui.vertical(|ui| {
                ui.add_space(20.0);
                
                // Enhanced scanning animation
                let spinner_size = 48.0;
                let time = ui.input(|i| i.time) as f32;
                let rotation = (time * 2.0) % (2.0 * std::f32::consts::PI);
                
                // Draw spinning gear animation centered
                let painter = ui.painter();
                let spinner_center = center;
                
                // Draw spinning gear animation
                for i in 0..8 {
                    let angle = rotation + (i as f32 * std::f32::consts::PI / 4.0);
                    let start = spinner_center + egui::vec2(angle.cos(), angle.sin()) * (spinner_size * 0.3);
                    let end = spinner_center + egui::vec2(angle.cos(), angle.sin()) * (spinner_size * 0.5);
                    painter.line_segment(
                        [start, end],
                        egui::Stroke::new(3.0, egui::Color32::from_rgb(100, 150, 255))
                    );
                }
                
                ui.add_space(20.0);
                ui.heading("Scanning Directory...");
                ui.add_space(16.0);
                
                // Progress bar with better styling
                ui.add(egui::ProgressBar::new(app.get_scan_progress())
                    .show_percentage()
                    .animate(true)
                    .desired_width(300.0)
                    .desired_height(24.0));
                
                ui.add_space(20.0);
            });
        });
    }

    fn render_sunburst(ui: &mut egui::Ui, app: &mut DiskAnalyzerApp, du: DiskUsage) {
        // Claim all remaining space for the chart to render at full scale
        let desired_size = ui.available_size();
        let (rect, resp) = ui.allocate_exact_size(desired_size, egui::Sense::click());
        // Make the chart as large as possible while respecting padding in the central panel
        let side = rect.width().min(rect.height());
        let center = rect.center();
        // Use a larger fraction to make the sunburst visually dominant
        let max_radius = side * 0.495;
        let painter = ui.painter_at(rect);

        painter.circle_filled(center, max_radius + 18.0, egui::Color32::from_rgb(22, 25, 28));
        if du.total_size == 0 { return; }
        let total = du.total_size as f64;

        #[derive(Clone)]
        struct Sector { a0: f32, a1: f32, r0: f32, r1: f32, path: PathBuf, size: u64, percent: f64 }
        let mut sectors: Vec<Sector> = Vec::new();

        let ring_thickness = max_radius * 0.32; let gap = ring_thickness * 0.04;
        let mut a0 = -std::f32::consts::FRAC_PI_2;
        for (i, f) in du.folders.iter().enumerate() {
            if f.size == 0 { continue; }
            let frac = (f.size as f64 / total) as f32; let sweep = frac * std::f32::consts::TAU;
            let outer = max_radius; let inner = max_radius - ring_thickness;
            let color = Self::dynamic_color(i, 0); Self::ring_sector(&painter, center, outer, inner, a0, a0 + sweep, color);
            sectors.push(Sector { a0, a1: a0 + sweep, r0: inner, r1: outer, path: f.path.clone(), size: f.size, percent: (f.size as f64 / total) * 100.0 });
            if !f.children.is_empty() {
                let mut cur = a0;
                for (j, ch) in f.children.iter().enumerate() {
                    if ch.size == 0 { continue; }
                    let ch_frac = (ch.size as f64 / f.size as f64) as f32; let ch_sweep = sweep * ch_frac;
                    let outer2 = inner - gap; let inner2 = outer2 - ring_thickness * 0.9;
                    let color2 = Self::dynamic_color(j, 1); Self::ring_sector(&painter, center, outer2, inner2, cur, cur + ch_sweep, color2);
                    sectors.push(Sector { a0: cur, a1: cur + ch_sweep, r0: inner2, r1: outer2, path: ch.path.clone(), size: ch.size, percent: (ch.size as f64 / total) * 100.0 });
                    cur += ch_sweep;
                }
            }
            a0 += sweep;
        }

        painter.circle_filled(center, max_radius - ring_thickness - gap - ring_thickness * 0.6, egui::Color32::from_rgb(24, 26, 30));
        painter.text(center, egui::Align2::CENTER_CENTER, du.total_human_size.clone(), egui::FontId::proportional(18.0), egui::Color32::WHITE);

        if let Some(pos) = resp.hover_pos() {
            let v = pos - center; let r = v.length(); let ang = v.y.atan2(v.x);
            for s in &sectors {
                if r >= s.r0 && r <= s.r1 && ang >= s.a0 && ang <= s.a1 {
                    egui::show_tooltip_at_pointer(ui.ctx(), ui.id().with("tip"), |ui: &mut egui::Ui| {
                        ui.label(s.path.file_name().and_then(|n| n.to_str()).unwrap_or("(root)"));
                        ui.label(format!("{} ({:.1}%)", humansize::format_size(s.size, humansize::DECIMAL), s.percent));
                    });
                    if resp.clicked() { app.scan_directory(s.path.clone()); }
                    break;
                }
            }
        }
    }

    fn ring_sector(p: &egui::Painter, c: egui::Pos2, r_outer: f32, r_inner: f32, a0: f32, a1: f32, color: egui::Color32) {
        let steps = 64.max((((a1 - a0).abs()) * 40.0) as i32) as usize;
        let mut pts: Vec<egui::Pos2> = Vec::with_capacity(steps * 2 + 2);
        for i in 0..=steps { let t = i as f32 / steps as f32; let a = egui::lerp(a0..=a1, t); pts.push(c + egui::vec2(a.cos(), a.sin()) * r_outer); }
        for i in (0..=steps).rev() { let t = i as f32 / steps as f32; let a = egui::lerp(a0..=a1, t); pts.push(c + egui::vec2(a.cos(), a.sin()) * r_inner); }
        p.add(egui::Shape::convex_polygon(pts, color, egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(20,20,20,180))));
    }

    fn render_tree_row(ui: &mut egui::Ui, app: &mut DiskAnalyzerApp, folder: &FolderInfo, depth: usize, total_size: u64) {
        let indent = 14.0 * depth as f32;
        ui.horizontal(|ui| {
            ui.add_space(indent);
            let expanded = app.is_expanded(&folder.path);
            let is_leaf = folder.children.is_empty();
            if is_leaf { ui.label("  "); } else { let label = if expanded { "−" } else { "+" }; if ui.button(label).clicked() { app.toggle_expanded(&folder.path); } }
            let label = folder.path.file_name().unwrap_or_default().to_string_lossy();
            let row = ui.selectable_label(app.get_selected_folder().map(|p| p == &folder.path).unwrap_or(false), label);
            if row.double_clicked() { app.scan_directory(folder.path.clone()); }
            else if row.clicked() { app.set_selected_folder(Some(folder.path.clone())); }
            if row.hovered() {
                let pct = if total_size > 0 { (folder.size as f64 / total_size as f64) * 100.0 } else { 0.0 };
                egui::show_tooltip_at_pointer(ui.ctx(), ui.id().with("rowtip"), |ui: &mut egui::Ui| {
                    ui.label(folder.path.to_string_lossy());
                    ui.label(format!("{} ({:.1}%)", humansize::format_size(folder.size, humansize::DECIMAL), pct));
                });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("🗑️").on_hover_text("Move to Trash").clicked() {
                    let mut do_delete = true;
                    if app.require_confirm() {
                        let mut confirmed = false;
                        egui::Window::new("Confirm delete").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0,0.0)).show(ui.ctx(), |ui| {
                            ui.label(format!("Move to Trash: {}?", folder.path.display()));
                            if ui.button("Yes").clicked() { confirmed = true; }
                            if ui.button("No").clicked() { confirmed = false; }
                        });
                        do_delete = confirmed;
                    }
                    if do_delete { if let Err(e) = app.move_folder_to_trash(&folder.path) { eprintln!("Error trashing folder: {}", e); } }
                }
                ui.label(humansize::format_size(folder.size, humansize::DECIMAL));
            });
        });
        if app.is_expanded(&folder.path) { for child in &folder.children { Self::render_tree_row(ui, app, child, depth + 1, total_size); } }
    }

    fn dynamic_color(i: usize, level: usize) -> egui::Color32 {
        let base_hues = [40.0, 140.0, 210.0, 280.0, 330.0, 20.0, 90.0, 160.0, 200.0, 250.0, 300.0, 0.0];
        let hue = (base_hues[i % base_hues.len()] + (level as f32) * 10.0) % 360.0;
        let sat = (0.70 - 0.08 * (level as f32)).clamp(0.45, 0.9);
        let val = 0.92; let (r,g,b) = Self::hsv_to_rgb(hue, sat, val); egui::Color32::from_rgb(r, g, b)
    }

    fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
        let c = v * s; let hh = h / 60.0; let x = c * (1.0 - ((hh % 2.0) - 1.0).abs());
        let (r1,g1,b1) = if (0.0..1.0).contains(&hh) { (c,x,0.0) }
        else if (1.0..2.0).contains(&hh) { (x,c,0.0) }
        else if (2.0..3.0).contains(&hh) { (0.0,c,x) }
        else if (3.0..4.0).contains(&hh) { (0.0,x,c) }
        else if (4.0..5.0).contains(&hh) { (x,0.0,c) }
        else { (c,0.0,x) };
        let m = v - c; let (r,g,b) = ((r1+m)*255.0, (g1+m)*255.0, (b1+m)*255.0); (r as u8, g as u8, b as u8)
    }
}
