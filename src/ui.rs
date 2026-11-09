use crate::app::{Collected, DiskAnalyzerApp, Page};
use crate::chart::{self, Target};
use crate::disk_scanner::{display_name, DiskUsage, Kind, NodeId};
use crate::export::ExportFormat;
use eframe::egui::{self, Align, Align2, Color32, FontId, Pos2, Rect, RichText, Stroke, Vec2};
use humansize::{format_size, DECIMAL};
use std::sync::atomic::Ordering::Relaxed;
use std::time::Duration;

#[derive(Clone, Copy)]
struct Palette {
    background: Color32,
    surface: Color32,
    hover: Color32,
    line: Color32,
    text: Color32,
    muted: Color32,
    blue: Color32,
    accent: Color32,
    warning: Color32,
    red: Color32,
}

impl Palette {
    fn for_mode(dark: bool) -> Self {
        let rgb = Color32::from_rgb;
        if dark {
            Self {
                background: rgb(20, 25, 37),
                surface: rgb(28, 35, 50),
                hover: rgb(42, 55, 77),
                line: rgb(58, 70, 92),
                text: rgb(235, 241, 253),
                muted: rgb(163, 177, 200),
                blue: rgb(54, 91, 186),
                accent: rgb(123, 167, 255),
                warning: rgb(249, 193, 123),
                red: rgb(165, 48, 70),
            }
        } else {
            Self {
                background: rgb(243, 246, 251),
                surface: Color32::WHITE,
                hover: rgb(231, 239, 255),
                line: rgb(209, 218, 233),
                text: rgb(24, 38, 64),
                muted: rgb(78, 96, 126),
                blue: rgb(35, 77, 180),
                accent: rgb(35, 77, 180),
                warning: rgb(146, 80, 16),
                red: rgb(167, 41, 65),
            }
        }
    }

    fn get(ctx: &egui::Context) -> Self {
        Self::for_mode(ctx.style().visuals.dark_mode)
    }
}

pub fn install_style(ctx: &egui::Context, dark: bool) {
    let colors = Palette::for_mode(dark);
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.panel_fill = colors.background;
    visuals.window_fill = colors.surface;
    visuals.extreme_bg_color = colors.surface;
    visuals.override_text_color = Some(colors.text);
    visuals.selection.bg_fill = colors.hover;
    visuals.selection.stroke = Stroke::new(1.0_f32, colors.accent);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, colors.line);
    for (widget, fill) in [
        (&mut visuals.widgets.inactive, colors.surface),
        (&mut visuals.widgets.hovered, colors.hover),
        (&mut visuals.widgets.active, colors.hover),
    ] {
        widget.bg_stroke = Stroke::new(1.0_f32, colors.line);
        widget.rounding = egui::Rounding::same(6.0);
        widget.weak_bg_fill = fill;
        widget.bg_fill = fill;
        widget.fg_stroke.color = colors.text;
    }
    ctx.set_visuals(visuals);
    ctx.style_mut(|style| {
        style.spacing.button_padding = egui::vec2(10.0, 4.0);
        style.spacing.item_spacing = egui::vec2(7.0, 6.0);
        style.spacing.interact_size.y = 25.0;
    });
}

fn filled_button(label: &str, fill: Color32) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).color(Color32::WHITE).strong())
        .fill(fill)
        .stroke(Stroke::NONE)
}

#[derive(Default)]
enum Action {
    #[default]
    None,
    Up,
    Open(Target),
    Reveal(NodeId),
    ToggleMark(NodeId),
    Export(ExportFormat),
}

#[derive(Clone, Copy)]
struct DragItem {
    generation: u64,
    id: NodeId,
}

pub fn render(app: &mut DiskAnalyzerApp, ctx: &egui::Context) {
    let dropped = ctx.input(|input| {
        input
            .raw
            .dropped_files
            .iter()
            .find_map(|file| file.path.clone())
    });
    if let Some(path) = dropped {
        if !app.busy() && !app.confirm_open {
            app.start_scan(path, ctx);
        }
    }

    let colors = Palette::get(ctx);
    if !app.busy() && !app.confirm_open {
        let shortcuts = if cfg!(target_os = "macos") {
            egui::Modifiers::COMMAND
        } else {
            egui::Modifiers::ALT
        };
        let (back, forward) = ctx.input_mut(|input| {
            let (left, right) = if cfg!(target_os = "macos") {
                (egui::Key::OpenBracket, egui::Key::CloseBracket)
            } else {
                (egui::Key::ArrowLeft, egui::Key::ArrowRight)
            };
            (
                input.consume_key(shortcuts, left),
                input.consume_key(shortcuts, right),
            )
        });
        if back {
            app.back();
        }
        if forward {
            app.forward();
        }
    }

    toolbar(app, ctx);
    footer(app, ctx);
    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(colors.background))
        .show(ctx, |ui| {
            ui.set_enabled(!app.confirm_open);
            if app.busy() {
                progress(app, ui);
            } else {
                match app.page() {
                    Page::Home => home(app, ui),
                    Page::Disks => disks(app, ui),
                    Page::Map { .. } => workspace(app, ui),
                }
            }
        });
    collector_window(app, ctx);
    confirmation(app, ctx);
}

fn toolbar(app: &mut DiskAnalyzerApp, ctx: &egui::Context) {
    let colors = Palette::get(ctx);
    egui::TopBottomPanel::top("toolbar")
        .frame(
            egui::Frame::none()
                .fill(colors.surface)
                .inner_margin(egui::Margin::symmetric(19.0, 11.0)),
        )
        .show(ctx, |ui| {
            ui.set_enabled(!app.confirm_open);
            let width = ui.available_width();
            ui.horizontal(|ui| {
                let (logo, _) =
                    ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
                draw_icon(ui.painter(), logo.center(), Icon::Brand, colors.accent);
                ui.label(RichText::new("Discope").size(16.0).strong());
                ui.add_space(8.0);
                if ui
                    .add_enabled(!app.busy() && app.can_back(), egui::Button::new("‹ Back"))
                    .clicked()
                {
                    app.back();
                }
                if ui
                    .add_enabled(
                        !app.busy() && app.can_forward(),
                        egui::Button::new("Forward ›"),
                    )
                    .clicked()
                {
                    app.forward();
                }
                if ui
                    .add_enabled(
                        !app.busy(),
                        egui::Button::new("Home").selected(app.page() == Page::Home),
                    )
                    .clicked()
                {
                    app.navigate(Page::Home);
                }
                if app.showing_map() && !app.busy() {
                    if let Some(id) = breadcrumbs(ui, app, width - 540.0) {
                        app.go_to(id);
                    }
                } else if app.page() == Page::Disks {
                    ui.label(RichText::new("Disks").color(colors.muted));
                }
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .button(if app.dark { "Light mode" } else { "Dark mode" })
                        .clicked()
                    {
                        app.set_dark(!app.dark, ctx);
                    }
                    if app.showing_map() {
                        ui.add_enabled_ui(!app.busy(), |ui| {
                            ui.menu_button("Options", |ui| {
                                ui.add(
                                    egui::Slider::new(&mut app.depth, 2..=12).text("Chart rings"),
                                );
                            });
                        });
                        if ui
                            .add_enabled(!app.busy(), egui::Button::new("Rescan"))
                            .clicked()
                        {
                            if let Some(snapshot) = &app.snapshot {
                                app.start_scan(snapshot.root.clone(), ctx);
                            }
                        }
                    }
                });
            });
        });
}

