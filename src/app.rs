use crate::chart::ChartCache;
use crate::disk_scanner::cache::{Cache, Summary};
use crate::disk_scanner::{self, DiskUsage, Identity, Kind, NodeId, ScanControl, Volume};
use crate::export::ExportFormat;
use crate::ui;
use eframe::egui;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{atomic::Ordering::Relaxed, mpsc, Arc};
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Home,
    Disks,
    Map {
        folder: NodeId,
        smaller: Option<NodeId>,
    },
}

const ROOT_MAP: Page = Page::Map {
    folder: 0,
    smaller: None,
};

pub struct ScanJob {
    pub path: PathBuf,
    pub started: Instant,
    pub restoring: bool,
    pub control: Arc<ScanControl>,
    receiver: mpsc::Receiver<io::Result<DiskUsage>>,
}

#[derive(Clone)]
pub struct Collected {
    pub path: PathBuf,
    pub size: u64,
    kind: Kind,
    identity: Identity,
}

struct CleanupJob {
    preview: bool,
    receiver: mpsc::Receiver<(usize, Vec<String>)>,
}

struct ExportJob {
    path: PathBuf,
    worker: std::thread::JoinHandle<io::Result<()>>,
}

pub struct DiskAnalyzerApp {
    pub snapshot: Option<Arc<DiskUsage>>,
    pub last_scan: Option<Summary>,
    pub cached_snapshot: bool,
    pub dark: bool,
    pub scan: Option<ScanJob>,
    pub volumes: Vec<Volume>,
    pub collection: Vec<Collected>,
    pub selected: Option<NodeId>,
    pub status: Option<(String, bool)>,
    pub confirm_open: bool,
    pub collector_open: bool,
    pub depth: usize,
    pub chart: ChartCache,
    pub filter: crate::ui::ListFilter,
    pub generation: u64,
    history: Vec<Page>,
    last_map: Page,
    cache: Option<Cache>,
    cache_query: Option<mpsc::Receiver<Option<Summary>>>,
    cache_sender: Option<mpsc::Sender<Arc<DiskUsage>>>,
    cache_writer: Option<std::thread::JoinHandle<()>>,
    cache_errors: Option<mpsc::Receiver<String>>,
    finder: Option<mpsc::Receiver<io::Result<()>>>,
    history_index: usize,
    volume_query: Option<mpsc::Receiver<Vec<Volume>>>,
    cleanup: Option<CleanupJob>,
    export: Option<ExportJob>,
}

