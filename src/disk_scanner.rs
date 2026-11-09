use std::collections::{HashSet, VecDeque};
use std::ffi::{OsStr, OsString};
use std::fs::{self, Metadata};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed},
    Condvar, Mutex,
};
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::volumes;

pub mod cache;

pub type NodeId = usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Folder,
    File,
    Unreadable,
    OtherVolume,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Identity {
    device: u64,
    inode: u64,
}

impl Identity {
    pub fn of(metadata: &Metadata) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Self {
                device: metadata.dev(),
                inode: metadata.ino(),
            }
        }
        #[cfg(not(unix))]
        {
            let _ = metadata;
            Self::default()
        }
    }
}

pub struct Node {
    pub name: OsString,
    pub size: u64,
    pub kind: Kind,
    pub identity: Identity,
    pub hidden: bool,
    pub complete: bool,
    pub parent: NodeId,
    first_child: NodeId,
    child_count: usize,
}

pub struct DiskUsage {
    pub root: PathBuf,
    pub nodes: Vec<Node>,
    pub files: usize,
    pub folders: usize,
    pub unreadable: usize,
    pub skipped_volumes: usize,
    pub elapsed: Duration,
}

impl DiskUsage {
    pub fn children(&self, id: NodeId) -> std::ops::Range<NodeId> {
        let node = &self.nodes[id];
        node.first_child..node.first_child + node.child_count
    }

    pub fn path(&self, mut id: NodeId) -> PathBuf {
        let mut names = Vec::new();
        while id != 0 {
            let node = &self.nodes[id];
            names.push(&node.name);
            id = node.parent;
        }
        let mut path = self.root.clone();
        path.extend(names.into_iter().rev());
        path
    }

    pub fn size(&self) -> u64 {
        self.nodes[0].size
    }
}

pub struct Volume {
    pub name: String,
    pub path: PathBuf,
    pub detail: String,
    pub space: Option<(u64, u64)>,
    pub system: bool,
}

#[cfg(not(target_os = "macos"))]
pub fn volumes() -> Vec<Volume> {
    Vec::new()
}

#[derive(Default)]
pub struct ScanControl {
    pub cancelled: AtomicBool,
    pub visited: AtomicUsize,
}

impl ScanControl {
    fn check(&self) -> io::Result<()> {
        if self.cancelled.load(Relaxed) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "Scan cancelled"));
        }
        Ok(())
    }
}

struct Entry {
    name: OsString,
    size: u64,
    identity: Identity,
    kind: Kind,
    hidden: bool,
    complete: bool,
    links: u32,
    directory: Option<usize>,
}

impl Entry {
    fn new(name: OsString, kind: Kind, identity: Identity) -> Self {
        let hidden = name.as_encoded_bytes().starts_with(b".");
        Self {
            name,
            kind,
            identity,
            hidden,
            complete: matches!(kind, Kind::Folder | Kind::File),
            size: 0,
            links: 1,
            directory: None,
        }
    }

    fn node(self, parent: NodeId) -> Node {
        Node {
            name: self.name,
            size: self.size,
            kind: self.kind,
            identity: self.identity,
            hidden: self.hidden,
            complete: self.complete,
            parent,
            first_child: 0,
            child_count: 0,
        }
    }
}

struct Job {
    id: usize,
    path: PathBuf,
    identity: Identity,
}

struct Directory {
    entries: Vec<Entry>,
    unreadable: usize,
    kind: Kind,
}

#[derive(Default)]
struct Queue {
    jobs: VecDeque<Job>,
    pending: usize,
    next: usize,
}

struct Scan<'a> {
    control: &'a ScanControl,
    queue: Mutex<Queue>,
    ready: Condvar,
    hard_links: Mutex<HashSet<Identity>>,
}

impl Scan<'_> {
    fn work(&self) -> io::Result<Vec<(usize, Directory)>> {
        #[cfg(target_os = "macos")]
        let _local_io = macos::LocalIo::enter();
        let mut directories = Vec::new();
        let mut reader = DirectoryReader::default();
        loop {
            let job = {
                let mut queue = self.queue.lock().unwrap();
                loop {
                    self.control.check()?;
                    if let Some(job) = queue.jobs.pop_back() {
                        break job;
                    }
                    if queue.pending == 0 {
                        return Ok(directories);
                    }
                    queue = self.ready.wait(queue).unwrap();
                }
            };

            let directory = reader.read(&job, self.control);
            let mut directory = match directory {
                Ok(directory) => directory,
                Err(error) if job.id == 0 => {
                    let _queue = self.queue.lock().unwrap();
                    self.control.cancelled.store(true, Relaxed);
                    self.ready.notify_all();
                    return Err(error);
                }
                Err(_) => Directory {
                    entries: Vec::new(),
                    unreadable: 1,
                    kind: Kind::Unreadable,
                },
            };
            self.control
                .visited
                .fetch_add(directory.entries.len(), Relaxed);

            for entry in &mut directory.entries {
                if entry.links > 1
                    && entry.identity.inode != 0
                    && !self.hard_links.lock().unwrap().insert(entry.identity)
                {
                    entry.size = 0;
                }
            }

            let jobs = directory
                .entries
                .iter_mut()
                .filter(|entry| entry.kind == Kind::Folder)
                .map(|entry| (job.path.join(&entry.name), entry));
            let mut queue = self.queue.lock().unwrap();
            if self.control.cancelled.load(Relaxed) {
                self.ready.notify_all();
                self.control.check()?;
            }
            let was_empty = queue.jobs.is_empty();
            for (path, entry) in jobs {
                let id = queue.next;
                queue.next += 1;
                queue.pending += 1;
                entry.directory = Some(id);
                queue.jobs.push_back(Job {
                    id,
                    path,
                    identity: entry.identity,
                });
            }
            queue.pending -= 1;
            if queue.pending == 0 || (was_empty && !queue.jobs.is_empty()) {
                self.ready.notify_all();
            }
            drop(queue);
            directories.push((job.id, directory));
        }
    }
}

