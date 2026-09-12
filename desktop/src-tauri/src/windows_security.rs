//! Current-user file security and process token identity shared by the desktop and manager.
use std::{io, os::windows::ffi::OsStrExt, path::Path, ptr};
use windows_sys::Win32::{
    Foundation::{CloseHandle, LocalFree, HANDLE},
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            GetNamedSecurityInfoW, SetNamedSecurityInfoW, SE_FILE_OBJECT,
        },
        EqualSid, GetAce, GetSecurityDescriptorControl, GetSecurityDescriptorDacl,
        GetTokenInformation, TokenUser, ACCESS_ALLOWED_ACE, ACE_HEADER, ACL,
        DACL_SECURITY_INFORMATION, INHERIT_ONLY_ACE, PROTECTED_DACL_SECURITY_INFORMATION,
        SE_DACL_PROTECTED, TOKEN_QUERY, TOKEN_USER,
    },
    System::Threading::{
        GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
    },
};

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

struct LocalMemory(*mut std::ffi::c_void);
impl Drop for LocalMemory {
    fn drop(&mut self) {
        unsafe { LocalFree(self.0) };
    }
}

fn check(ok: i32) -> io::Result<()> {
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn status(code: u32) -> io::Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(code as i32))
    }
}

fn wide(path: &Path) -> io::Result<Vec<u16>> {
    let mut value: Vec<_> = path.as_os_str().encode_wide().collect();
    if value.contains(&0) {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    value.push(0);
    Ok(value)
}

// Word-aligned token storage keeps TOKEN_USER and its embedded SID alive together.
fn token_user(process: HANDLE) -> io::Result<Vec<usize>> {
    let mut token = ptr::null_mut();
    check(unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) })?;
    let token = Handle(token);
    let mut size = 0;
    unsafe { GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut size) };
    if size < std::mem::size_of::<TOKEN_USER>() as u32 || size > 65536 {
        return Err(io::Error::from(io::ErrorKind::InvalidData));
    }
    let mut buffer = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
    check(unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            size,
            &mut size,
        )
    })?;
    Ok(buffer)
}

fn sid_string(user: &[usize]) -> io::Result<String> {
    let sid = unsafe { (*(user.as_ptr().cast::<TOKEN_USER>())).User.Sid };
    let mut text = ptr::null_mut();
    check(unsafe { ConvertSidToStringSidW(sid, &mut text) })?;
    let _memory = LocalMemory(text.cast());
    let mut len = 0;
    unsafe {
        while *text.add(len) != 0 {
            len += 1;
        }
        String::from_utf16(std::slice::from_raw_parts(text, len))
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))
    }
}

pub(crate) fn current_sid() -> io::Result<String> {
    sid_string(&token_user(unsafe { GetCurrentProcess() })?)
}

pub(crate) fn process_sid(pid: u32) -> io::Result<String> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(io::Error::last_os_error());
    }
    let process = Handle(process);
    sid_string(&token_user(process.0)?)
}

fn dacl(path: &Path) -> io::Result<(LocalMemory, *mut ACL)> {
    let path = wide(path)?;
    let mut descriptor = ptr::null_mut();
    let mut acl = ptr::null_mut();
    status(unsafe {
        GetNamedSecurityInfoW(
            path.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut acl,
            ptr::null_mut(),
            &mut descriptor,
        )
    })?;
    let memory = LocalMemory(descriptor);
    if acl.is_null() {
        return Err(io::Error::from(io::ErrorKind::PermissionDenied));
    }
    Ok((memory, acl))
}

fn set_dacl(path: &Path, acl: *mut ACL) -> io::Result<()> {
    let path = wide(path)?;
    status(unsafe {
        SetNamedSecurityInfoW(
            path.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            acl,
            ptr::null_mut(),
        )
    })
}

pub(crate) fn restrict_private(path: &Path, directory: bool) -> io::Result<()> {
    let flags = if directory { "OICI" } else { "" };
    apply_sddl(path, &format!("D:P(A;{flags};FA;;;{})", current_sid()?))
}