impl DiskAnalyzerApp {
    pub fn new(ctx: &egui::Context, initial_folder: Option<PathBuf>) -> Self {
        let cache = Cache::local();
        let dark = cache.as_ref().is_some_and(Cache::dark);
        ui::install_style(ctx, dark);
        let mut app = Self {
            snapshot: None,
            last_scan: None,
            cached_snapshot: false,
            dark,
            scan: None,
            volumes: Vec::new(),
            collection: Vec::new(),
            selected: None,
            status: None,
            confirm_open: false,
            collector_open: false,
            depth: 10,
            chart: ChartCache::default(),
            filter: crate::ui::ListFilter::default(),
            generation: 0,
            history: vec![Page::Home],
            history_index: 0,
            last_map: ROOT_MAP,
            cache,
            cache_query: None,
            cache_sender: None,
            cache_writer: None,
            cache_errors: None,
            finder: None,
            volume_query: None,
            cleanup: None,
            export: None,
        };
        app.refresh_volumes(ctx);
        if let Some(cache) = app.cache.clone() {
            let (sender, receiver) = mpsc::channel();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let _ = sender.send(cache.summary().ok());
                ctx.request_repaint();
            });
            app.cache_query = Some(receiver);
        }
        if let Some(path) = initial_folder {
            app.start_scan(path, ctx);
        }
        app
    }

    pub fn refresh_volumes(&mut self, ctx: &egui::Context) {
        if self.volume_query.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = sender.send(disk_scanner::volumes());
            ctx.request_repaint();
        });
        self.volume_query = Some(receiver);
    }

    pub fn busy(&self) -> bool {
        self.scan.is_some() || self.cleanup.is_some()
    }

    pub fn cleanup_label(&self) -> Option<&'static str> {
        self.cleanup.as_ref().map(|job| {
            if job.preview {
                "Checking collected items…"
            } else {
                "Moving collected items to Trash…"
            }
        })
    }

    pub fn set_dark(&mut self, dark: bool, ctx: &egui::Context) {
        self.dark = dark;
        ui::install_style(ctx, dark);
        if let Some(cache) = &self.cache {
            if let Err(error) = cache.save_appearance(dark) {
                self.status = Some((format!("Could not save appearance · {error}"), true));
            }
        }
    }

    pub fn open_last_scan(&mut self, ctx: &egui::Context) {
        if self.snapshot.is_some() {
            self.navigate(self.last_map);
        } else if let Some(summary) = &self.last_scan {
            if let Some(cache) = self.cache.clone() {
                self.start_job(summary.root.clone(), Some(cache), ctx);
            }
        }
    }

    pub fn start_scan(&mut self, path: PathBuf, ctx: &egui::Context) {
        self.start_job(path, None, ctx);
    }

    fn start_job(&mut self, path: PathBuf, cache: Option<Cache>, ctx: &egui::Context) {
        if self.busy() {
            return;
        }
        let restoring = cache.is_some();
        let (sender, receiver) = mpsc::channel();
        let control = Arc::new(ScanControl::default());
        let worker_control = Arc::clone(&control);
        let worker_path = path.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = match cache {
                Some(cache) => cache.load(&worker_control),
                None => disk_scanner::scan(&worker_path, &worker_control),
            };
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        self.status = None;
        self.scan = Some(ScanJob {
            path,
            started: Instant::now(),
            restoring,
            control,
            receiver,
        });
    }

    fn save_scan(&mut self, snapshot: Arc<DiskUsage>, ctx: &egui::Context) {
        if self.cache_sender.is_none() {
            let Some(cache) = self.cache.clone() else {
                return;
            };
            let (sender, receiver) = mpsc::channel::<Arc<DiskUsage>>();
            let (errors, messages) = mpsc::channel();
            let ctx = ctx.clone();
            self.cache_writer = Some(std::thread::spawn(move || {
                while let Ok(snapshot) = receiver.recv() {
                    // Coalesce queued snapshots when scans finish faster than the cache can write.
                    let snapshot = receiver.try_iter().last().unwrap_or(snapshot);
                    if let Err(error) = cache.save(&snapshot) {
                        let _ = errors.send(format!("Could not save the last scan · {error}"));
                        ctx.request_repaint();
                    }
                }
            }));
            self.cache_errors = Some(messages);
            self.cache_sender = Some(sender);
        }
        if let Some(sender) = &self.cache_sender {
            let _ = sender.send(snapshot);
        }
    }

    pub fn exporting(&self) -> bool {
        self.export.is_some()
    }

    pub fn export_folder(&mut self, folder: NodeId, format: ExportFormat, ctx: &egui::Context) {
        if self.exporting() || self.busy() {
            return;
        }
        let Some(snapshot) = self.snapshot.as_ref().map(Arc::clone) else {
            return;
        };
        let name = disk_scanner::display_name(&snapshot.nodes[folder].name);
        let name = name.trim_matches(['.', ' ']);
        let name: String = name
            .chars()
            .take(100)
            .map(|ch| {
                if matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                    '_'
                } else {
                    ch
                }
            })
            .collect();
        let name = if name.is_empty() || name == "_" {
            "storage"
        } else {
            &name
        };
        let extension = format.extension();
        let Some(mut path) = rfd::FileDialog::new()
            .set_title("Save Discope result")
            .add_filter(format.label(), &[extension])
            .set_file_name(format!("{name}-discope.{extension}"))
            .save_file()
        else {
            return;
        };
        let overwrite = match path.extension() {
            Some(value) if value.eq_ignore_ascii_case(extension) => true,
            Some(_) => {
                self.status = Some((
                    format!("Choose a .{extension} filename for this export."),
                    true,
                ));
                return;
            }
            None => {
                path.set_extension(extension);
                false
            }
        };
        let destination = path.clone();
        let worker =
            std::thread::spawn(move || format.save(&snapshot, folder, &destination, overwrite));
        self.export = Some(ExportJob { path, worker });
        self.status = None;
        ctx.request_repaint();
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(job) = &self.export {
            if job.worker.is_finished() {
                let job = self.export.take().unwrap();
                let result = job
                    .worker
                    .join()
                    .unwrap_or_else(|_| Err(io::Error::other("The export ended unexpectedly")));
                self.status = Some(match result {
                    Ok(()) => (
                        format!("Saved {}", disk_scanner::display_name(job.path.as_os_str())),
                        false,
                    ),
                    Err(error) => (format!("Could not save result · {error}"), true),
                });
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }
        if let Some(receiver) = &self.cache_query {
            match receiver.try_recv() {
                Ok(summary) => {
                    if self.last_scan.is_none() {
                        self.last_scan = summary;
                    }
                    self.cache_query = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => self.cache_query = None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(message) = self.cache_errors.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.status = Some((message, true));
        }
        if let Some(receiver) = &self.finder {
            if let Ok(result) = receiver.try_recv() {
                self.finder = None;
                if let Err(error) = result {
                    self.status = Some((format!("Could not open item · {error}"), true));
                }
            }
        }
        if let Some(receiver) = &self.volume_query {
            match receiver.try_recv() {
                Ok(volumes) => {
                    self.volumes = volumes;
                    self.volume_query = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => self.volume_query = None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(job) = &self.cleanup {
            match job.receiver.try_recv() {
                Ok((count, errors)) => {
                    let preview = job.preview;
                    self.cleanup = None;
                    let mut message = if preview {
                        format!("Dry run · {count} item(s) eligible for Trash.")
                    } else {
                        format!("Moved {count} item(s) to Trash.")
                    };
                    if let Some(error) = errors.first() {
                        message.push_str(&format!(
                            " {} could not be processed. {error}",
                            errors.len()
                        ));
                    }
                    if !preview && count > 0 {
                        self.cached_snapshot = true;
                        if let Some(snapshot) = &self.snapshot {
                            self.start_scan(snapshot.root.clone(), ctx);
                        }
                    }
                    self.status = Some((message, !errors.is_empty()));
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.cleanup = None;
                    self.status = Some((
                        "Cleanup ended unexpectedly. Rescan to check the remaining items.".into(),
                        true,
                    ));
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        let Some(job) = &self.scan else { return };
        let result = match job.receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err(io::Error::other("The scan worker stopped"))
            }
        };
        let cancelled = job.control.cancelled.load(Relaxed);
        let restoring = job.restoring;
        self.scan = None;
        match result {
            Ok(snapshot) if !cancelled => {
                let snapshot = Arc::new(snapshot);
                self.last_scan = Some(Summary::from_usage(&snapshot));
                self.cached_snapshot = restoring;
                if !restoring {
                    self.save_scan(Arc::clone(&snapshot), ctx);
                }
                if let Some(previous) = self.snapshot.replace(snapshot) {
                    std::thread::spawn(move || drop(previous));
                }
                self.history.truncate(self.history_index + 1);
                self.history
                    .retain(|page| !matches!(page, Page::Map { .. }));
                self.history_index = self.history.len() - 1;
                self.navigate(ROOT_MAP);
                self.collection.clear();
                self.collector_open = false;
                self.generation = self.generation.wrapping_add(1);
            }
            Ok(_) => self.status = Some(("Scan cancelled.".into(), false)),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                self.status = Some(("Scan cancelled.".into(), false));
            }
            Err(error) => {
                if restoring {
                    self.status = Some((format!("Could not open the saved scan · {error}"), true));
                } else {
                    self.status = Some((format!("Scan failed · {error}"), true));
                }
            }
        }
    }

    pub fn page(&self) -> Page {
        self.history[self.history_index]
    }

    pub fn showing_map(&self) -> bool {
        matches!(self.page(), Page::Map { .. })
    }

    pub fn focus(&self) -> NodeId {
        match self.page() {
            Page::Map { folder, .. } => folder,
            _ => 0,
        }
    }

    pub fn smaller(&self) -> Option<NodeId> {
        match self.page() {
            Page::Map { smaller, .. } => smaller,
            _ => None,
        }
    }

    pub fn navigate(&mut self, page: Page) {
        if self.page() == page {
            return;
        }
        if self.history.get(self.history_index + 1) != Some(&page) {
            self.history.truncate(self.history_index + 1);
            self.history.push(page);
        }
        self.history_index += 1;
        self.page_changed();
    }

    fn page_changed(&mut self) {
        if self.showing_map() {
            self.last_map = self.page();
        } else {
            self.collector_open = false;
        }
        self.selected = None;
        self.filter.clear();
    }

    pub fn go_to(&mut self, folder: NodeId) {
        self.navigate(Page::Map {
            folder,
            smaller: None,
        });
    }

    pub fn up(&mut self) {
        if self.smaller().is_some() {
            self.go_to(self.focus());
        } else if let Some(snapshot) = &self.snapshot {
            self.go_to(snapshot.nodes[self.focus()].parent);
        }
    }

    pub fn can_back(&self) -> bool {
        self.history_index > 0
    }

    pub fn can_forward(&self) -> bool {
        self.history_index + 1 < self.history.len()
    }

    pub fn back(&mut self) {
        if self.can_back() {
            self.history_index -= 1;
            self.page_changed();
        }
    }

    pub fn forward(&mut self) {
        if self.can_forward() {
            self.history_index += 1;
            self.page_changed();
        }
    }

    pub fn toggle_mark(&mut self, id: NodeId) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let path = snapshot.path(id);
        if let Some(index) = self.collection.iter().position(|item| item.path == path) {
            self.collection.remove(index);
        } else {
            self.collect(id);
        }
    }

    pub fn collect(&mut self, id: NodeId) {
        if self.cached_snapshot {
            self.status = Some((
                "Rescan to verify items before marking them for deletion.".into(),
                true,
            ));
            return;
        }
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let node = &snapshot.nodes[id];
        if id == 0 || !matches!(node.kind, Kind::File | Kind::Folder) {
            return;
        }
        let path = snapshot.path(id);
        if !node.complete {
            self.status = Some((
                "This folder contains unmeasured entries or volume boundaries.".into(),
                true,
            ));
            return;
        }
        if protected(&path) {
            self.status = Some((
                "This system location is protected from cleanup.".into(),
                true,
            ));
            return;
        }
        if self
            .collection
            .iter()
            .any(|item| path.starts_with(&item.path))
        {
            return;
        }
        self.collection.retain(|item| !item.path.starts_with(&path));
        self.collection.push(Collected {
            path,
            size: node.size,
            identity: node.identity,
            kind: node.kind,
        });
        self.status = None;
    }

    pub fn can_clean(&self) -> bool {
        !self.busy() && !self.cached_snapshot && !self.collection.is_empty()
    }

    pub fn preview_cleanup(&mut self, ctx: &egui::Context) {
        if self.can_clean() {
            self.run_cleanup(ctx, true);
        }
    }

    pub fn request_cleanup(&mut self) {
        self.confirm_open = self.can_clean();
    }

    pub fn cleanup(&mut self, ctx: &egui::Context) {
        if self.confirm_open && self.can_clean() {
            self.run_cleanup(ctx, false);
        }
    }

    fn run_cleanup(&mut self, ctx: &egui::Context, preview: bool) {
        let items = if preview {
            self.collection.clone()
        } else {
            self.confirm_open = false;
            self.collector_open = false;
            std::mem::take(&mut self.collection)
        };
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let mut moved = 0;
            let mut errors = Vec::new();
            #[cfg(target_os = "macos")]
            let trash = {
                use trash::macos::{DeleteMethod, TrashContextExtMacos};
                let mut context = trash::TrashContext::new();
                context.set_delete_method(DeleteMethod::NsFileManager);
                context
            };
            #[cfg(not(target_os = "macos"))]
            let trash = trash::TrashContext::new();
            for item in items {
                let result = verify_target(&item).and_then(|()| {
                    if preview {
                        Ok(())
                    } else {
                        trash.delete(&item.path).map_err(io::Error::other)
                    }
                });
                match result {
                    Ok(()) => moved += 1,
                    Err(error) => errors.push(format!(
                        "{} · {error}",
                        disk_scanner::display_name(
                            item.path.file_name().unwrap_or(item.path.as_os_str()),
                        )
                    )),
                }
            }
            let _ = sender.send((moved, errors));
            ctx.request_repaint();
        });
        self.cleanup = Some(CleanupJob { preview, receiver });
    }

    pub fn reveal(&mut self, id: NodeId, ctx: &egui::Context) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let path = snapshot.path(id);
        let folder = matches!(snapshot.nodes[id].kind, Kind::Folder | Kind::OtherVolume);
        let (sender, receiver) = mpsc::channel();
        self.finder = Some(receiver);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let mut command = {
                #[cfg(target_os = "macos")]
                {
                    let mut command = std::process::Command::new("/usr/bin/open");
                    if folder {
                        command.args(["-a", "Finder"]);
                    } else {
                        command.arg("-R");
                    }
                    command.arg(&path);
                    command
                }
                #[cfg(target_os = "windows")]
                {
                    let mut command = std::process::Command::new("explorer.exe");
                    if !folder {
                        command.arg("/select,");
                    }
                    command.arg(&path);
                    command
                }
                #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                {
                    let mut command = std::process::Command::new("xdg-open");
                    command.arg(if folder {
                        &path
                    } else {
                        path.parent().unwrap_or(&path)
                    });
                    command
                }
            };
            let result = command.status().and_then(|status| {
                if status.success() {
                    Ok(())
                } else {
                    Err(io::Error::other("The folder or file may have moved"))
                }
            });
            let _ = sender.send(result);
            ctx.request_repaint();
        });
    }
}

