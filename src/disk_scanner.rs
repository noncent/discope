use std::fs::{self, ReadDir};
use std::path::{Path, PathBuf};

use humansize::{format_size, DECIMAL};

#[derive(Clone, Debug)]
pub struct FolderInfo {
    pub path: PathBuf,
    pub size: u64,
    pub children: Vec<FolderInfo>,
}

#[derive(Clone, Debug)]
pub struct DiskUsage {
    pub total_size: u64,
    pub total_human_size: String,
    pub folders: Vec<FolderInfo>,
}

pub struct DiskScanner;

impl DiskScanner {
    pub fn new() -> Self {
        Self
    }

    pub fn scan_directory_with_depth(
        &self,
        path: &PathBuf,
        max_depth: usize,
    ) -> Result<DiskUsage, Box<dyn std::error::Error>> {
        let (folders, total_size) = self.collect_folders(path.as_path(), 0, max_depth)?;
        let total_human_size = format_size(total_size, DECIMAL);
        Ok(DiskUsage { total_size, total_human_size, folders })
    }

    fn collect_folders(
        &self,
        dir: &Path,
        depth: usize,
        max_depth: usize,
    ) -> Result<(Vec<FolderInfo>, u64), Box<dyn std::error::Error>> {
        let mut folders: Vec<FolderInfo> = Vec::new();
        let mut total_in_dir: u64 = 0;

        let read_dir: ReadDir = match fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(_) => return Ok((folders, 0)),
        };

        for entry in read_dir.flatten() {
            let path = entry.path();
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };

            // Skip symlinks to avoid cycles
            if meta.file_type().is_symlink() {
                continue;
            }

            if meta.is_dir() {
                let mut size_accum: u64 = 0;
                let mut children: Vec<FolderInfo> = Vec::new();

                // If we can go deeper, collect subfolders as children
                if depth + 1 < max_depth {
                    match self.collect_folders(path.as_path(), depth + 1, max_depth) {
                        Ok((sub_children, sub_total)) => {
                            children = sub_children;
                            size_accum += sub_total;
                        }
                        Err(_) => {}
                    }
                } else {
                    // Reached depth limit; just aggregate sizes within this folder
                    size_accum += self.compute_dir_size_shallow(path.as_path());
                }

                let info = FolderInfo {
                    path: path.clone(),
                    size: size_accum,
                    children,
                };
                total_in_dir += size_accum;
                folders.push(info);
            } else if meta.is_file() {
                total_in_dir = total_in_dir.saturating_add(meta.len());
            }
        }

        // Sort folders by size descending for more useful UI presentation
        folders.sort_by(|a, b| b.size.cmp(&a.size));

        Ok((folders, total_in_dir))
    }

    fn compute_dir_size_shallow(&self, dir: &Path) -> u64 {
        let mut size: u64 = 0;
        let read_dir = match fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(_) => return 0,
        };
        for entry in read_dir.flatten() {
            if let Ok(meta) = entry.metadata() {
                if meta.file_type().is_symlink() {
                    continue;
                }
                if meta.is_file() {
                    size = size.saturating_add(meta.len());
                } else if meta.is_dir() {
                    size = size.saturating_add(self.compute_dir_size_shallow(entry.path().as_path()));
                }
            }
        }
        size
    }
}


