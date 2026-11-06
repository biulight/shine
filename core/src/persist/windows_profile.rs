//! Windows profile replacement must also preserve security when the Shell journal
//! has moved the previous object to its rollback path.
use anyhow::{Context, Result, ensure};
use std::{
    fs::File,
    io::Write,
    os::windows::{ffi::OsStrExt, io::FromRawHandle},
    path::Path,
    ptr,
};
use windows_sys::Win32::{
    Foundation::*,
    Security::{Authorization::*, *},
    Storage::FileSystem::*,
};

fn wide(path: &Path) -> Result<Vec<u16>> {
    let mut wide: Vec<_> = path.as_os_str().encode_wide().collect();
    ensure!(!wide.contains(&0), "NUL in profile path");
    wide.push(0);
    Ok(wide)
}

fn stage_like(template: &Path, temp: &Path) -> Result<File> {
    let template = wide(template)?;
    let temp = wide(temp)?;
    let mut descriptor = ptr::null_mut();
    // Create with the original owner, group and DACL before writing any contents.
    unsafe {
        let code = GetNamedSecurityInfoW(
            template.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &mut descriptor,
        );
        if code != 0 {
            return Err(std::io::Error::from_raw_os_error(code as i32).into());
        }
        let security = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let handle = CreateFileW(
            temp.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_READ,
            &security,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        );
        let error = std::io::Error::last_os_error();
        LocalFree(descriptor);
        if handle == INVALID_HANDLE_VALUE {
            return Err(error.into());
        }
        Ok(File::from_raw_handle(handle))
    }
}

pub async fn write_profile_like(path: &Path, bytes: &[u8], template: &Path) -> Result<()> {
    let temp = path.with_file_name(format!(".shine-profile-{}", uuid::Uuid::new_v4()));
    let mut file = stage_like(template, &temp)?;
    let result = async {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        if path == template {
            return replace_profile(path, &temp);
        }
        // Retain named streams as well as the DACL from the rollback object. Merge on a
        // temporary link so a failed publication leaves both the journal and rollback usable.
        let link = path.with_file_name(format!(".shine-profile-{}", uuid::Uuid::new_v4()));
        std::fs::hard_link(template, &link)?;
        let publish = async {
            replace_profile(&link, &temp)?;
            super::finalize_temp_new(&link, path).await
        }
        .await;
        if publish.is_err() {
            let _ = std::fs::remove_file(&link);
        }
        publish
    }
    .await;
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result.context("preserving Windows profile security")
}

fn replace_profile(path: &Path, temp: &Path) -> Result<()> {
    let path = wide(path)?;
    let temp = wide(temp)?;
    if unsafe {
        ReplaceFileW(
            path.as_ptr(),
            temp.as_ptr(),
            ptr::null(),
            0,
            ptr::null(),
            ptr::null(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
