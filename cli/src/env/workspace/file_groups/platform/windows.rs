use super::*;
use std::{
    os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    ptr,
};
use windows_sys::Win32::{
    Foundation::*,
    Security::{Authorization::*, *},
    Storage::FileSystem::*,
    System::{Diagnostics::ToolHelp::*, JobObjects::*, Threading::*},
};
use windows_sys::core::BOOL;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct Identity(u32, u32, u32);

fn wide(path: &Path) -> Result<Vec<u16>> {
    let mut value: Vec<u16> = path.as_os_str().encode_wide().collect();
    ensure!(!value.contains(&0), "NUL in file path");
    value.push(0);
    Ok(value)
}
fn checked(value: BOOL) -> Result<()> {
    if value == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
struct Local(*mut core::ffi::c_void);
impl Drop for Local {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}

fn sid() -> Result<String> {
    unsafe {
        let mut token = ptr::null_mut();
        checked(OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_QUERY,
            &mut token,
        ))?;
        let token = OwnedHandle::from_raw_handle(token);
        let mut length = 0;
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            ptr::null_mut(),
            0,
            &mut length,
        );
        ensure!(length > 0, "cannot inspect current user");
        let mut buffer = vec![0u64; (length as usize).div_ceil(8)];
        checked(GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            buffer.as_mut_ptr().cast(),
            length,
            &mut length,
        ))?;
        let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
        let mut text = ptr::null_mut();
        checked(ConvertSidToStringSidW(user.User.Sid, &mut text))?;
        let allocation = Local(text.cast());
        let mut n = 0;
        while *text.add(n) != 0 {
            n += 1;
        }
        let value = String::from_utf16(std::slice::from_raw_parts(text, n))?;
        drop(allocation);
        Ok(value)
    }
}

fn descriptor() -> Result<Local> {
    let sddl: Vec<u16> = format!("O:{}D:P(A;;FA;;;{})\0", sid()?, sid()?)
        .encode_utf16()
        .collect();
    let mut descriptor = ptr::null_mut();
    unsafe {
        checked(ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            ptr::null_mut(),
        ))?;
    }
    Ok(Local(descriptor))
}

fn acl_volume(path: &Path) -> Result<()> {
    let path = wide(path)?;
    let mut volume = vec![0u16; 32768];
    let mut flags = 0;
    unsafe {
        checked(GetVolumePathNameW(
            path.as_ptr(),
            volume.as_mut_ptr(),
            volume.len() as u32,
        ))?;
        checked(GetVolumeInformationW(
            volume.as_ptr(),
            ptr::null_mut(),
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut flags,
            ptr::null_mut(),
            0,
        ))?;
    }
    ensure!(
        flags & 0x00000008 != 0,
        "filesystem cannot guarantee private ACLs"
    );
    Ok(())
}

fn verify_acl(path: &Path) -> Result<()> {
    let expected = descriptor()?;
    let mut expected_owner = ptr::null_mut();
    let mut defaulted = 0;
    let mut owner = ptr::null_mut();
    let mut acl = ptr::null_mut();
    let mut sd = ptr::null_mut();
    let path = wide(path)?;
    unsafe {
        checked(GetSecurityDescriptorOwner(
            expected.0,
            &mut expected_owner,
            &mut defaulted,
        ))?;
        let code = GetNamedSecurityInfoW(
            path.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            &mut acl,
            ptr::null_mut(),
            &mut sd,
        );
        ensure!(code == 0, "cannot inspect private ACL");
        let allocation = Local(sd);
        let mut control = 0;
        let mut revision = 0;
        checked(GetSecurityDescriptorControl(
            sd,
            &mut control,
            &mut revision,
        ))?;
        ensure!(
            EqualSid(owner, expected_owner) != 0
                && control & SE_DACL_PROTECTED != 0
                && !acl.is_null()
                && (*acl).AceCount == 1,
            "private path ACL is not current-user-only"
        );
        let mut ace = ptr::null_mut();
        checked(GetAce(acl, 0, &mut ace))?;
        let ace = &*ace.cast::<ACCESS_ALLOWED_ACE>();
        ensure!(
            ace.Header.AceType == 0
                && EqualSid(
                    ptr::addr_of!(ace.SidStart).cast_mut().cast(),
                    expected_owner
                ) != 0,
            "private path ACL grants another principal"
        );
        drop(allocation);
    }
    Ok(())
}

