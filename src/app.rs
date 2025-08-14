use eframe::egui;
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use crate::disk_scanner::{DiskScanner, DiskUsage};
use crate::ui::DiskAnalyzerUI;

pub struct DiskAnalyzerApp {
    scanner: DiskScanner,
    current_path: Option<PathBuf>,
    disk_usage: Option<DiskUsage>,
    is_scanning: bool,
    scan_progress: f32,
    selected_folder: Option<PathBuf>,
    expanded_nodes: HashSet<PathBuf>,
    // safety features
    dry_run: bool,
    require_confirm: bool,
    recently_deleted: VecDeque<PathBuf>,
    // navigation history
    history: Vec<PathBuf>,
    history_index: isize,
    // scan config
    max_depth: usize,
}

impl DiskAnalyzerApp {
    pub fn new() -> Self {
        Self {
            scanner: DiskScanner::new(),
            current_path: None,
            disk_usage: None,
            is_scanning: false,
            scan_progress: 0.0,
            selected_folder: None,
            expanded_nodes: HashSet::new(),
            dry_run: true,
            require_confirm: true,
            recently_deleted: VecDeque::with_capacity(10),
            history: Vec::new(),
            history_index: -1,
            max_depth: 8,
        }
    }

    fn scan_without_history(&mut self, path: &PathBuf) {
        self.current_path = Some(path.clone());
        self.is_scanning = true;
        self.scan_progress = 0.0;
        self.expanded_nodes.clear();
        match self.scanner.scan_directory_with_depth(path, self.max_depth) {
            Ok(usage) => { self.disk_usage = Some(usage); self.is_scanning = false; self.scan_progress = 1.0; }
            Err(e) => { eprintln!("Error scanning directory: {}", e); self.is_scanning = false; self.scan_progress = 0.0; }
        }
    }

    pub fn scan_directory(&mut self, path: PathBuf) {
        if self.history_index >= 0 && (self.history_index as usize) < self.history.len() - 1 { let idx = (self.history_index + 1) as usize; self.history.truncate(idx); }
        self.history.push(path.clone());
        self.history_index = self.history.len() as isize - 1;
        self.scan_without_history(&path);
    }

    pub fn can_go_back(&self) -> bool { self.history_index > 0 }
    pub fn can_go_forward(&self) -> bool { self.history_index >= 0 && (self.history_index as usize) < self.history.len() - 1 }

    pub fn go_back(&mut self) { if self.can_go_back() { self.history_index -= 1; let path = self.history[self.history_index as usize].clone(); self.scan_without_history(&path); } }
    pub fn go_forward(&mut self) { if self.can_go_forward() { self.history_index += 1; let path = self.history[self.history_index as usize].clone(); self.scan_without_history(&path); } }

    pub fn set_dry_run(&mut self, v: bool) { self.dry_run = v; }
    pub fn dry_run(&self) -> bool { self.dry_run }
    pub fn set_require_confirm(&mut self, v: bool) { self.require_confirm = v; }
    pub fn require_confirm(&self) -> bool { self.require_confirm }

    pub fn set_max_depth(&mut self, depth: usize) { self.max_depth = depth.max(2).min(20); }
    pub fn max_depth(&self) -> usize { self.max_depth }

    pub fn move_folder_to_trash(&mut self, path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        if self.dry_run { println!("[Dry run] Would move to Trash: {}", path.display()); return Ok(()); }
        if path.exists() {
            trash::delete(path)?;
            self.recently_deleted.push_back(path.clone());
            if self.recently_deleted.len() > 10 { self.recently_deleted.pop_front(); }
            if let Some(current_path) = &self.current_path { if path.starts_with(current_path) { let cp = current_path.clone(); self.scan_without_history(&cp); } }
        }
        Ok(())
    }

    // Note: We intentionally omit an undo implementation to avoid misleading users

    pub fn get_current_path(&self) -> Option<&PathBuf> { self.current_path.as_ref() }
    pub fn get_disk_usage(&self) -> Option<&DiskUsage> { self.disk_usage.as_ref() }
    pub fn is_scanning(&self) -> bool { self.is_scanning }
    pub fn get_scan_progress(&self) -> f32 { self.scan_progress }

    pub fn set_selected_folder(&mut self, path: Option<PathBuf>) { self.selected_folder = path; }
    pub fn get_selected_folder(&self) -> Option<&PathBuf> { self.selected_folder.as_ref() }

    pub fn is_expanded(&self, path: &PathBuf) -> bool { self.expanded_nodes.contains(path) }
    pub fn toggle_expanded(&mut self, path: &PathBuf) { if self.expanded_nodes.contains(path) { self.expanded_nodes.remove(path); } else { self.expanded_nodes.insert(path.clone()); } }
}

impl eframe::App for DiskAnalyzerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) { DiskAnalyzerUI::render(self, ctx); }
}