fn breadcrumbs(ui: &mut egui::Ui, app: &DiskAnalyzerApp, budget: f32) -> Option<NodeId> {
    let colors = Palette::get(ui.ctx());
    let snapshot = app.snapshot.as_ref()?;
    let mut crumbs = Vec::new();
    let mut id = app.focus();
    loop {
        crumbs.push(id);
        if id == 0 {
            break;
        }
        id = snapshot.nodes[id].parent;
    }
    crumbs.reverse();
    let mut picked = None;
    let limit = if budget > 330.0 { 4 } else { 2 };
    if crumbs.len() > limit {
        if ui.button(short_name(&snapshot.nodes[0].name, 15)).clicked() {
            picked = Some(0);
        }
        ui.label(RichText::new("› … ›").color(colors.muted));
    }
    let visible = if crumbs.len() > limit {
        limit - 1
    } else {
        limit
    };
    for &id in &crumbs[crumbs.len().saturating_sub(visible)..] {
        let name = short_name(&snapshot.nodes[id].name, if limit == 2 { 15 } else { 19 });
        if ui
            .button(name)
            .on_hover_ui(|ui| {
                ui.label(display_name(snapshot.path(id).as_os_str()));
            })
            .clicked()
        {
            picked = Some(id);
        }
    }
    picked
}

fn pick_folder(app: &mut DiskAnalyzerApp, ctx: &egui::Context) {
    if let Some(path) = rfd::FileDialog::new()
        .set_title("Choose a folder or disk to scan")
        .pick_folder()
    {
        app.start_scan(path, ctx);
    }
}

#[derive(Clone, Copy)]
enum Icon {
    Brand,
    Disk,
    Folder,
    History,
    Save,
}

fn draw_icon(painter: &egui::Painter, center: Pos2, icon: Icon, color: Color32) {
    let stroke = Stroke::new(1.6_f32, color);
    match icon {
        Icon::Brand => {
            painter.circle_stroke(center, 9.0, stroke);
            painter.circle_stroke(center, 4.0, stroke);
            for angle in [0.25_f32, 2.35, 4.4] {
                let direction = Vec2::angled(angle);
                painter.line_segment([center + direction * 4.0, center + direction * 9.0], stroke);
            }
        }
        Icon::Disk => {
            painter.rect_stroke(
                Rect::from_center_size(center, egui::vec2(23.0, 17.0)),
                4.0,
                stroke,
            );
            painter.line_segment(
                [
                    center + egui::vec2(-8.0, 2.0),
                    center + egui::vec2(8.0, 2.0),
                ],
                stroke,
            );
            painter.circle_filled(center + egui::vec2(6.5, 5.0), 1.2, color);
        }
        Icon::Folder => {
            let points = [
                (-11., 7.),
                (-11., -7.),
                (-3., -7.),
                (0., -4.),
                (11., -4.),
                (11., 7.),
                (-11., 7.),
            ];
            painter.add(egui::Shape::line(
                points
                    .into_iter()
                    .map(|(x, y)| center + egui::vec2(x, y))
                    .collect(),
                stroke,
            ));
        }
        Icon::History => {
            painter.circle_stroke(center, 9.0, stroke);
            painter.line_segment([center, center - egui::vec2(0.0, 5.0)], stroke);
            painter.line_segment([center, center + egui::vec2(4.0, 2.0)], stroke);
        }
        Icon::Save => {
            let line = |points: &[(f32, f32)]| {
                painter.add(egui::Shape::line(
                    points
                        .iter()
                        .map(|&(x, y)| center + egui::vec2(x, y))
                        .collect(),
                    stroke,
                ));
            };
            line(&[(-6.0, 2.0), (-6.0, 6.0), (6.0, 6.0), (6.0, 2.0)]);
            line(&[(0.0, -6.0), (0.0, 2.0)]);
            line(&[(-3.0, -1.0), (0.0, 2.0), (3.0, -1.0)]);
        }
    }
}

fn home(app: &mut DiskAnalyzerApp, ui: &mut egui::Ui) {
    let colors = Palette::get(ui.ctx());
    let gutter = ((ui.available_width() - 920.0) * 0.5).max(28.0);
    egui::ScrollArea::vertical().id_source("home").auto_shrink([false; 2]).show(ui, |ui| {
        egui::Frame::none().inner_margin(egui::Margin::symmetric(gutter, 32.0)).show(ui, |ui| {
            ui.label(RichText::new("STORAGE ANALYZER").size(12.0).strong().color(colors.accent));
            ui.add_space(3.0);
            ui.label(RichText::new("See where your storage goes").size(35.0).strong());
            ui.label(RichText::new("Choose what to scan, then explore the files and folders taking up space.").size(15.0).color(colors.muted));
            ui.add_space(27.0);

            let width = (ui.available_width() - 13.0) * 0.5;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 13.0;
                if scan_card(ui, width, Icon::Disk, "Scan a disk", "Analyze an entire internal or external drive.", "Choose disk  ›") {
                    app.navigate(Page::Disks);
                    app.refresh_volumes(ui.ctx());
                }
                if scan_card(ui, width, Icon::Folder, "Scan a folder", "Inspect a folder and everything inside it.", "Choose folder  ›") {
                    pick_folder(app, ui.ctx());
                }
            });
            ui.add_space(30.0);
            ui.label(RichText::new("Last scan").size(21.0).strong());
            ui.add_space(5.0);
            if let Some(summary) = &app.last_scan {
                let title = app.volumes.iter().find(|volume| volume.path == summary.root)
                    .map(|volume| volume.name.clone())
                    .unwrap_or_else(|| display_name(summary.root.file_name().unwrap_or(summary.root.as_os_str())));
                let path = display_name(summary.root.as_os_str());
                let detail = format!("{}  ·  {} files  ·  {} folders", size(summary.size), summary.files, summary.folders);
                if recent_card(ui, &title, &path, &detail) {
                    app.open_last_scan(ui.ctx());
                }
            } else {
                recent_card(ui, "No scans yet", "Your most recent completed scan will appear here.", "");
            }
            ui.add_space(17.0);
            ui.label(RichText::new("You can also drop a folder or mounted disk onto this window.").color(colors.muted).size(13.0));
        });
    });
}