pub fn scan(path: &Path, control: &ScanControl) -> io::Result<DiskUsage> {
    let start = Instant::now();
    #[cfg(target_os = "macos")]
    let _local_io = macos::LocalIo::enter();
    let root = fs::canonicalize(path)?;
    let metadata = fs::metadata(&root)?;
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Choose a folder or volume",
        ));
    }
    let identity = Identity::of(&metadata);
    let scan = Scan {
        control,
        queue: Mutex::new(Queue {
            jobs: VecDeque::from([Job {
                id: 0,
                path: root.clone(),
                identity,
            }]),
            pending: 1,
            next: 1,
        }),
        ready: Condvar::new(),
        hard_links: Mutex::new(HashSet::new()),
    };
    let worker_count = std::thread::available_parallelism().map_or(1, |count| count.get().min(6));
    let results = std::thread::scope(|scope| {
        let threads: Vec<_> = (1..worker_count)
            .map(|_| scope.spawn(|| scan.work()))
            .collect();
        let mut results = vec![scan.work()];
        results.extend(threads.into_iter().map(|thread| thread.join().unwrap()));
        results
    });
    // Prefer the original I/O error when other workers stopped as a result.
    if let Some(error) = results
        .iter()
        .filter_map(|result| result.as_ref().err())
        .find(|error| error.kind() != io::ErrorKind::Interrupted)
    {
        return Err(io::Error::new(error.kind(), error.to_string()));
    }
    control.check()?;

    let directory_count = scan.queue.into_inner().unwrap().next;
    let mut directories: Vec<Option<Directory>> = (0..directory_count).map(|_| None).collect();
    for result in results {
        for (index, directory) in result? {
            directories[index] = Some(directory);
        }
    }
    let mut totals = vec![0_u64; directory_count];
    let mut complete = vec![true; directory_count];
    let (mut files, mut unreadable, mut skipped_volumes) = (0, 0, 0);
    for index in (0..directory_count).rev() {
        control.check()?;
        let (remaining, current) = directories.split_at_mut(index + 1);
        let directory = remaining[index].as_mut().unwrap();
        unreadable += directory.unreadable;
        complete[index] = directory.unreadable == 0 && directory.kind == Kind::Folder;
        for entry in &mut directory.entries {
            if let Some(child) = entry.directory {
                entry.size = totals[child];
                entry.complete = complete[child];
                entry.kind = current[child - index - 1].as_ref().unwrap().kind;
            }
            complete[index] &= entry.complete;
            files += usize::from(entry.kind == Kind::File);
            skipped_volumes += usize::from(entry.kind == Kind::OtherVolume);
            totals[index] = totals[index].saturating_add(entry.size);
        }
        directory.entries.sort_unstable_by(|left, right| {
            right
                .size
                .cmp(&left.size)
                .then_with(|| left.name.cmp(&right.name))
        });
    }

    let mut root_node = Entry::new(
        root.file_name().unwrap_or(root.as_os_str()).to_owned(),
        Kind::Folder,
        identity,
    )
    .node(0);
    root_node.size = totals[0];
    root_node.complete = complete[0];
    let mut nodes = Vec::with_capacity(control.visited.load(Relaxed) + 1);
    nodes.push(root_node);
    let mut pending = vec![(0, 0)];
    while let Some((parent, directory)) = pending.pop() {
        control.check()?;
        let entries = directories[directory].take().unwrap().entries;
        nodes[parent].first_child = nodes.len();
        nodes[parent].child_count = entries.len();
        for entry in entries {
            if let Some(child) = entry.directory {
                pending.push((nodes.len(), child));
            }
            nodes.push(entry.node(parent));
        }
    }

    Ok(DiskUsage {
        root,
        nodes,
        files,
        folders: directory_count,
        unreadable,
        skipped_volumes,
        elapsed: start.elapsed(),
    })
}

#[derive(Default)]
struct DirectoryReader {
    #[cfg(target_os = "macos")]
    bulk: macos::BulkReader,
}

