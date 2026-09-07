use std::fs::{self, OpenOptions};
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

pub fn replace_atomic(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}

pub fn sync_directory(path: &Path) -> io::Result<()> {
    let dir = OpenOptions::new().read(true).open(path)?;
    dir.sync_all()
}

pub fn restrict_private(path: &Path, directory: bool) -> io::Result<()> {
    let mode = if directory { 0o700 } else { 0o600 };
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

pub fn apply_private_mode(path: &Path, source_mode: u32) -> io::Result<()> {
    let mut mode = source_mode & 0o700;
    if mode & 0o400 == 0 {
        mode |= 0o400;
    }
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

pub fn apply_target_permissions(path: &Path, _target: &Path, mode: u32) -> io::Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o777))
}

pub fn private_permissions_ok(_path: &Path, directory: bool, mode: u32) -> bool {
    let want = if directory { 0o700 } else { 0o600 };
    mode & 0o777 == want
}