fn protected(path: &Path) -> bool {
    if path.parent().is_none() {
        return true;
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            let home = Path::new(&home);
            if path == home
                || path
                    .strip_prefix("/System/Volumes/Data")
                    .is_ok_and(|relative| relative == home.strip_prefix("/").unwrap_or(home))
            {
                return true;
            }
        }
        if path.starts_with("/System") {
            return !path.starts_with("/System/Volumes/Data/Users")
                || path == Path::new("/System/Volumes/Data/Users");
        }
        [
            "/bin",
            "/sbin",
            "/usr",
            "/Library",
            "/Applications",
            "/private/etc",
            "/private/var/db",
            "/Volumes/Recovery",
        ]
        .into_iter()
        .any(|prefix| path.starts_with(prefix))
    }
    #[cfg(not(target_os = "macos"))]
    false
}

fn verify_target(item: &Collected) -> io::Result<()> {
    let path = &item.path;
    let metadata = std::fs::symlink_metadata(path)?;
    if protected(path)
        || std::fs::canonicalize(path)? != *path
        || Identity::of(&metadata) != item.identity
        || !(item.kind == Kind::Folder && metadata.is_dir()
            || item.kind == Kind::File && metadata.is_file())
    {
        return Err(io::Error::other(
            "Item changed since the scan or is protected",
        ));
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt;
        // Restricted, immutable and append-only entries require explicit system tools.
        const SF_RESTRICTED: u32 = 0x0008_0000;
        const PROTECTED_FLAGS: u32 = SF_RESTRICTED
            | libc::SF_IMMUTABLE
            | libc::SF_APPEND
            | libc::UF_IMMUTABLE
            | libc::UF_APPEND;
        if metadata.st_flags() & PROTECTED_FLAGS != 0
            || metadata.st_uid() != unsafe { libc::geteuid() }
        {
            return Err(io::Error::other(
                "Item is protected or owned by another user",
            ));
        }
    }
    Ok(())
}