impl DirectoryReader {
    fn read(&mut self, job: &Job, control: &ScanControl) -> io::Result<Directory> {
        #[cfg(target_os = "macos")]
        if let Some(directory) = self.bulk.read(job, control)? {
            return Ok(directory);
        }
        read_portable(job, control)
    }
}

fn read_portable(job: &Job, control: &ScanControl) -> io::Result<Directory> {
    let metadata = fs::symlink_metadata(&job.path)?;
    let identity = Identity::of(&metadata);
    let mut directory = Directory {
        entries: Vec::new(),
        unreadable: 0,
        kind: Kind::Folder,
    };
    if identity.device != job.identity.device {
        directory.kind = Kind::OtherVolume;
        return Ok(directory);
    }
    if !metadata.is_dir() || (job.identity.inode != 0 && identity != job.identity) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Folder changed during scan",
        ));
    }

    for (index, result) in fs::read_dir(&job.path)?.enumerate() {
        if index % 256 == 0 {
            control.check()?;
        }
        let Ok(item) = result else {
            directory.unreadable += 1;
            continue;
        };
        let name = item.file_name();
        let entry = match item.metadata() {
            Ok(metadata) => {
                let identity = Identity::of(&metadata);
                #[cfg(target_os = "macos")]
                {
                    use std::os::macos::fs::MetadataExt;
                    if metadata.st_flags() & macos::SF_DATALESS != 0 {
                        directory
                            .entries
                            .push(Entry::new(name, Kind::Unreadable, identity));
                        directory.unreadable += 1;
                        continue;
                    }
                }
                if identity.device != job.identity.device {
                    Entry::new(name, Kind::OtherVolume, identity)
                } else if metadata.is_dir() {
                    Entry::new(name, Kind::Folder, identity)
                } else if metadata.is_file() {
                    let mut entry = Entry::new(name, Kind::File, identity);
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::MetadataExt;
                        entry.size = metadata.blocks().saturating_mul(512);
                        entry.links = metadata.nlink() as u32;
                    }
                    #[cfg(not(unix))]
                    {
                        entry.size = metadata.len();
                    }
                    entry
                } else {
                    continue;
                }
            }
            Err(_) => {
                directory.unreadable += 1;
                Entry::new(name, Kind::Unreadable, Identity::default())
            }
        };
        directory.entries.push(entry);
    }
    Ok(directory)
}

pub fn display_name(name: &OsStr) -> String {
    // Render filesystem names as one line, including names containing bidi overrides.
    name.to_string_lossy()
        .chars()
        .map(|ch| {
            if ch.is_control() || matches!(ch, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') {
                '�'
            } else {
                ch
            }
        })
        .collect()
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub struct Fixture(pub PathBuf);

    impl Fixture {
        pub fn new() -> Self {
            static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
            let name = format!(
                "discope-test-{}-{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Relaxed),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
            );
            let path = std::env::temp_dir().join(name);
            fs::create_dir(&path).unwrap();
            Self(fs::canonicalize(path).unwrap())
        }

        pub fn write(&self, name: &str, size: usize) -> u64 {
            let path = self.0.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, vec![37; size]).unwrap();
            let metadata = fs::metadata(path).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                metadata.blocks() * 512
            }
            #[cfg(not(unix))]
            {
                metadata.len()
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn indexes_hidden_files_and_folders_beyond_the_original_depth_limit() {
        let fixture = Fixture::new();
        let deep = ".hidden/a/b/c/d/e/f/g/h/i/j/content.dat";
        let expected = fixture.write(deep, 8192);
        let usage = scan(&fixture.0, &ScanControl::default()).unwrap();
        assert_eq!(usage.size(), expected);
        assert_eq!(usage.files, 1);
        let hidden = usage.children(0).next().unwrap();
        assert!(usage.nodes[hidden].hidden);
        assert_eq!(usage.nodes[hidden].size, expected);
        let file = usage
            .nodes
            .iter()
            .position(|node| node.kind == Kind::File)
            .unwrap();
        assert_eq!(usage.path(file), fixture.0.join(deep));
    }

    #[cfg(unix)]
    #[test]
    fn ignores_symlink_loops_and_counts_hard_links_once() {
        let fixture = Fixture::new();
        let expected = fixture.write("payload", 16_384);
        fs::hard_link(fixture.0.join("payload"), fixture.0.join("copy")).unwrap();
        std::os::unix::fs::symlink(&fixture.0, fixture.0.join("loop")).unwrap();
        let usage = scan(&fixture.0, &ScanControl::default()).unwrap();
        assert_eq!(usage.files, 2);
        assert_eq!(usage.folders, 1);
        assert_eq!(usage.size(), expected);
        assert_eq!(usage.children(0).len(), 2);
    }

    #[test]
    fn reports_an_error_for_a_missing_or_cancelled_scan() {
        let fixture = Fixture::new();
        let control = ScanControl::default();
        assert_eq!(
            scan(&fixture.0.join("missing"), &control)
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::NotFound
        );
        control.cancelled.store(true, Relaxed);
        assert_eq!(
            scan(&fixture.0, &control).err().unwrap().kind(),
            io::ErrorKind::Interrupted
        );
    }
}
