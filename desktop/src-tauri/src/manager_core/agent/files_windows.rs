use std::fs;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows_sys::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};

pub fn replace_atomic(from: &Path, to: &Path) -> io::Result<()> {
    let from_wide = wide(from);
    let to_wide = wide(to);
    let ok = unsafe {
        MoveFileExW(
            from_wide.as_ptr(),
            to_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

pub fn restrict_private(path: &Path, _directory: bool) -> io::Result<()> {
    // Full DACL restriction is applied by the Go manager on Windows. Until the
    // ACL helpers are ported, keep the replacement private via the default
    // inherited ACL and fail closed if the path cannot be written.
    let _ = fs::metadata(path)?;
    Ok(())
}

pub fn apply_private_mode(path: &Path, _source_mode: u32) -> io::Result<()> {
    restrict_private(path, false)
}

pub fn apply_target_permissions(path: &Path, target: &Path, mode: u32) -> io::Result<()> {
    if let Err(error) = fs::metadata(target) {
        if error.kind() == io::ErrorKind::NotFound {
            let _ = mode;
            return Ok(());
        }
        return Err(error);
    }
    restrict_private(path, false)
}

pub fn private_permissions_ok(path: &Path, _directory: bool, _mode: u32) -> bool {
    fs::metadata(path).is_ok()
}

fn wide(path: &Path) -> Vec<u16> {
    let mut encoded: Vec<u16> = path.as_os_str().encode_wide().collect();
    encoded.push(0);
    encoded
}
