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
    write_profile_like_inner(path, bytes, template, false).await
}

/// Replace a generated artifact while deriving its security from the active profile.
pub async fn replace_profile_like(path: &Path, bytes: &[u8], template: &Path) -> Result<()> {
    write_profile_like_inner(path, bytes, template, true).await
}

async fn write_profile_like_inner(
    path: &Path,
    bytes: &[u8],
    template: &Path,
    replace_existing: bool,
) -> Result<()> {
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
            if replace_existing {
                super::finalize_temp(&link, path).await
            } else {
                super::finalize_temp_new(&link, path).await
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn profile_artifact_replacement_does_not_weaken_recovery_create_only_semantics() {
        let root =
            std::env::temp_dir().join(format!("shine-profile-copy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let active = root.join("active.ps1");
        let artifact = root.join("active.merge.ps1");
        std::fs::write(&active, b"active").unwrap();
        write_profile_like(&artifact, b"first", &active)
            .await
            .unwrap();
        assert!(
            write_profile_like(&artifact, b"unexpected", &active)
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(&artifact).unwrap(), b"first");
        replace_profile_like(&artifact, b"second", &active)
            .await
            .unwrap();
        assert_eq!(std::fs::read(&artifact).unwrap(), b"second");
        assert_eq!(std::fs::read(&active).unwrap(), b"active");
        std::fs::remove_dir_all(&root).unwrap();
    }
    fn acl_script(path: &Path, script: &str) -> String {
        let output = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            // A pwsh parent may supply a module path incompatible with Windows PowerShell.
            .env_remove("PSModulePath")
            .env("SHINE_ACL_TEST_PATH", path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }

    fn protect_profile(path: &Path) -> String {
        acl_script(
            path,
            r#"$ErrorActionPreference='Stop'; $p=$env:SHINE_ACL_TEST_PATH; $acl=Get-Acl -LiteralPath $p; $acl.SetAccessRuleProtection($true,$false); foreach($r in @($acl.Access)) { [void]$acl.RemoveAccessRuleSpecific($r) }; $sid=[System.Security.Principal.WindowsIdentity]::GetCurrent().User; $rule=[System.Security.AccessControl.FileSystemAccessRule]::new($sid,'FullControl','Allow'); $acl.AddAccessRule($rule); Set-Acl -LiteralPath $p -AclObject $acl; (Get-Acl -LiteralPath $p).Sddl"#,
        )
    }

    #[tokio::test]
    async fn profile_artifacts_preserve_private_acl_streams_and_failed_publication() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = std::env::temp_dir().join(format!("shine-profile-acl-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let active = root.join("active.ps1");
        std::fs::write(&active, b"private active").unwrap();
        std::fs::write(root.join("active.ps1:shine-test"), b"private stream").unwrap();
        let private_acl = protect_profile(&active);
        assert!(!private_acl.is_empty());
        for phase in ["new", "merge"] {
            let artifact = root.join(format!("active.{phase}.ps1"));
            std::fs::write(&artifact, b"existing artifact with inherited ACL").unwrap();
            for content in [b"first".as_slice(), b"second", b"third"] {
                replace_profile_like(&artifact, content, &active)
                    .await
                    .unwrap();
                assert_eq!(std::fs::read(&artifact).unwrap(), content);
                assert_eq!(
                    acl_script(
                        &artifact,
                        "(Get-Acl -LiteralPath $env:SHINE_ACL_TEST_PATH).Sddl"
                    ),
                    private_acl
                );
                assert_eq!(
                    std::fs::read(root.join(format!("active.{phase}.ps1:shine-test"))).unwrap(),
                    b"private stream"
                );
            }
            assert!(
                write_profile_like(&artifact, b"must not overwrite", &active)
                    .await
                    .is_err()
            );
            assert_eq!(std::fs::read(&artifact).unwrap(), b"third");
            let locked = std::fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(&artifact)
                .unwrap();
            assert!(
                replace_profile_like(&artifact, b"blocked", &active)
                    .await
                    .is_err()
            );
            drop(locked);
            assert_eq!(std::fs::read(&artifact).unwrap(), b"third");
            assert_eq!(
                acl_script(
                    &artifact,
                    "(Get-Acl -LiteralPath $env:SHINE_ACL_TEST_PATH).Sddl"
                ),
                private_acl
            );
        }
        assert_eq!(std::fs::read(&active).unwrap(), b"private active");
        assert_eq!(
            acl_script(
                &active,
                "(Get-Acl -LiteralPath $env:SHINE_ACL_TEST_PATH).Sddl"
            ),
            private_acl
        );
        assert!(!std::fs::read_dir(&root).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".shine-")
        }));
        std::fs::remove_dir_all(&root).unwrap();
    }
    #[tokio::test]
    async fn sys_init_native_repeated_conflicts_preserve_profile_and_artifact_acls() {
        use crate::plan::PlanApprovalV1;
        use crate::runtime::{
            CoreRuntime, PresetSnapshot, PresetSourceKind, RealHost, RuntimeContext,
            RuntimePlatform, ShellType, SysInitPlanRequest, SysItemStatus,
        };
        let home =
            std::env::temp_dir().join(format!("shine-sys-init-acl-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(home.join("Documents/PowerShell")).unwrap();
        std::fs::create_dir_all(home.join("Documents/WindowsPowerShell")).unwrap();
        let mut context = RuntimeContext::isolated(
            home.clone(),
            home.join("state"),
            home.join("presets"),
            home.join("bin"),
            RuntimePlatform::Windows,
        );
        context.shell = ShellType::PowerShell;
        let snapshot = |body: &[u8]| {
            PresetSnapshot::builder(PresetSourceKind::Embedded)
                .file("sys/windows/shine.toml", b"version = 2\n".to_vec())
                .file("sys/windows/profile/base.pre.ps1", body.to_vec())
                .build()
        };
        let request = SysInitPlanRequest {
            os_id: "windows".into(),
            force_profile: false,
        };
        let startup = home.join("Documents/PowerShell/Microsoft.PowerShell_profile.ps1");
        std::fs::write(&startup, "\u{feff}# private user startup\r\n").unwrap();
        let startup_acl = protect_profile(&startup);
        let runtime = CoreRuntime::new(RealHost, context.clone(), snapshot(b"old\nkeep\n"));
        let plan = runtime.plan_sys_init(request.clone()).await.unwrap();
        runtime
            .init_sys_profile_approved(
                request.clone(),
                &PlanApprovalV1::for_reviewed_plan(&plan).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            acl_script(
                &startup,
                "(Get-Acl -LiteralPath $env:SHINE_ACL_TEST_PATH).Sddl"
            ),
            startup_acl
        );
        assert!(
            std::fs::read(&startup)
                .unwrap()
                .starts_with("\u{feff}".as_bytes())
        );
        let active = home.join(".shine/profile/windows-sys.pre.ps1");
        let private_acl = protect_profile(&active);
        let runtime = CoreRuntime::new(RealHost, context, snapshot(b"new\nkeep\n"));
        for user_content in [b"user one\n".as_slice(), b"user two\n", b"user three\n"] {
            std::fs::write(&active, user_content).unwrap();
            let plan = runtime.plan_sys_init(request.clone()).await.unwrap();
            let outcome = runtime
                .init_sys_profile_approved(
                    request.clone(),
                    &PlanApprovalV1::for_reviewed_plan(&plan).unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(outcome.status, SysItemStatus::NeedsAction);
            assert_eq!(std::fs::read(&active).unwrap(), user_content);
            for phase in ["new", "merge"] {
                let artifact = home.join(format!(".shine/profile/windows-sys.pre.{phase}.ps1"));
                assert_eq!(
                    acl_script(
                        &artifact,
                        "(Get-Acl -LiteralPath $env:SHINE_ACL_TEST_PATH).Sddl"
                    ),
                    private_acl
                );
                if phase == "merge" {
                    assert!(
                        std::fs::read(&artifact)
                            .unwrap()
                            .windows(user_content.len())
                            .any(|bytes| bytes == user_content)
                    );
                }
            }
        }
        assert_eq!(
            acl_script(
                &active,
                "(Get-Acl -LiteralPath $env:SHINE_ACL_TEST_PATH).Sddl"
            ),
            private_acl
        );
        let forced = SysInitPlanRequest {
            force_profile: true,
            ..request
        };
        let plan = runtime.plan_sys_init(forced.clone()).await.unwrap();
        runtime
            .init_sys_profile_approved(forced, &PlanApprovalV1::for_reviewed_plan(&plan).unwrap())
            .await
            .unwrap();
        assert_eq!(std::fs::read(&active).unwrap(), b"new\nkeep\n");
        assert_eq!(
            acl_script(
                &active,
                "(Get-Acl -LiteralPath $env:SHINE_ACL_TEST_PATH).Sddl"
            ),
            private_acl
        );
        let backups = std::fs::read_dir(active.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.to_string_lossy().contains(".bak."))
            .collect::<Vec<_>>();
        assert_eq!(backups.len(), 1);
        assert_eq!(std::fs::read(&backups[0]).unwrap(), b"user three\n");
        assert_eq!(
            acl_script(
                &backups[0],
                "(Get-Acl -LiteralPath $env:SHINE_ACL_TEST_PATH).Sddl"
            ),
            private_acl
        );
        // A clean merge must preserve the active ACL as well as force and conflict paths.
        std::fs::write(&active, b"new\nkeep\nuser addition\n").unwrap();
        let runtime = CoreRuntime::new(
            RealHost,
            runtime.context().clone(),
            snapshot(b"newer\nkeep\n"),
        );
        let request = SysInitPlanRequest {
            os_id: "windows".into(),
            force_profile: false,
        };
        let plan = runtime.plan_sys_init(request.clone()).await.unwrap();
        runtime
            .init_sys_profile_approved(request, &PlanApprovalV1::for_reviewed_plan(&plan).unwrap())
            .await
            .unwrap();
        assert_eq!(
            std::fs::read(&active).unwrap(),
            b"newer\nkeep\nuser addition\n"
        );
        assert_eq!(
            acl_script(
                &active,
                "(Get-Acl -LiteralPath $env:SHINE_ACL_TEST_PATH).Sddl"
            ),
            private_acl
        );
        assert!(!home.join("state/sys-manifest.toml").exists());
        std::fs::remove_dir_all(&home).unwrap();
    }
}
