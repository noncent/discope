use super::{DiskUsage, Identity, Kind, Node, ScanControl};
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime};

const VERSION: &[u8; 8] = b"DSCOPE01";
const RECORD_BYTES: u64 = 25;
const MAX_NODES: usize = 16_000_000;
const MAX_PATH_BYTES: usize = 128 * 1024;

#[derive(Clone)]
pub struct Summary {
    pub root: PathBuf,
    pub size: u64,
    pub files: usize,
    pub folders: usize,
    pub unreadable: usize,
    pub skipped_volumes: usize,
    pub elapsed: Duration,
}

impl Summary {
    pub fn from_usage(usage: &DiskUsage) -> Self {
        Self {
            root: usage.root.clone(),
            size: usage.size(),
            files: usage.files,
            folders: usage.folders,
            unreadable: usage.unreadable,
            skipped_volumes: usage.skipped_volumes,
            elapsed: usage.elapsed,
        }
    }
}

#[derive(Clone)]
pub struct Cache(PathBuf);

impl Cache {
    #[cfg(test)]
    pub fn for_test(path: PathBuf) -> Self {
        Self(path)
    }

    pub fn local() -> Option<Self> {
        #[cfg(test)]
        return None;
        #[cfg(not(test))]
        {
            let root = if cfg!(target_os = "macos") {
                PathBuf::from(std::env::var_os("HOME")?).join("Library/Caches")
            } else if cfg!(windows) {
                PathBuf::from(std::env::var_os("LOCALAPPDATA")?)
            } else {
                std::env::var_os("XDG_CACHE_HOME")
                    .map(PathBuf::from)
                    .filter(|p| p.is_absolute())
                    .or_else(|| {
                        std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache"))
                    })?
            };
            Some(Self(root.join("com.noncent.discope")))
        }
    }

    pub fn dark(&self) -> bool {
        let mut value = [0];
        File::open(self.0.join("appearance"))
            .and_then(|mut file| file.read_exact(&mut value))
            .is_ok()
            && value == [1]
    }

    pub fn save_appearance(&self, dark: bool) -> io::Result<()> {
        self.replace("appearance", |file| file.write_all(&[u8::from(dark)]))
    }

    pub fn summary(&self) -> io::Result<Summary> {
        let mut file = BufReader::new(File::open(self.0.join("last-scan"))?);
        header(&mut file).map(|(summary, _)| summary)
    }

    pub fn save(&self, usage: &DiskUsage) -> io::Result<()> {
        if usage.nodes.len() > MAX_NODES {
            return Err(io::Error::other(
                "This scan exceeds the saved result size limit",
            ));
        }
        self.replace("last-scan", |file| {
            let mut writer = BufWriter::with_capacity(256 * 1024, file);
            writer.write_all(VERSION)?;
            write_name(&mut writer, usage.root.as_os_str())?;
            for value in [
                usage.size(),
                usage.files as u64,
                usage.folders as u64,
                usage.unreadable as u64,
                usage.skipped_volumes as u64,
                usage.elapsed.as_millis() as u64,
                usage.nodes.len() as u64,
            ] {
                writer.write_all(&value.to_le_bytes())?;
            }
            for node in &usage.nodes {
                writer.write_all(&node.size.to_le_bytes())?;
                for index in [node.parent, node.first_child, node.child_count] {
                    writer.write_all(&(index as u32).to_le_bytes())?;
                }
                let kind = match node.kind {
                    Kind::Folder => 0,
                    Kind::File => 1,
                    Kind::Unreadable => 2,
                    Kind::OtherVolume => 3,
                };
                writer.write_all(&[kind | (u8::from(node.hidden) << 2)])?;
                write_name(&mut writer, &node.name)?;
            }
            writer.flush()
        })
    }

    pub fn load(&self, control: &ScanControl) -> io::Result<DiskUsage> {
        let file = File::open(self.0.join("last-scan"))?;
        let length = file.metadata()?.len();
        let mut reader = BufReader::with_capacity(256 * 1024, file);
        let (summary, count) = header(&mut reader)?;
        if length < count as u64 * RECORD_BYTES {
            return Err(invalid());
        }
        let mut nodes = Vec::<Node>::new();
        let mut name_bytes = Vec::new();
        for id in 0..count {
            if id % 4096 == 0 {
                control.check()?;
            }
            let size = number(&mut reader)?;
            let parent = index(&mut reader)?;
            let first_child = index(&mut reader)?;
            let child_count = index(&mut reader)?;
            let mut flags = [0];
            reader.read_exact(&mut flags)?;
            let kind = match flags[0] & 3 {
                0 => Kind::Folder,
                1 => Kind::File,
                2 => Kind::Unreadable,
                _ => Kind::OtherVolume,
            };
            let name = read_name(&mut reader, &mut name_bytes)?;
            let mut components = Path::new(&name).components();
            if first_child > count
                || child_count > count - first_child
                || (child_count > 0 && (first_child <= id || kind != Kind::Folder))
                || (id == 0 && (kind != Kind::Folder || parent != 0 || size != summary.size))
                || (id > 0
                    && (!matches!(components.next(), Some(Component::Normal(value)) if value == name)
                        || components.next().is_some()
                        || parent >= id
                        || !(nodes[parent].first_child
                            ..nodes[parent].first_child + nodes[parent].child_count)
                            .contains(&id)))
            {
                return Err(invalid());
            }
            nodes.push(Node {
                name,
                size,
                kind,
                identity: Identity::default(),
                hidden: flags[0] & 4 != 0,
                complete: false,
                parent,
                first_child,
                child_count,
            });
        }
        for (id, node) in nodes.iter().enumerate() {
            if id % 4096 == 0 {
                control.check()?;
            }
            if node.kind == Kind::Folder {
                let mut previous = u64::MAX;
                let mut total = 0_u64;
                for child in &nodes[node.first_child..node.first_child + node.child_count] {
                    if child.parent != id || child.size > previous {
                        return Err(invalid());
                    }
                    total = total.checked_add(child.size).ok_or_else(invalid)?;
                    previous = child.size;
                }
                if node.size != total {
                    return Err(invalid());
                }
            }
        }
        Ok(DiskUsage {
            root: summary.root,
            nodes,
            files: summary.files,
            folders: summary.folders,
            unreadable: summary.unreadable,
            skipped_volumes: summary.skipped_volumes,
            elapsed: summary.elapsed,
        })
    }

