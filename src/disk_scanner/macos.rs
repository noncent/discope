use super::{Directory, Entry, Identity, Job, Kind, ScanControl, Volume};
use libc::*;
use std::ffi::{CStr, OsStr};
use std::fs::OpenOptions;
use std::io;
use std::mem::{size_of, MaybeUninit};
use std::os::fd::AsRawFd;
use std::os::unix::{ffi::OsStrExt, fs::OpenOptionsExt};
use std::path::PathBuf;

const ATTR_CMN_ERROR: u32 = 0x2000_0000;
const SF_FIRMLINK: u32 = 0x0080_0000;
pub(super) const SF_DATALESS: u32 = 0x4000_0000;

const IOPOL_SCOPE_THREAD: c_int = 1;
const IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES: c_int = 3;
const IOPOL_TYPE_VFS_TRIGGER_RESOLVE: c_int = 5;
const IOPOL_OFF: c_int = 1;

extern "C" {
    fn getiopolicy_np(iotype: c_int, scope: c_int) -> c_int;
    fn setiopolicy_np(iotype: c_int, scope: c_int, policy: c_int) -> c_int;
}

// Apple TN3150 describes opting out of File Provider downloads on the scanning thread.
pub(super) struct LocalIo([(c_int, c_int); 2]);

impl LocalIo {
    pub fn enter() -> Self {
        let policies = [
            IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES,
            IOPOL_TYPE_VFS_TRIGGER_RESOLVE,
        ];
        Self(policies.map(|policy| {
            // Both calls operate on this thread and take only Darwin enum values.
            let previous = unsafe { getiopolicy_np(policy, IOPOL_SCOPE_THREAD) };
            if previous >= 0 {
                unsafe { setiopolicy_np(policy, IOPOL_SCOPE_THREAD, IOPOL_OFF) };
            }
            (policy, previous)
        }))
    }
}

impl Drop for LocalIo {
    fn drop(&mut self) {
        for (policy, previous) in self.0 {
            if previous >= 0 {
                unsafe { setiopolicy_np(policy, IOPOL_SCOPE_THREAD, previous) };
            }
        }
    }
}

const VREG: u32 = 1;
const VDIR: u32 = 2;

#[derive(Default)]
pub(super) struct BulkReader {
    buffer: Vec<u8>,
}

impl BulkReader {
    pub fn read(&mut self, job: &Job, control: &ScanControl) -> io::Result<Option<Directory>> {
        let fd = OpenOptions::new()
            .read(true)
            .custom_flags(O_DIRECTORY | O_NOFOLLOW_ANY | O_NONBLOCK)
            .open(&job.path)?;
        let identity = Identity::of(&fd.metadata()?);
        let mut directory = Directory {
            entries: Vec::new(),
            unreadable: 0,
            kind: Kind::Folder,
        };
        if identity.device != job.identity.device {
            directory.kind = Kind::OtherVolume;
            return Ok(Some(directory));
        }
        if identity.inode != job.identity.inode {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Folder changed during scan",
            ));
        }
        let mut attributes = attrlist {
            bitmapcount: ATTR_BIT_MAP_COUNT,
            reserved: 0,
            commonattr: ATTR_CMN_RETURNED_ATTRS
                | ATTR_CMN_ERROR
                | ATTR_CMN_NAME
                | ATTR_CMN_OBJTYPE
                | ATTR_CMN_FLAGS
                | ATTR_CMN_FILEID,
            volattr: 0,
            dirattr: ATTR_DIR_MOUNTSTATUS,
            fileattr: ATTR_FILE_LINKCOUNT | ATTR_FILE_ALLOCSIZE,
            forkattr: 0,
        };
        if self.buffer.is_empty() {
            self.buffer.resize(128 * 1024, 0);
        }

        loop {
            control.check()?;
            // The kernel writes packed, length-prefixed records into this owned buffer.
            let count = unsafe {
                getattrlistbulk(
                    fd.as_raw_fd(),
                    (&mut attributes as *mut attrlist).cast(),
                    self.buffer.as_mut_ptr().cast(),
                    self.buffer.len(),
                    0,
                )
            };
            if count < 0 {
                let error = io::Error::last_os_error();
                if matches!(error.raw_os_error(), Some(ENOTSUP | EINVAL | ENOSYS)) {
                    return Ok(None);
                }
                if directory.entries.is_empty() {
                    return Err(error);
                }
                directory.unreadable += 1;
                break;
            }
            if count == 0 {
                break;
            }
            let mut records = self.buffer.as_slice();
            for _ in 0..count {
                let len = u32_at(records, 0).unwrap_or(0) as usize;
                if len < 24 || len > records.len() {
                    return Ok(None);
                }
                let Some(entry) = decode(&records[..len], identity.device) else {
                    return Ok(None);
                };
                if let Some(entry) = entry {
                    directory.unreadable += usize::from(entry.kind == Kind::Unreadable);
                    directory.entries.push(entry);
                }
                records = &records[len..];
            }
        }

        Ok(Some(directory))
    }
}