fn scan_card(
    ui: &mut egui::Ui,
    width: f32,
    icon: Icon,
    title: &str,
    description: &str,
    action: &str,
) -> bool {
    let colors = Palette::get(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 162.0), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, title));
    let active = response.hovered() || response.has_focus();
    let painter = ui.painter_at(rect);
    painter.rect(
        rect,
        13.0,
        if active { colors.hover } else { colors.surface },
        Stroke::new(1.0_f32, if active { colors.accent } else { colors.line }),
    );
    let tile = Rect::from_min_size(rect.min + egui::vec2(19.0, 16.0), egui::vec2(40.0, 37.0));
    painter.rect_filled(tile, 8.0, colors.hover);
    draw_icon(&painter, tile.center(), icon, colors.accent);
    for (offset, label, font, color) in [
        (65.0, title, 21.0, colors.text),
        (96.0, description, 13.0, colors.muted),
        (130.0, action, 14.0, colors.accent),
    ] {
        painter.text(
            rect.min + egui::vec2(19.0, offset),
            Align2::LEFT_TOP,
            label,
            FontId::proportional(font),
            color,
        );
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

fn recent_card(ui: &mut egui::Ui, title: &str, path: &str, detail: &str) -> bool {
    let colors = Palette::get(ui.ctx());
    let has_result = !detail.is_empty();
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 83.0),
        if has_result {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    let active = has_result && (response.hovered() || response.has_focus());
    let painter = ui.painter_at(rect);
    painter.rect(
        rect,
        10.0,
        if active { colors.hover } else { colors.surface },
        Stroke::new(1.0_f32, colors.line),
    );
    let tile = Rect::from_min_size(rect.min + egui::vec2(17.0, 20.0), egui::Vec2::splat(43.0));
    painter.rect_filled(tile, 8.0, colors.hover);
    draw_icon(&painter, tile.center(), Icon::History, colors.accent);
    let left = rect.left() + 77.0;
    let labels = painter.with_clip_rect(Rect::from_min_max(
        Pos2::new(left, rect.top()),
        Pos2::new(rect.right() - 143.0, rect.bottom()),
    ));
    labels.text(
        Pos2::new(left, rect.top() + 14.0),
        Align2::LEFT_TOP,
        title,
        FontId::proportional(16.0),
        colors.text,
    );
    labels.text(
        Pos2::new(left, rect.top() + 38.0),
        Align2::LEFT_TOP,
        path,
        FontId::proportional(12.0),
        colors.muted,
    );
    if has_result {
        labels.text(
            Pos2::new(left, rect.top() + 57.0),
            Align2::LEFT_TOP,
            detail,
            FontId::proportional(12.0),
            colors.muted,
        );
        let button = Rect::from_center_size(
            Pos2::new(rect.right() - 70.0, rect.center().y),
            egui::vec2(112.0, 33.0),
        );
        painter.rect_filled(button, 7.0, colors.blue);
        painter.text(
            button.center(),
            Align2::CENTER_CENTER,
            "Open result  ›",
            FontId::proportional(13.0),
            Color32::WHITE,
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, format!("Open last scan: {title}"))
        });
        return response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked();
    }
    false
}

fn disks(app: &mut DiskAnalyzerApp, ui: &mut egui::Ui) {
    let colors = Palette::get(ui.ctx());
    let mut scan_path = None;
    egui::ScrollArea::vertical().id_source("disks").auto_shrink([false; 2]).show(ui, |ui| {
        egui::Frame::none().inner_margin(egui::Margin::symmetric(32.0, 25.0)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("Choose a disk").size(29.0).strong());
                    ui.label(RichText::new("Scan an entire volume to map the files and folders it contains.").color(colors.muted));
                });
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Refresh disks").clicked() {
                        app.refresh_volumes(ui.ctx());
                    }
                });
            });
            ui.add_space(16.0);
            if app.volumes.is_empty() {
                ui.label(RichText::new("No disks listed. You can choose a mounted disk in the folder picker.").color(colors.muted));
                if ui.add(filled_button("Choose disk or folder…", colors.blue)).clicked() {
                    pick_folder(app, ui.ctx());
                }
            }
            for volume in app.volumes.iter().filter(|volume| !volume.system) {
                if volume_row(ui, &volume.name, &volume.detail, volume.space) {
                    scan_path = Some(volume.path.clone());
                }
            }
            if app.volumes.iter().any(|volume| volume.system) {
                ui.add_space(10.0);
                egui::CollapsingHeader::new(RichText::new("System-managed volumes").color(colors.muted).strong())
                    .id_source("system_volumes")
                    .show(ui, |ui| {
                        ui.label(RichText::new("APFS volumes can share capacity. Scan each one separately to see its accessible files.").color(colors.muted));
                        for volume in app.volumes.iter().filter(|volume| volume.system) {
                            if volume_row(ui, &volume.name, &volume.detail, volume.space) {
                                scan_path = Some(volume.path.clone());
                            }
                        }
                    });
            }
        });
    });
    if let Some(path) = scan_path {
        app.start_scan(path, ui.ctx());
    }
}

fn volume_row(ui: &mut egui::Ui, title: &str, detail: &str, space: Option<(u64, u64)>) -> bool {
    let colors = Palette::get(ui.ctx());
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 64.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 7.0, colors.surface);
    let icon = Rect::from_center_size(
        Pos2::new(rect.left() + 36.0, rect.center().y),
        egui::vec2(49.0, 33.0),
    );
    painter.rect_filled(icon, 6.0, colors.hover);
    draw_icon(painter, icon.center(), Icon::Disk, colors.accent);
    painter.text(
        rect.left_top() + egui::vec2(76.0, 13.0),
        Align2::LEFT_TOP,
        title,
        FontId::proportional(16.0),
        colors.text,
    );
    painter.text(
        rect.left_top() + egui::vec2(76.0, 37.0),
        Align2::LEFT_TOP,
        detail,
        FontId::proportional(12.0),
        colors.muted,
    );

    if let Some((total, available)) = space {
        let gauge_width = (rect.width() * 0.24).clamp(100.0, 240.0);
        let right = rect.right() - 108.0;
        let gauge = Rect::from_min_size(
            Pos2::new(right - gauge_width, rect.top() + 19.0),
            egui::vec2(gauge_width, 5.0),
        );
        painter.rect_filled(gauge, 2.5, colors.background);
        let occupancy = 1.0 - (available as f64 / total as f64) as f32;
        let filled = Rect::from_min_size(
            gauge.min,
            egui::vec2(gauge.width() * occupancy, gauge.height()),
        );
        painter.rect_filled(
            filled,
            2.5,
            if occupancy > 0.85 {
                colors.warning
            } else {
                colors.accent
            },
        );
        painter.text(
            Pos2::new(right, rect.top() + 34.0),
            Align2::RIGHT_TOP,
            format!("{} available / {}", size(available), size(total)),
            FontId::proportional(11.0),
            colors.muted,
        );
    }

    let action = Rect::from_center_size(
        Pos2::new(rect.right() - 53.0, rect.center().y),
        egui::vec2(79.0, 27.0),
    );
    ui.child_ui(
        action,
        egui::Layout::centered_and_justified(egui::Direction::TopDown),
    )
    .add(filled_button("Scan", colors.blue))
    .clicked()
}