pub(in super::super) fn is_link(metadata: &Metadata) -> bool {
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}
pub(in super::super) fn directory_identity(file: &File) -> Result<Identity> {
    let mut info = unsafe { std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>() };
    unsafe {
        checked(GetFileInformationByHandle(file.as_raw_handle(), &mut info))?;
    }
    ensure!(
        info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT == 0,
        "reparse point rejected"
    );
    Ok(Identity(
        info.dwVolumeSerialNumber,
        info.nFileIndexHigh,
        info.nFileIndexLow,
    ))
}
pub(in super::super) fn identity(file: &File) -> Result<Identity> {
    let mut info = unsafe { std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>() };
    unsafe {
        checked(GetFileInformationByHandle(file.as_raw_handle(), &mut info))?;
    }
    ensure!(
        info.nNumberOfLinks == 1 && info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0,
        "mapped file must be regular with exactly one hard link"
    );
    directory_identity(file)
}
pub(in super::super) fn open_read(path: &Path) -> Result<File> {
    Ok(OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?)
}
pub(in super::super) fn path_identity(path: &Path) -> Result<Identity> {
    // A metadata-only handle avoids requesting plaintext read permission.
    let file = OpenOptions::new()
        .access_mode(0)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    identity(&file)
}
pub(in super::super) fn open_directory(path: &Path) -> Result<File> {
    Ok(OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)?)
}
pub(in super::super) fn create_private(path: &Path) -> Result<File> {
    acl_volume(path.parent().context("parent required")?)?;
    let descriptor = descriptor()?;
    let security = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    let path_wide = wide(path)?;
    let handle = unsafe {
        CreateFileW(
            path_wide.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            &security,
            CREATE_NEW,
            FILE_FLAG_OPEN_REPARSE_POINT,
            ptr::null_mut(),
        )
    };
    ensure!(
        handle != INVALID_HANDLE_VALUE,
        "cannot privately create file: {}",
        std::io::Error::last_os_error()
    );
    let file = unsafe { File::from_raw_handle(handle) };
    if let Err(error) = verify_acl(path) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(error);
    }
    Ok(file)
}
pub(in super::super) fn private_directory(path: &Path) -> Result<bool> {
    acl_volume(path.parent().context("parent required")?)?;
    let descriptor = descriptor()?;
    let security = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    let path_wide = wide(path)?;
    let created = unsafe {
        let created = CreateDirectoryW(path_wide.as_ptr(), &security) != 0;
        ensure!(
            created || GetLastError() == ERROR_ALREADY_EXISTS,
            "cannot create private directory"
        );
        created
    };
    ensure!(
        path.is_dir() && !is_link(&fs::symlink_metadata(path)?),
        "invalid private directory"
    );
    verify_acl(path)?;
    Ok(created)
}
pub(in super::super) fn lock_directory() -> Result<String> {
    Ok(format!("shine-file-group-locks-{}", sid()?))
}

pub(in super::super) fn publish_new(temp: &Path, path: &Path) -> Result<()> {
    let temp = wide(temp)?;
    let path = wide(path)?;
    // No REPLACE_EXISTING: a concurrent creator is never overwritten. WRITE_THROUGH
    // waits for durable publication before --remove-plaintext may discard originals.
    unsafe {
        checked(MoveFileExW(
            temp.as_ptr(),
            path.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        ))
    }
}