impl Drop for DiskAnalyzerApp {
    fn drop(&mut self) {
        self.cache_sender.take();
        if let Some(job) = self.export.take() {
            let _ = job.worker.join();
        }
        if let Some(writer) = self.cache_writer.take() {
            let _ = writer.join();
        }
    }
}

impl eframe::App for DiskAnalyzerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);
        ui::render(self, ctx);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::disk_scanner::tests::Fixture;

    #[test]
    fn rejects_a_collected_item_replaced_after_scan() {
        let fixture = Fixture::new();
        fixture.write("target", 8192);
        let path = fixture.0.join("target");
        let metadata = std::fs::symlink_metadata(&path).unwrap();
        let item = Collected {
            path: path.clone(),
            size: 8192,
            kind: Kind::File,
            identity: Identity::of(&metadata),
        };
        verify_target(&item).unwrap();
        std::fs::rename(&path, fixture.0.join("original")).unwrap();
        fixture.write("target", 8192);
        assert!(verify_target(&item).is_err());
    }

    #[test]
    fn dry_run_checks_targets_and_leaves_them_in_place() {
        let fixture = Fixture::new();
        fixture.write("target", 8192);
        let ctx = egui::Context::default();
        let mut app = DiskAnalyzerApp::new(&ctx, None);
        app.snapshot = Some(Arc::new(
            disk_scanner::scan(&fixture.0, &ScanControl::default()).unwrap(),
        ));
        app.collect(1);
        app.preview_cleanup(&ctx);
        let until = Instant::now() + std::time::Duration::from_secs(3);
        while app.busy() && Instant::now() < until {
            app.poll(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(
            app.status.as_ref().unwrap(),
            &("Dry run · 1 item(s) eligible for Trash.".into(), false)
        );
        assert!(fixture.0.join("target").is_file());
        assert_eq!(app.collection.len(), 1);
    }

    #[test]
    fn last_scan_reopens_after_relaunch_and_navigation_preserves_folder_history() {
        let fixture = Fixture::new();
        fixture.write("folder/subfolder/file.txt", 8192);
        let cache_fixture = Fixture::new();
        let cache = Cache::for_test(cache_fixture.0.clone());
        let ctx = egui::Context::default();
        let mut app = DiskAnalyzerApp::new(&ctx, None);
        app.cache = Some(cache.clone());
        app.navigate(Page::Disks);
        app.start_scan(fixture.0.clone(), &ctx);
        finish_job(&mut app, &ctx);
        assert_eq!(app.page(), ROOT_MAP);
        app.back();
        assert_eq!(app.page(), Page::Disks);
        app.forward();
        app.go_to(1);
        app.navigate(Page::Home);
        app.back();
        assert_eq!(app.focus(), 1);
        app.forward();
        assert_eq!(app.page(), Page::Home);
        app.open_last_scan(&ctx);
        assert_eq!(app.focus(), 1);
        drop(app);

        let mut reopened = DiskAnalyzerApp::new(&ctx, None);
        reopened.last_scan = cache.summary().ok();
        reopened.cache = Some(cache);
        reopened.open_last_scan(&ctx);
        finish_job(&mut reopened, &ctx);
        assert_eq!(reopened.page(), ROOT_MAP);
        assert!(reopened.cached_snapshot);
        reopened.toggle_mark(1);
        assert!(reopened.collection.is_empty());
    }

    #[test]
    fn marking_a_parent_coalesces_descendants_and_can_be_undone() {
        let fixture = Fixture::new();
        fixture.write("parent/subfolder/file.txt", 8192);
        let ctx = egui::Context::default();
        let mut app = DiskAnalyzerApp::new(&ctx, None);
        app.snapshot = Some(Arc::new(
            disk_scanner::scan(&fixture.0, &ScanControl::default()).unwrap(),
        ));
        app.toggle_mark(3);
        app.toggle_mark(2);
        assert_eq!(app.collection.len(), 1);
        assert_eq!(app.collection[0].path, fixture.0.join("parent/subfolder"));
        app.toggle_mark(2);
        assert!(app.collection.is_empty());
        assert!(fixture.0.join("parent/subfolder/file.txt").exists());
    }

    fn finish_job(app: &mut DiskAnalyzerApp, ctx: &egui::Context) {
        let until = Instant::now() + std::time::Duration::from_secs(3);
        while app.busy() && Instant::now() < until {
            app.poll(ctx);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!app.busy());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn protects_system_locations_from_cleanup() {
        assert!(protected(Path::new("/System/Library")));
        assert!(protected(Path::new("/System/Volumes/Data/private")));
        assert!(!protected(Path::new(
            "/System/Volumes/Data/Users/example/Downloads/test"
        )));
        assert!(!protected(Path::new("/Volumes/External/Photos")));
    }
}