fn progress(app: &mut DiskAnalyzerApp, ui: &mut egui::Ui) {
    let colors = Palette::get(ui.ctx());
    let rect = ui.available_rect_before_wrap();
    let panel = Rect::from_center_size(rect.center(), egui::vec2(rect.width().min(520.0), 160.0));
    ui.allocate_ui_at_rect(panel, |ui| {
        ui.vertical_centered(|ui| {
            ui.add(egui::Spinner::new().size(31.0).color(colors.accent));
            ui.add_space(9.0);
            if let Some(job) = &app.scan {
                let cancelled = job.control.cancelled.load(Relaxed);
                ui.label(
                    RichText::new(if cancelled {
                        "Cancelling…"
                    } else if job.restoring {
                        "Opening last scan…"
                    } else {
                        "Scanning storage…"
                    })
                    .size(22.0),
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(display_name(job.path.as_os_str())).color(colors.muted),
                    )
                    .truncate(true),
                );
                ui.label(
                    RichText::new(if job.restoring {
                        "Reading saved file and folder data".into()
                    } else {
                        format!(
                            "{} entries found  ·  {:.1} s",
                            job.control.visited.load(Relaxed),
                            job.started.elapsed().as_secs_f32()
                        )
                    })
                    .color(colors.muted),
                );
                ui.add_space(6.0);
                if ui
                    .add_enabled(!cancelled, egui::Button::new("Cancel"))
                    .clicked()
                {
                    job.control.cancelled.store(true, Relaxed);
                }
            } else if let Some(message) = app.cleanup_label() {
                ui.label(RichText::new(message).size(21.0));
            }
        });
    });
    ui.ctx().request_repaint_after(Duration::from_millis(90));
}

fn workspace(app: &mut DiskAnalyzerApp, ui: &mut egui::Ui) {
    let colors = Palette::get(ui.ctx());
    let Some(snapshot) = &app.snapshot else {
        return;
    };
    let focus = app.focus();
    let smaller = app.smaller();
    let selected = app.selected;
    let exporting = app.exporting();
    let rect = ui
        .available_rect_before_wrap()
        .shrink2(egui::vec2(16.0, 12.0));
    let side_width = (rect.width() * 0.41).clamp(325.0, 485.0);
    let split = rect.right() - side_width;
    let map_rect = Rect::from_min_max(rect.min, Pos2::new(split - 8.0, rect.bottom()));
    let chart_rect = map_rect.shrink2(egui::vec2(12.0, 29.0));
    let side_card = Rect::from_min_max(Pos2::new(split + 5.0, rect.top()), rect.max);
    let side = side_card.shrink2(egui::vec2(14.0, 13.0));
    for card in [map_rect, side_card] {
        ui.painter().rect(
            card,
            11.0,
            colors.surface,
            Stroke::new(1.0_f32, colors.line),
        );
    }
    ui.painter().text(
        map_rect.left_top() + egui::vec2(17.0, 14.0),
        Align2::LEFT_TOP,
        "STORAGE MAP",
        FontId::proportional(12.0),
        colors.muted,
    );
    ui.painter().text(
        map_rect.center_bottom() - egui::vec2(0.0, 13.0),
        Align2::CENTER_BOTTOM,
        "Click a folder to explore  ·  Click the center to go up",
        FontId::proportional(12.0),
        colors.muted,
    );
    let map = ui.interact(
        map_rect,
        ui.id().with(("disk_map", app.generation, focus)),
        egui::Sense::click_and_drag(),
    );
    app.chart
        .prepare(chart_rect, snapshot, focus, app.depth, app.generation);

    let pointer = map.hover_pos().or_else(|| map.interact_pointer_pos());
    let target = pointer.and_then(|pos| app.chart.hit(pos));
    let in_hub = pointer.is_some_and(|pos| pos.distance(app.chart.center) <= app.chart.hub);
    let mut action = if map.clicked() && in_hub {
        Action::Up
    } else if let Some(target) = target.filter(|_| map.clicked()) {
        Action::Open(target)
    } else {
        Action::None
    };
    let menu_id = map.id.with((app.generation, focus));
    if map.secondary_clicked() {
        ui.data_mut(|data| data.remove::<NodeId>(menu_id));
    }
    if let Some(Target::Node(id)) = target {
        if matches!(snapshot.nodes[id].kind, Kind::File | Kind::Folder) {
            map.dnd_set_drag_payload(DragItem {
                generation: app.generation,
                id,
            });
        }
        if map.secondary_clicked() {
            ui.data_mut(|data| data.insert_temp(menu_id, id));
        }
    }
    if map.secondary_clicked() || map.context_menu_opened() {
        if let Some(id) = ui.data(|data| data.get_temp::<NodeId>(menu_id)) {
            map.context_menu(|ui| {
                let next = item_menu(ui, snapshot, &app.collection, app.cached_snapshot, id);
                if !matches!(next, Action::None) {
                    action = next;
                }
            });
        }
    }
    if target.is_some() || (in_hub && (focus != 0 || app.smaller().is_some())) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    let (row_target, row_action) = ui
        .allocate_ui_at_rect(side, |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(side));
            ui.set_min_width(side.width());
            let volume_name = app
                .volumes
                .iter()
                .find(|volume| volume.path == snapshot.root)
                .map(|volume| volume.name.as_str());
            contents(
                ui,
                snapshot,
                &mut app.filter,
                View {
                    focus,
                    smaller,
                    selected,
                    chart_target: target,
                    smaller_objects: app.chart.smaller_objects(focus),
                    generation: app.generation,
                    volume_name,
                    collection: &app.collection,
                    cached: app.cached_snapshot,
                    exporting,
                },
            )
        })
        .inner;
    if !matches!(row_action, Action::None) {
        action = row_action;
    }
    let painter = ui.painter_at(map_rect);
    app.chart.draw(
        &painter,
        target.or(row_target).or(selected.map(Target::Node)),
    );

    let center = app.chart.center;
    let center_text = target
        .or(row_target)
        .map_or(snapshot.nodes[focus].size, |hit| target_size(snapshot, hit));
    painter.circle_filled(
        center,
        app.chart.hub - 1.3,
        if in_hub && focus != 0 {
            colors.hover
        } else {
            colors.surface
        },
    );
    painter.circle_stroke(
        center,
        app.chart.hub - 1.3,
        Stroke::new(1.0_f32, colors.line),
    );
    let formatted = size(center_text);
    let (amount, unit) = formatted.split_once(' ').unwrap_or((&formatted, ""));
    painter.text(
        center - egui::vec2(0.0, 5.0),
        Align2::CENTER_CENTER,
        amount,
        FontId::proportional((app.chart.hub * 0.47).clamp(12.0, 22.0)),
        colors.accent,
    );
    painter.text(
        center + egui::vec2(0.0, 11.0),
        Align2::CENTER_CENTER,
        unit,
        FontId::proportional(11.0),
        colors.muted,
    );
    if snapshot.nodes[focus].size == 0 {
        painter.text(
            center + egui::vec2(0.0, app.chart.hub + 26.0),
            Align2::CENTER_CENTER,
            "No allocated file data in this folder",
            FontId::proportional(14.0),
            colors.muted,
        );
    }

    if !app.confirm_open
        && !app.collector_open
        && !ui.ctx().wants_keyboard_input()
        && ui
            .ctx()
            .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace))
    {
        action = Action::Up;
    }
    match action {
        Action::None => {}
        Action::Up => app.up(),
        Action::Open(Target::Node(id)) => {
            if snapshot.nodes[id].kind == Kind::Folder {
                app.go_to(id);
            } else {
                app.selected = Some(id);
            }
        }
        Action::Open(Target::Smaller { parent, first }) => {
            app.navigate(Page::Map {
                folder: parent,
                smaller: Some(first),
            });
        }
        Action::Reveal(id) => app.reveal(id, ui.ctx()),
        Action::ToggleMark(id) => app.toggle_mark(id),
        Action::Export(format) => app.export_folder(focus, format, ui.ctx()),
    }
}