pub(in super::super) fn sync_cipher_parents(_root: &Path, _path: &Path) -> Result<()> {
    // Both existing-file replacement and new publication use MOVEFILE_WRITE_THROUGH.
    Ok(())
}
pub(in super::super) fn lock_file(path: &Path) -> Result<File> {
    let file = if path.try_exists()? {
        verify_acl(path)?;
        OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?
    } else {
        match create_private(path) {
            Ok(file) => file,
            Err(error) if path.try_exists()? => {
                verify_acl(path)?;
                return lock_file(path).with_context(|| error.to_string());
            }
            Err(error) => return Err(error),
        }
    };
    identity(&file)?;
    Ok(file)
}

struct Job(OwnedHandle);
impl Job {
    fn create() -> Result<Self> {
        let raw = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        ensure!(!raw.is_null(), "cannot create process job");
        let job = Self(unsafe { OwnedHandle::from_raw_handle(raw) });
        let mut limits = unsafe { std::mem::zeroed::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        unsafe {
            checked(SetInformationJobObject(
                job.0.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                ptr::addr_of!(limits).cast(),
                std::mem::size_of_val(&limits) as u32,
            ))?;
        }
        Ok(job)
    }
    fn terminate(&self) -> Result<()> {
        unsafe { checked(TerminateJobObject(self.0.as_raw_handle(), 130)) }
    }
    async fn drain(&self) -> Result<()> {
        self.terminate()?;
        for _ in 0..250 {
            let mut info = unsafe { std::mem::zeroed::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() };
            unsafe {
                checked(QueryInformationJobObject(
                    self.0.as_raw_handle(),
                    JobObjectBasicAccountingInformation,
                    ptr::addr_of_mut!(info).cast(),
                    std::mem::size_of_val(&info) as u32,
                    ptr::null_mut(),
                ))?;
            }
            if info.ActiveProcesses == 0 {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        bail!("command descendants did not terminate")
    }
}

fn resume(pid: u32) -> Result<()> {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        ensure!(
            snapshot != INVALID_HANDLE_VALUE,
            "cannot inspect suspended command"
        );
        let snapshot = OwnedHandle::from_raw_handle(snapshot);
        let mut entry = std::mem::zeroed::<THREADENTRY32>();
        entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
        let mut valid = Thread32First(snapshot.as_raw_handle(), &mut entry);
        while valid != 0 {
            if entry.th32OwnerProcessID == pid {
                let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                ensure!(!thread.is_null(), "cannot resume command");
                let thread = OwnedHandle::from_raw_handle(thread);
                ensure!(
                    ResumeThread(thread.as_raw_handle()) != u32::MAX,
                    "cannot resume command"
                );
                return Ok(());
            }
            valid = Thread32Next(snapshot.as_raw_handle(), &mut entry);
        }
    }
    bail!("suspended command thread not found")
}

pub(in super::super) async fn command_status(
    command: &[OsString],
    values: &BTreeMap<String, String>,
    override_env: bool,
    explicit: &BTreeMap<String, String>,
) -> Result<std::process::ExitStatus> {
    let mut interrupt = tokio::signal::windows::ctrl_c()?;
    let mut ctrl_break = tokio::signal::windows::ctrl_break()?;
    let job = Job::create()?;
    let mut builder = command_builder(command, values, override_env, explicit)?;
    builder
        .creation_flags(CREATE_SUSPENDED | CREATE_NEW_PROCESS_GROUP)
        .kill_on_drop(true);
    let mut child = builder.spawn().context("starting file-group command")?;
    let result = (|| {
        let handle = child
            .raw_handle()
            .context("child process handle required")?;
        unsafe {
            checked(AssignProcessToJobObject(job.0.as_raw_handle(), handle))?;
        }
        resume(child.id().context("child PID required")?)
    })();
    if let Err(error) = result {
        let _ = child.kill().await;
        let _ = child.wait().await;
        return Err(error);
    }
    let status = tokio::select! {
        status = child.wait() => status?,
        _ = interrupt.recv() => { job.terminate()?; child.wait().await? },
        _ = ctrl_break.recv() => { job.terminate()?; child.wait().await? },
    };
    job.drain().await?;
    Ok(status)
}
