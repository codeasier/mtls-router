use std::fs::File;
use std::io;
use std::os::windows::io::AsRawHandle;

use windows_sys::Win32::Foundation::{ERROR_LOCK_VIOLATION, HANDLE};
use windows_sys::Win32::Storage::FileSystem::{
    LockFileEx, UnlockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY,
};
use windows_sys::Win32::System::IO::OVERLAPPED;

/// A zeroed OVERLAPPED locks byte range 0 with no event handle.
fn zeroed_overlapped() -> OVERLAPPED {
    // SAFETY: OVERLAPPED is a plain C struct for which all-zero bytes are a
    // valid value (null hEvent, zero offsets).
    unsafe { std::mem::zeroed() }
}

pub fn try_lock_file(file: &File) -> io::Result<bool> {
    let mut overlapped = zeroed_overlapped();
    let ok = unsafe {
        LockFileEx(
            file.as_raw_handle() as HANDLE,
            LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
            0,
            1,
            0,
            &mut overlapped,
        )
    };
    if ok != 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(ERROR_LOCK_VIOLATION as i32) {
        Ok(false)
    } else {
        Err(error)
    }
}

pub fn unlock_file(file: &File) -> io::Result<()> {
    let mut overlapped = zeroed_overlapped();
    let ok = unsafe { UnlockFileEx(file.as_raw_handle() as HANDLE, 0, 1, 0, &mut overlapped) };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