#[derive(Default)]
pub struct ListFilter {
    query: String,
    hidden: bool,
    key: Option<(u64, NodeId, NodeId)>,
    matches: Vec<NodeId>,
}

impl ListFilter {
    pub fn clear(&mut self) {
        self.query.clear();
        self.hidden = false;
        self.key = None;
        self.matches.clear();
    }
}

struct View<'a> {
    focus: NodeId,
    smaller: Option<NodeId>,
    selected: Option<NodeId>,
    chart_target: Option<Target>,
    smaller_objects: Option<NodeId>,
    generation: u64,
    volume_name: Option<&'a str>,
    collection: &'a [Collected],
    cached: bool,
    exporting: bool,
}

fn contents(
    ui: &mut egui::Ui,
    snapshot: &DiskUsage,
    filter: &mut ListFilter,
    view: View<'_>,
) -> (Option<Target>, Action) {
    let colors = Palette::get(ui.ctx());
    let View {
        focus,
        smaller,
        selected,
        chart_target,
        smaller_objects,
        generation,
        volume_name,
        collection,
        cached,
        exporting,
    } = view;
    let range = snapshot.children(focus);
    let start = smaller.unwrap_or(range.start);
    let inspected = chart_target.or(selected.map(Target::Node));
    let heading = if smaller.is_some() {
        "Smaller objects".into()
    } else if let Some(name) = volume_name.filter(|_| focus == 0) {
        name.to_owned()
    } else {
        display_name(&snapshot.nodes[focus].name)
    };
    let heading_size = if let Some(first) = smaller {
        target_size(
            snapshot,
            Target::Smaller {
                parent: focus,
                first,
            },
        )
    } else {
        snapshot.nodes[focus].size
    };

    ui.horizontal(|ui| {
        let (dot, _) = ui.allocate_exact_size(egui::vec2(11.0, 22.0), egui::Sense::hover());
        ui.painter().circle_filled(dot.center(), 4.2, colors.accent);
        let available = (ui.available_width() - 105.0).max(50.0);
        ui.allocate_ui_with_layout(
            egui::vec2(available, 22.0),
            egui::Layout::left_to_right(Align::Center),
            |ui| {
                ui.add(egui::Label::new(RichText::new(heading).size(19.0).strong()).truncate(true))
                    .on_hover_ui(|ui| {
                        ui.label(display_name(snapshot.path(focus).as_os_str()));
                    });
            },
        );
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(size(heading_size)).size(17.0));
        });
    });
    ui.horizontal(|ui| {
        let summary = if let Some(target) = inspected {
            match target {
                Target::Node(id) => format!(
                    "{}  ·  {}",
                    display_name(&snapshot.nodes[id].name),
                    size(snapshot.nodes[id].size)
                ),
                Target::Smaller { .. } => format!(
                    "Smaller objects  ·  {}",
                    size(target_size(snapshot, target))
                ),
            }
        } else {
            format!("{} items  ·  Allocated file space", range.end - start)
        };
        ui.add(
            egui::Label::new(RichText::new(summary).size(11.5).color(colors.muted)).truncate(true),
        );
    });
    let mut action = Action::None;
    ui.horizontal_wrapped(|ui| {
        if ui.button(finder_label(Kind::Folder)).clicked() {
            action = Action::Reveal(focus);
        }
        if focus != 0 {
            let mark = mark_status(snapshot, collection, focus);
            let label = if mark == Mark::Direct {
                "Unmark folder"
            } else {
                "Mark folder for deletion"
            };
            let enabled = !cached && snapshot.nodes[focus].complete && mark != Mark::Inherited;
            if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                action = Action::ToggleMark(focus);
            }
        }
        let menu = ui
            .add_enabled_ui(!exporting, |ui| {
                ui.menu_button(
                    if exporting {
                        "     Saving…"
                    } else {
                        "     Save result"
                    },
                    |ui| {
                        for format in [ExportFormat::Excel, ExportFormat::Csv] {
                            if ui.button(format.label()).clicked() {
                                action = Action::Export(format);
                                ui.close_menu();
                            }
                        }
                    },
                )
            })
            .inner;
        let rect = menu.response.rect;
        draw_icon(
            ui.painter(),
            Pos2::new(rect.left() + 15.0, rect.center().y),
            Icon::Save,
            if exporting {
                colors.muted
            } else {
                colors.accent
            },
        );
        menu.response
            .on_hover_text("Export all items in this folder");
    });
    if cached {
        ui.label(
            RichText::new("Saved result · Rescan to verify items for cleanup.")
                .size(11.5)
                .color(colors.warning),
        );
    }
    ui.add_space(7.0);
    let changed = ui
        .horizontal(|ui| {
            let width = (ui.available_width() - 84.0).max(80.0);
            let query = ui.add(
                egui::TextEdit::singleline(&mut filter.query)
                    .desired_width(width)
                    .hint_text("Filter this folder"),
            );
            let hidden = ui
                .checkbox(&mut filter.hidden, "Hidden")
                .on_hover_text("Show only dotfiles and items flagged hidden by the filesystem");
            query.changed() || hidden.changed()
        })
        .inner;
    let filtered = filter.hidden || !filter.query.trim().is_empty();
    let key = (generation, focus, start);
    if filtered && (changed || filter.key != Some(key)) {
        let query = filter.query.trim().to_lowercase();
        filter.matches.clear();
        filter.matches.extend((start..range.end).filter(|&id| {
            let node = &snapshot.nodes[id];
            (!filter.hidden || node.hidden)
                && (query.is_empty() || node.name.to_string_lossy().to_lowercase().contains(&query))
        }));
        filter.key = Some(key);
    } else if !filtered {
        filter.key = None;
    }
    ui.separator();
    let (header, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::hover());
    let back_rect = Rect::from_center_size(
        Pos2::new(header.left() + 14.0, header.center().y),
        egui::Vec2::splat(24.0),
    );
    let back_label = if smaller.is_some() {
        "Back to folder contents"
    } else {
        "Back to parent folder"
    };
    let mut back_ui = ui.child_ui_with_id_source(
        back_rect,
        egui::Layout::centered_and_justified(egui::Direction::TopDown),
        "parent_folder",
    );
    back_ui.spacing_mut().interact_size = back_rect.size();
    let can_go_up = focus != 0 || smaller.is_some();
    let back = back_ui.add_enabled(can_go_up, egui::Button::new(""));
    let center = back.rect.center();
    let arrow_color = if can_go_up { colors.text } else { colors.line };
    let stroke = Stroke::new(1.4_f32, arrow_color);
    ui.painter().line_segment(
        [
            center + egui::vec2(-5.0, 0.0),
            center + egui::vec2(5.0, 0.0),
        ],
        stroke,
    );
    ui.painter().add(egui::Shape::line(
        vec![
            center + egui::vec2(-1.0, -4.0),
            center + egui::vec2(-5.0, 0.0),
            center + egui::vec2(-1.0, 4.0),
        ],
        stroke,
    ));
    back.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, back_label));
    if back.on_hover_text(back_label).clicked() {
        action = Action::Up;
    }

    let font = FontId::proportional(10.5);
    for (x, label, align) in [
        (header.left() + 36.0, "NAME", Align2::LEFT_CENTER),
        (header.right() - 52.0, "SIZE", Align2::RIGHT_CENTER),
        (header.right() - 21.0, "MARK", Align2::CENTER_CENTER),
    ] {
        ui.painter().text(
            Pos2::new(x, header.center().y),
            align,
            label,
            font.clone(),
            colors.muted,
        );
    }

    // Keep unreadable and zero-byte items visible beyond the positive-size group.
    let group = smaller_objects
        .filter(|_| smaller.is_none() && !filtered)
        .map(|first| {
            let end =
                range.start + snapshot.nodes[range.clone()].partition_point(|node| node.size > 0);
            first..end
        });
    let total_rows = if filtered {
        filter.matches.len()
    } else {
        range.end
            - start
            - group
                .as_ref()
                .map_or(0, |range| range.len().saturating_sub(1))
    };
    let mut hover = None;
    if total_rows == 0 {
        let message = if filtered {
            "No matching files or folders."
        } else {
            "No files or folders to show."
        };
        ui.label(RichText::new(message).color(colors.muted));
    }
    egui::ScrollArea::vertical()
        .id_source(("contents", generation, focus, start, filtered))
        .auto_shrink([false; 2])
        .show_rows(ui, 23.0, total_rows, |ui, rows| {
            for index in rows {
                let mut id = if filtered {
                    filter.matches[index]
                } else {
                    start + index
                };
                if let Some(group) = &group {
                    if id == group.start {
                        let target = Target::Smaller {
                            parent: focus,
                            first: id,
                        };
                        let (rect, row) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width() - 1.0, 23.0),
                            egui::Sense::click(),
                        );
                        row.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::WidgetType::Button, "Smaller objects")
                        });
                        if row.hovered() || chart_target == Some(target) || row.has_focus() {
                            ui.painter().rect_filled(rect, 4.0, colors.hover);
                        }
                        if row.hovered() {
                            hover = Some(target);
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if row.clicked() {
                            action = Action::Open(target);
                        }
                        paint_row(
                            ui,
                            Rect::from_min_max(
                                rect.min,
                                Pos2::new(rect.right() - 43.0, rect.bottom()),
                            ),
                            chart::SMALL,
                            "Smaller objects…",
                            &size(target_size(snapshot, target)),
                            colors.muted,
                        );
                        continue;
                    }
                    if id > group.start {
                        id += group.len().saturating_sub(1);
                    }
                }
                let node = &snapshot.nodes[id];
                let target = Target::Node(id);
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width() - 1.0, 23.0),
                    egui::Sense::hover(),
                );
                let content_rect =
                    Rect::from_min_max(rect.min, Pos2::new(rect.right() - 43.0, rect.bottom()));
                let row = ui.interact(
                    content_rect,
                    ui.id().with(("entry", id)),
                    egui::Sense::click_and_drag(),
                );
                let mark = mark_status(snapshot, collection, id);
                row.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, display_name(&node.name))
                });
                if row.hovered() {
                    hover = Some(target);
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                let active = selected == Some(id)
                    || chart_target == Some(target)
                    || row.hovered()
                    || row.has_focus();
                if active || mark != Mark::Clear {
                    ui.painter().rect_filled(rect, 4.0, colors.hover);
                }
                let suffix = match node.kind {
                    Kind::Unreadable => "Unavailable".into(),
                    Kind::OtherVolume => "Separate volume".into(),
                    _ => size(node.size),
                };
                let label = if node.hidden {
                    format!("{}  ·  hidden", display_name(&node.name))
                } else {
                    display_name(&node.name)
                };
                paint_row(
                    ui,
                    content_rect,
                    chart::row_color(node.kind, id - range.start),
                    &label,
                    &suffix,
                    if node.kind == Kind::Unreadable {
                        colors.warning
                    } else {
                        colors.muted
                    },
                );
                if matches!(node.kind, Kind::File | Kind::Folder) {
                    row.dnd_set_drag_payload(DragItem { generation, id });
                }
                row.context_menu(|ui| {
                    let next = item_menu(ui, snapshot, collection, cached, id);
                    if !matches!(next, Action::None) {
                        action = next;
                    }
                });
                if row.clicked() {
                    action = Action::Open(target);
                }
                if matches!(node.kind, Kind::Folder | Kind::File) {
                    let marker = Rect::from_center_size(
                        Pos2::new(rect.right() - 21.0, rect.center().y),
                        egui::vec2(28.0, 21.0),
                    );
                    let enabled = !cached && node.complete && mark != Mark::Inherited;
                    let response = ui.interact(
                        marker,
                        ui.id().with(("mark", id)),
                        if enabled {
                            egui::Sense::click()
                        } else {
                            egui::Sense::hover()
                        },
                    );
                    let label = match mark {
                        Mark::Clear => "Mark for deletion",
                        Mark::Direct => "Unmark for deletion",
                        Mark::Inherited => "Marked through a parent folder",
                    };
                    let square = Rect::from_center_size(marker.center(), Vec2::splat(13.0));
                    let stroke = if mark != Mark::Clear || (enabled && response.hovered()) {
                        colors.red
                    } else {
                        colors.line
                    };
                    ui.painter()
                        .rect_stroke(square, 3.0, Stroke::new(1.2_f32, stroke));
                    if mark != Mark::Clear {
                        let center = square.center();
                        ui.painter().add(egui::Shape::line(
                            vec![
                                center + egui::vec2(-3.5, 0.0),
                                center + egui::vec2(-1.0, 2.5),
                                center + egui::vec2(4.0, -2.7),
                            ],
                            Stroke::new(1.5_f32, colors.red),
                        ));
                    }
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Checkbox,
                            format!("{label}: {}", display_name(&node.name)),
                        )
                    });
                    if response.clicked() {
                        action = Action::ToggleMark(id);
                    }
                    response.on_hover_text(if cached {
                        "Rescan before marking for deletion"
                    } else if !node.complete {
                        "This folder contains unavailable items"
                    } else {
                        label
                    });
                }
                row.on_hover_ui(|ui| {
                    ui.label(display_name(snapshot.path(id).as_os_str()));
                });
            }
        });
    (hover, action)
}