fn apply_sddl(path: &Path, sddl: &str) -> io::Result<()> {
    let sddl: Vec<_> = sddl.encode_utf16().chain(Some(0)).collect();
    let mut descriptor = ptr::null_mut();
    check(unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            ptr::null_mut(),
        )
    })?;
    let _memory = LocalMemory(descriptor);
    let mut present = 0;
    let mut defaulted = 0;
    let mut acl = ptr::null_mut();
    check(unsafe {
        GetSecurityDescriptorDacl(descriptor, &mut present, &mut acl, &mut defaulted)
    })?;
    if present == 0 || acl.is_null() {
        return Err(io::Error::from(io::ErrorKind::InvalidData));
    }
    set_dacl(path, acl)
}

pub(crate) fn copy_dacl(path: &Path, target: &Path) -> io::Result<()> {
    let (_descriptor, acl) = dacl(target)?;
    set_dacl(path, acl)
}

pub(crate) fn private_permissions_ok(path: &Path) -> bool {
    let result = (|| -> io::Result<bool> {
        let (descriptor, acl) = dacl(path)?;
        let mut control = 0;
        let mut revision = 0;
        check(unsafe { GetSecurityDescriptorControl(descriptor.0, &mut control, &mut revision) })?;
        if control & SE_DACL_PROTECTED == 0 || unsafe { (*acl).AceCount } != 1 {
            return Ok(false);
        }
        let mut ace = ptr::null_mut();
        check(unsafe { GetAce(acl, 0, &mut ace) })?;
        let header = unsafe { &*ace.cast::<ACE_HEADER>() };
        // Reject extra grants, inherited-only grants and nonstandard ACE layouts.
        if header.AceType != 0
            || u32::from(header.AceFlags) & INHERIT_ONLY_ACE != 0
            || (header.AceSize as usize) < std::mem::size_of::<ACCESS_ALLOWED_ACE>()
        {
            return Ok(false);
        }
        let ace = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
        if ace.Mask != windows_sys::Win32::Storage::FileSystem::FILE_ALL_ACCESS {
            return Ok(false);
        }
        let user = token_user(unsafe { GetCurrentProcess() })?;
        let sid = unsafe { (*(user.as_ptr().cast::<TOKEN_USER>())).User.Sid };
        Ok(unsafe { EqualSid(ptr::addr_of!(ace.SidStart).cast_mut().cast(), sid) } != 0)
    })();
    result.unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_current_process_sid_matches_current_user() {
        assert_eq!(
            current_sid().unwrap(),
            process_sid(std::process::id()).unwrap()
        );
        assert!(process_sid(0).is_err());
    }

    #[test]
    fn native_private_dacl_survives_replacement() {
        let root = std::env::temp_dir().join(format!("mtls-dacl-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        apply_sddl(
            &root,
            &format!(
                "D:P(A;OICI;FA;;;{})(A;OICI;GR;;;WD)",
                current_sid().unwrap()
            ),
        )
        .unwrap();
        assert!(!private_permissions_ok(&root));
        restrict_private(&root, true).unwrap();
        assert!(private_permissions_ok(&root));
        let target = root.join("target");
        let replacement = root.join("replacement");
        std::fs::write(&target, b"test").unwrap();
        restrict_private(&target, false).unwrap();
        std::fs::write(&replacement, b"replacement").unwrap();
        assert!(!private_permissions_ok(&replacement));
        copy_dacl(&replacement, &target).unwrap();
        assert!(private_permissions_ok(&replacement));
        check(unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                wide(&replacement).unwrap().as_ptr(),
                wide(&target).unwrap().as_ptr(),
                windows_sys::Win32::Storage::FileSystem::MOVEFILE_REPLACE_EXISTING,
            )
        })
        .unwrap();
        assert!(private_permissions_ok(&target));
        assert!(!private_permissions_ok(&root.join("missing")));
        std::fs::remove_dir_all(root).unwrap();
    }
}