fn u32_at(record: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_ne_bytes(
        record.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn decode(record: &[u8], device: u64) -> Option<Option<Entry>> {
    let common = u32_at(record, 4)?;
    let directories = u32_at(record, 12)?;
    let files = u32_at(record, 16)?;
    let mut offset = 24;

    // ATTR_CMN_ERROR, if present, immediately follows the returned attribute bitmap.
    let error = if common & ATTR_CMN_ERROR != 0 {
        take_u32(record, &mut offset)?
    } else {
        0
    };
    if common & ATTR_CMN_NAME == 0 {
        return None;
    }
    let name_start = offset.checked_add_signed(u32_at(record, offset)? as i32 as isize)?;
    let name_len = u32_at(record, offset + 4)? as usize;
    let name = record
        .get(name_start..name_start.checked_add(name_len)?)?
        .strip_suffix(&[0])?;
    if name == b"." || name == b".." || name.is_empty() || name.contains(&b'/') || name.contains(&0)
    {
        return None;
    }
    offset += 8;
    if error != 0 {
        return Some(Some(Entry::new(
            OsStr::from_bytes(name).to_owned(),
            Kind::Unreadable,
            Identity::default(),
        )));
    }

    if common & (ATTR_CMN_OBJTYPE | ATTR_CMN_FILEID) != (ATTR_CMN_OBJTYPE | ATTR_CMN_FILEID) {
        return None;
    }
    let file_type = take_u32(record, &mut offset)?;
    if file_type != VDIR && file_type != VREG {
        return Some(None);
    }
    let flags = if common & ATTR_CMN_FLAGS != 0 {
        take_u32(record, &mut offset)?
    } else {
        0
    };
    let inode = u64::from_ne_bytes(record.get(offset..offset + 8)?.try_into().ok()?);
    offset += 8;
    if inode == 0 {
        return None;
    }
    let identity = Identity { device, inode };
    let kind = if flags & SF_DATALESS != 0 {
        Kind::Unreadable
    } else if file_type == VDIR {
        let mount = directories & ATTR_DIR_MOUNTSTATUS != 0
            && take_u32(record, &mut offset)? & DIR_MNTSTATUS_MNTPOINT != 0;
        if mount || flags & SF_FIRMLINK != 0 {
            Kind::OtherVolume
        } else {
            Kind::Folder
        }
    } else {
        Kind::File
    };
    let mut entry = Entry::new(OsStr::from_bytes(name).to_owned(), kind, identity);
    entry.hidden |= flags & UF_HIDDEN != 0;
    if kind == Kind::File {
        if files & (ATTR_FILE_LINKCOUNT | ATTR_FILE_ALLOCSIZE)
            != (ATTR_FILE_LINKCOUNT | ATTR_FILE_ALLOCSIZE)
        {
            return None;
        }
        entry.links = take_u32(record, &mut offset)?;
        entry.size =
            i64::from_ne_bytes(record.get(offset..offset + 8)?.try_into().ok()?).max(0) as u64;
    }
    Some(Some(entry))
}

fn take_u32(record: &[u8], offset: &mut usize) -> Option<u32> {
    let value = u32_at(record, *offset)?;
    *offset += 4;
    Some(value)
}

pub fn volumes() -> Vec<Volume> {
    // MNT_NOWAIT uses cached mount information, including when a network volume is slow.
    let count = unsafe { getfsstat(std::ptr::null_mut(), 0, MNT_NOWAIT) };
    if count <= 0 {
        return Vec::new();
    }
    let mut mounts = Vec::<MaybeUninit<statfs>>::with_capacity(count as usize + 16);
    let bytes = (mounts.capacity() * size_of::<statfs>()) as c_int;
    let filled = unsafe { getfsstat(mounts.as_mut_ptr().cast(), bytes, MNT_NOWAIT) };
    if filled < 0 || filled as usize > mounts.capacity() {
        return Vec::new();
    }
    // getfsstat returns the number of initialized records when supplied a buffer.
    unsafe { mounts.set_len(filled as usize) };

    let mut volumes: Vec<_> = mounts
        .into_iter()
        .filter_map(|mount| {
            let mount = unsafe { mount.assume_init() };
            let path = PathBuf::from(OsStr::from_bytes(
                unsafe { CStr::from_ptr(mount.f_mntonname.as_ptr()) }.to_bytes(),
            ));
            let filesystem =
                unsafe { CStr::from_ptr(mount.f_fstypename.as_ptr()) }.to_string_lossy();
            let startup = path == std::path::Path::new("/System/Volumes/Data");
            let macos = path == std::path::Path::new("/");
            let auxiliary = !startup
                && (path.starts_with("/System/Volumes")
                    || path.starts_with("/Volumes") && mount.f_flags & MNT_DONTBROWSE as u32 != 0);
            if filesystem == "autofs"
                || !(startup || macos || auxiliary || path.starts_with("/Volumes"))
            {
                return None;
            }
            let (name, role) = if startup {
                ("Startup Data".into(), "Startup disk · user and shared data")
            } else if macos {
                ("macOS System".into(), "Startup disk · system volume")
            } else if auxiliary {
                (
                    super::display_name(path.file_name()?),
                    "System-managed volume",
                )
            } else if mount.f_flags & MNT_LOCAL as u32 != 0 {
                (super::display_name(path.file_name()?), "Mounted disk")
            } else {
                (super::display_name(path.file_name()?), "Network volume")
            };
            let read_only = mount.f_flags & MNT_RDONLY as u32 != 0;
            let capacity = mount.f_blocks.saturating_mul(mount.f_bsize as u64);
            let available = mount.f_bavail.saturating_mul(mount.f_bsize as u64);
            Some(Volume {
                name,
                path,
                detail: format!(
                    "{}  ·  {}{}",
                    role,
                    filesystem.to_uppercase(),
                    if read_only { "  ·  Read-only" } else { "" }
                ),
                space: (capacity > 0 && !(macos && filesystem == "apfs") && !auxiliary)
                    .then_some((capacity, available.min(capacity))),
                system: auxiliary,
            })
        })
        .collect();
    volumes.sort_by_key(|volume| match volume.path.to_str() {
        Some("/System/Volumes/Data") => (0, String::new()),
        Some("/") => (1, String::new()),
        _ => (
            if volume.system { 3 } else { 2 },
            volume.name.to_lowercase(),
        ),
    });
    volumes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_opt_out_of_cloud_downloads_and_restore_the_thread_policy() {
        let policy = IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES;
        let original = unsafe { getiopolicy_np(policy, IOPOL_SCOPE_THREAD) };
        assert!(original >= 0);
        {
            let _local = LocalIo::enter();
            let active = unsafe { getiopolicy_np(policy, IOPOL_SCOPE_THREAD) };
            assert_eq!(active & 3, IOPOL_OFF);
        }
        assert_eq!(
            unsafe { getiopolicy_np(policy, IOPOL_SCOPE_THREAD) } & 3,
            original & 3
        );
    }
}