fn paint_row(
    ui: &egui::Ui,
    rect: Rect,
    color: Color32,
    name: &str,
    detail: &str,
    detail_color: Color32,
) {
    let colors = Palette::get(ui.ctx());
    ui.painter()
        .circle_filled(Pos2::new(rect.left() + 10.0, rect.center().y), 3.1, color);
    let name_area = Rect::from_min_max(
        Pos2::new(rect.left() + 21.0, rect.top()),
        Pos2::new(rect.right() - 103.0, rect.bottom()),
    );
    ui.painter().with_clip_rect(name_area).text(
        name_area.left_center(),
        Align2::LEFT_CENTER,
        name,
        FontId::proportional(13.0),
        colors.text,
    );
    ui.painter().text(
        Pos2::new(rect.right() - 9.0, rect.center().y),
        Align2::RIGHT_CENTER,
        detail,
        FontId::proportional(12.0),
        detail_color,
    );
}

#[derive(PartialEq, Eq)]
enum Mark {
    Clear,
    Direct,
    Inherited,
}

fn mark_status(snapshot: &DiskUsage, collection: &[Collected], id: NodeId) -> Mark {
    if collection.is_empty() {
        return Mark::Clear;
    }
    let path = snapshot.path(id);
    for item in collection {
        if path == item.path {
            return Mark::Direct;
        }
        if path.starts_with(&item.path) {
            return Mark::Inherited;
        }
    }
    Mark::Clear
}