    fn replace(
        &self,
        name: &str,
        write: impl FnOnce(&mut File) -> io::Result<()>,
    ) -> io::Result<()> {
        let mut directory = fs::DirBuilder::new();
        directory.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            directory.mode(0o700);
        }
        directory.create(&self.0)?;
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let temporary = self
            .0
            .join(format!(".{name}-{}-{timestamp}", std::process::id()));
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        let result = write(&mut file).and_then(|()| file.sync_all());
        drop(file);
        let result = result.and_then(|()| fs::rename(&temporary, self.0.join(name)));
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }
}

fn header(reader: &mut impl Read) -> io::Result<(Summary, usize)> {
    let mut version = [0; 8];
    reader.read_exact(&mut version)?;
    if &version != VERSION {
        return Err(invalid());
    }
    let root = PathBuf::from(read_name(reader, &mut Vec::new())?);
    let size = number(reader)?;
    let files = number(reader)? as usize;
    let folders = number(reader)? as usize;
    let unreadable = number(reader)? as usize;
    let skipped_volumes = number(reader)? as usize;
    let elapsed = Duration::from_millis(number(reader)?);
    let count = number(reader)?;
    if !root.is_absolute() || !(1..=MAX_NODES as u64).contains(&count) {
        return Err(invalid());
    }
    Ok((
        Summary {
            root,
            size,
            files,
            folders,
            unreadable,
            skipped_volumes,
            elapsed,
        },
        count as usize,
    ))
}

fn number(reader: &mut impl Read) -> io::Result<u64> {
    let mut bytes = [0; 8];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

fn index(reader: &mut impl Read) -> io::Result<usize> {
    let mut bytes = [0; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes) as usize)
}

fn read_name(reader: &mut impl Read, bytes: &mut Vec<u8>) -> io::Result<OsString> {
    let length = index(reader)?;
    if length > MAX_PATH_BYTES {
        return Err(invalid());
    }
    bytes.resize(length, 0);
    reader.read_exact(bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        if bytes.contains(&0) {
            return Err(invalid());
        }
        Ok(OsStr::from_bytes(bytes).to_owned())
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        if length % 2 != 0 {
            return Err(invalid());
        }
        let wide: Vec<_> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        if wide.contains(&0) {
            return Err(invalid());
        }
        Ok(OsString::from_wide(&wide))
    }
}

fn write_name(writer: &mut impl Write, name: &OsStr) -> io::Result<()> {
    #[cfg(unix)]
    let bytes = {
        use std::os::unix::ffi::OsStrExt;
        name.as_bytes()
    };
    #[cfg(windows)]
    let bytes = {
        use std::os::windows::ffi::OsStrExt;
        name.encode_wide()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>()
    };
    if bytes.len() > MAX_PATH_BYTES {
        return Err(invalid());
    }
    writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
    writer.write_all(bytes.as_ref())
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "The saved scan could not be read",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk_scanner::{scan, tests::Fixture};

    #[test]
    fn restores_a_complete_index_without_accessing_the_original_folder() {
        let folder = Fixture::new();
        folder.write(".nested/file.txt", 8192);
        folder.write("root.txt", 4096);
        let cache_folder = Fixture::new();
        let cache = Cache::for_test(cache_folder.0.clone());
        let usage = scan(&folder.0, &ScanControl::default()).unwrap();
        cache.save(&usage).unwrap();
        let summary = cache.summary().unwrap();
        assert_eq!(summary.files, 2);
        assert_eq!(summary.size, usage.size());
        let path = folder.0.clone();
        drop(folder);

        let restored = cache.load(&ScanControl::default()).unwrap();
        assert_eq!(restored.root, path);
        assert_eq!(restored.nodes.len(), usage.nodes.len());
        for (index, node) in restored.nodes.iter().enumerate() {
            assert_eq!(restored.path(index), usage.path(index));
            assert_eq!(node.hidden, usage.nodes[index].hidden);
            assert!(!node.complete);
        }
        assert!(!cache.dark());
        cache.save_appearance(true).unwrap();
        assert!(cache.dark());
    }

    #[test]
    fn rejects_a_truncated_result() {
        let folder = Fixture::new();
        folder.write("folder/file", 4096);
        let cache_folder = Fixture::new();
        let cache = Cache::for_test(cache_folder.0.clone());
        cache
            .save(&scan(&folder.0, &ScanControl::default()).unwrap())
            .unwrap();
        let file = OpenOptions::new()
            .write(true)
            .open(cache_folder.0.join("last-scan"))
            .unwrap();
        file.set_len(file.metadata().unwrap().len() - 1).unwrap();
        assert!(cache.load(&ScanControl::default()).is_err());
    }
}
