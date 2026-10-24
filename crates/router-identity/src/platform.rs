use crate::{IdentityError, Result};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;
use zeroize::Zeroizing;

pub fn directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
            .map_err(|_| IdentityError::Storage)?;
        let meta = fs::symlink_metadata(path).map_err(|_| IdentityError::Storage)?;
        if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } {
            return Err(IdentityError::InvalidState);
        }
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| IdentityError::Storage)?;
    }
    #[cfg(windows)]
    {
        fs::create_dir_all(path).map_err(|_| IdentityError::Storage)?;
        let meta = fs::symlink_metadata(path).map_err(|_| IdentityError::Storage)?;
        use std::os::windows::fs::MetadataExt;
        if !meta.is_dir()
            || meta.file_attributes()
                & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
                != 0
        {
            return Err(IdentityError::InvalidState);
        }
        router_contract::windows_security::restrict_private(path, true)
            .map_err(|_| IdentityError::Storage)?;
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        return Err(IdentityError::Storage);
    }
    Ok(())
}

fn check_file(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path).map_err(|_| IdentityError::Storage)?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(IdentityError::InvalidState);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if meta.uid() != unsafe { libc::geteuid() } || meta.permissions().mode() & 0o777 != 0o600 {
            return Err(IdentityError::InvalidState);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
        {
            return Err(IdentityError::InvalidState);
        }
        router_contract::windows_security::restrict_private(path, false)
            .map_err(|_| IdentityError::Storage)?;
        if !router_contract::windows_security::private_permissions_ok(path) {
            return Err(IdentityError::InvalidState);
        }
    }
    Ok(())
}

fn options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    options
}

pub fn read(path: &Path) -> Result<Option<Zeroizing<Vec<u8>>>> {
    match fs::symlink_metadata(path) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(IdentityError::Storage),
        Ok(_) => check_file(path)?,
    }
    let mut file = options()
        .read(true)
        .open(path)
        .map_err(|_| IdentityError::Storage)?;
    let mut data = Zeroizing::new(Vec::new());
    (&mut file)
        .take(512 * 1024 + 1)
        .read_to_end(&mut data)
        .map_err(|_| IdentityError::Storage)?;
    if data.len() > 512 * 1024 {
        return Err(IdentityError::InvalidState);
    }
    Ok(Some(data))
}

pub fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or(IdentityError::Storage)?;
    let stage = parent.join(format!(".identity-stage-{}", Uuid::new_v4()));
    let result = (|| {
        let mut file = options()
            .write(true)
            .create_new(true)
            .open(&stage)
            .map_err(|_| IdentityError::Storage)?;
        #[cfg(windows)]
        router_contract::windows_security::restrict_private(&stage, false)
            .map_err(|_| IdentityError::Storage)?;
        file.write_all(data).map_err(|_| IdentityError::Storage)?;
        file.sync_all().map_err(|_| IdentityError::Storage)?;
        drop(file);
        #[cfg(unix)]
        {
            fs::rename(&stage, path).map_err(|_| IdentityError::Storage)?;
            File::open(parent)
                .and_then(|f| f.sync_all())
                .map_err(|_| IdentityError::Storage)?;
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::Storage::FileSystem::{
                MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
            };
            let old: Vec<u16> = stage.as_os_str().encode_wide().chain(Some(0)).collect();
            let new: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            if unsafe {
                MoveFileExW(
                    old.as_ptr(),
                    new.as_ptr(),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                )
            } == 0
            {
                return Err(IdentityError::Storage);
            }
        }
        Ok(())
    })();
    let _ = fs::remove_file(stage);
    result
}

pub struct Lock {
    file: File,
    _path: PathBuf,
}
impl Lock {
    pub fn acquire(path: &Path) -> Result<Self> {
        if fs::symlink_metadata(path).is_ok() {
            check_file(path)?;
        }
        let file = options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|_| IdentityError::Storage)?;
        #[cfg(windows)]
        router_contract::windows_security::restrict_private(path, false)
            .map_err(|_| IdentityError::Storage)?;
        check_file(path)?;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            #[cfg(unix)]
            let locked = {
                use std::os::fd::AsRawFd;
                (unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) }) == 0
            };
            #[cfg(windows)]
            let locked = {
                use std::os::windows::io::AsRawHandle;
                use windows_sys::Win32::{
                    Storage::FileSystem::{
                        LockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY,
                    },
                    System::IO::OVERLAPPED,
                };
                let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
                (unsafe {
                    LockFileEx(
                        file.as_raw_handle().cast(),
                        LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                        0,
                        1,
                        0,
                        &mut overlapped,
                    )
                }) != 0
            };
            #[cfg(not(any(unix, windows)))]
            let locked = false;
            if locked {
                return Ok(Self {
                    file,
                    _path: path.to_path_buf(),
                });
            }
            if Instant::now() >= deadline {
                return Err(IdentityError::Busy);
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let _ = unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::{Storage::FileSystem::UnlockFileEx, System::IO::OVERLAPPED};
            let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
            let _ =
                unsafe { UnlockFileEx(self.file.as_raw_handle().cast(), 0, 1, 0, &mut overlapped) };
        }
    }
}