fn finder_label(kind: Kind) -> &'static str {
    let folder = matches!(kind, Kind::Folder | Kind::OtherVolume);
    match (cfg!(target_os = "macos"), folder) {
        (true, true) => "Open in Finder",
        (true, false) => "Reveal in Finder",
        (false, true) => "Open in file manager",
        (false, false) => "Reveal in file manager",
    }
}

fn item_menu(
    ui: &mut egui::Ui,
    snapshot: &DiskUsage,
    collection: &[Collected],
    cached: bool,
    id: NodeId,
) -> Action {
    let node = &snapshot.nodes[id];
    let mut action = Action::None;
    if node.kind == Kind::Folder && ui.button("Explore folder").clicked() {
        action = Action::Open(Target::Node(id));
    }
    if ui.button(finder_label(node.kind)).clicked() {
        action = Action::Reveal(id);
    }
    if matches!(node.kind, Kind::Folder | Kind::File) {
        let mark = mark_status(snapshot, collection, id);
        let label = if mark == Mark::Direct {
            "Unmark for deletion"
        } else {
            "Mark for deletion"
        };
        if ui
            .add_enabled(
                !cached && node.complete && mark != Mark::Inherited,
                egui::Button::new(label),
            )
            .clicked()
        {
            action = Action::ToggleMark(id);
        }
    }
    if !matches!(action, Action::None) {
        ui.close_menu();
    }
    action
}

fn target_size(snapshot: &DiskUsage, target: Target) -> u64 {
    match target {
        Target::Node(id) => snapshot.nodes[id].size,
        Target::Smaller { parent, first } => {
            let range = snapshot.children(parent);
            let parent_size = snapshot.nodes[parent].size;
            if first - range.start < range.end - first {
                parent_size.saturating_sub(
                    snapshot.nodes[range.start..first]
                        .iter()
                        .map(|node| node.size)
                        .sum(),
                )
            } else {
                snapshot.nodes[first..range.end]
                    .iter()
                    .map(|node| node.size)
                    .sum()
            }
        }
    }
}

fn footer(app: &mut DiskAnalyzerApp, ctx: &egui::Context) {
    let colors = Palette::get(ctx);
    let height = if app.status.is_some() { 77.0 } else { 49.0 };
    egui::TopBottomPanel::bottom("footer").exact_height(height).frame(
        egui::Frame::none().fill(colors.surface).inner_margin(egui::Margin::symmetric(16.0, 10.0))
    ).show(ctx, |ui| {
        ui.set_enabled(!app.confirm_open);
        if let Some((message, warning)) = &app.status {
            let dismissed = ui.horizontal(|ui| {
                ui.add(egui::Label::new(RichText::new(message).color(if *warning { colors.warning } else { colors.accent })).truncate(true));
                ui.small_button("Dismiss").clicked()
            }).inner;
            if dismissed {
                app.status = None;
            }
        }
        if !app.showing_map() {
            ui.label(RichText::new("Scans stay on this Mac").size(12.0).color(colors.muted));
            return;
        }
        ui.horizontal(|ui| {
            let collected = app.collection.iter().map(|item| item.size).sum::<u64>();
            let button = ui.add_enabled(
                !app.busy(),
                egui::Button::new(format!("Cleanup  ·  {} marked", app.collection.len())).fill(colors.hover).min_size(egui::vec2(134.0, 29.0)),
            );
            if button.clicked() {
                app.collector_open = !app.collector_open;
            }
            if let Some(item) = button.dnd_release_payload::<DragItem>() {
                if item.generation == app.generation && !app.busy() && !app.confirm_open {
                    app.collect(item.id);
                }
            }
            if button.dnd_hover_payload::<DragItem>().is_some() {
                ui.painter().rect_stroke(button.rect, 6.0, Stroke::new(1.5_f32, colors.accent));
            }
            if !app.collection.is_empty() {
                if ui.add_enabled(!app.busy(), filled_button("Review cleanup", colors.red)).clicked() {
                    app.collector_open = true;
                }
            } else {
                ui.label(RichText::new("Mark items to build a cleanup list").color(colors.muted).size(11.0));
            }

            if collected > 0 {
                ui.label(RichText::new(size(collected)).color(colors.muted));
            }
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if let Some(snapshot) = app.snapshot.as_ref().filter(|_| !app.busy()) {
                    let stats = format!("{} files  ·  {} folders  ·  {:.2} s", snapshot.files, snapshot.folders, snapshot.elapsed.as_secs_f64());
                    ui.label(RichText::new(stats).size(11.0).color(colors.muted))
                        .on_hover_text("Counts and elapsed time are for the whole scan.");
                    if snapshot.unreadable > 0 {
                        ui.colored_label(colors.warning, format!("{} unavailable", snapshot.unreadable))
                            .on_hover_text("These entries could not be measured. Permissions and cloud-only items can limit access.");
                    }
                    if snapshot.skipped_volumes > 0 {
                        ui.label(RichText::new(format!("{} volume boundaries", snapshot.skipped_volumes)).color(colors.muted).size(11.0))
                            .on_hover_text("Mount points and APFS links are listed separately to avoid scanning the same data twice.");
                    }
                } else {
                    ui.label(RichText::new("Discope  ·  Local processing").color(colors.muted).size(11.0));
                }
            });
        });
    });
}

fn collector_window(app: &mut DiskAnalyzerApp, ctx: &egui::Context) {
    let colors = Palette::get(ctx);
    if !app.collector_open || !app.showing_map() || app.busy() {
        return;
    }
    let mut open = true;
    egui::Window::new("Cleanup review")
        .open(&mut open)
        .collapsible(false)
        .default_width(420.0)
        .anchor(Align2::LEFT_BOTTOM, egui::vec2(17.0, -62.0))
        .show(ctx, |ui| {
            ui.set_enabled(!app.confirm_open);
            ui.label(
                RichText::new("Review the files and folders marked for deletion.")
                    .color(colors.muted),
            );
            if app.collection.is_empty() {
                ui.label("Use the Mark column or drag items onto the Cleanup button.");
            }
            let mut remove = None;
            egui::ScrollArea::vertical()
                .max_height(230.0)
                .show(ui, |ui| {
                    for (index, item) in app.collection.iter().enumerate() {
                        ui.horizontal(|ui| {
                            if ui.small_button("Unmark").clicked() {
                                remove = Some(index);
                            }
                            ui.label(RichText::new(size(item.size)).color(colors.muted));
                            ui.add(
                                egui::Label::new(display_name(
                                    item.path.file_name().unwrap_or(item.path.as_os_str()),
                                ))
                                .truncate(true),
                            )
                            .on_hover_ui(|ui| {
                                ui.label(display_name(item.path.as_os_str()));
                            });
                        });
                    }
                });
            if let Some(index) = remove {
                app.collection.remove(index);
            }
            ui.separator();
            ui.horizontal(|ui| {
                let enabled = app.can_clean();
                if ui
                    .add_enabled(enabled, egui::Button::new("Preview cleanup"))
                    .clicked()
                {
                    app.preview_cleanup(ctx);
                }
                if ui
                    .add_enabled(enabled, filled_button("Move to Trash…", colors.red))
                    .clicked()
                {
                    app.request_cleanup();
                }
            });
        });
    app.collector_open &= open;
}

fn confirmation(app: &mut DiskAnalyzerApp, ctx: &egui::Context) {
    let colors = Palette::get(ctx);
    if !app.confirm_open {
        return;
    }
    egui::Window::new("Confirm Move to Trash")
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .show(ctx, |ui| {
            ui.label(format!(
                "Move {} collected item(s) to Trash?",
                app.collection.len()
            ));
            ui.label("Marked folders will be moved with everything inside them.");
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    app.confirm_open = false;
                }
                if ui.add(filled_button("Move to Trash", colors.red)).clicked() {
                    app.cleanup(ctx);
                }
            });
        });
}

fn size(bytes: u64) -> String {
    format_size(bytes, DECIMAL)
}

fn short_name(name: &std::ffi::OsStr, max_chars: usize) -> String {
    let mut text = display_name(name);
    if let Some((index, _)) = text.char_indices().nth(max_chars) {
        text.truncate(index);
        text.push('…');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk_scanner::{scan, tests::Fixture, ScanControl};

    #[test]
    fn map_navigation_and_marking_do_not_rescan_or_move_files() {
        let fixture = Fixture::new();
        fixture.write("folder/file", 8192);
        let ctx = egui::Context::default();
        let mut app = DiskAnalyzerApp::new(&ctx, None);
        app.snapshot = Some(std::sync::Arc::new(
            scan(&fixture.0, &ScanControl::default()).unwrap(),
        ));
        app.go_to(0);

        let frame = |app: &mut DiskAnalyzerApp, events| {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1100.0, 720.0))),
                events,
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| render(app, ctx));
        };
        frame(&mut app, Vec::new());
        frame(&mut app, Vec::new());
        let click = |app: &mut DiskAnalyzerApp, pos| {
            frame(app, vec![egui::Event::PointerMoved(pos)]);
            for pressed in [true, false] {
                frame(
                    app,
                    vec![egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
            }
        };

        let folder = app.chart.center + egui::vec2(0.0, app.chart.hub + 12.0);
        click(&mut app, folder);
        assert_eq!(app.focus(), 1);
        assert!(app.scan.is_none());

        click(&mut app, egui::pos2(1047.0, 258.0));
        assert_eq!(app.focus(), 1);
        assert_eq!(app.collection.len(), 1);
        assert!(fixture.0.join("folder/file").exists());
        click(&mut app, egui::pos2(220.0, 695.0));
        assert!(app.collector_open);
        app.collector_open = false;

        let list_back = egui::pos2(679.0, 226.0);
        click(&mut app, list_back);
        assert_eq!(app.focus(), 0);
        click(&mut app, list_back);
        assert_eq!(app.focus(), 0);
        assert!(app.scan.is_none());
        app.go_to(1);
        frame(&mut app, Vec::new());

        let hub = app.chart.center;
        click(&mut app, hub);
        assert_eq!(app.focus(), 0);
        assert!(app.scan.is_none());
    }
}
