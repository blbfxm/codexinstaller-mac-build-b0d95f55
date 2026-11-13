use std::{
    fs,
    os::windows::process::CommandExt,
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc, LazyLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const SCRIPT: &str = include_str!("install_codex_msix.ps1");
static INSTALLING: LazyLock<Arc<AtomicBool>> = LazyLock::new(|| Arc::new(AtomicBool::new(false)));

struct InstallationLease(Arc<AtomicBool>);
impl Drop for InstallationLease {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

#[derive(serde::Deserialize)]
struct InstallStatus {
    stage: String,
    message: String,
}

#[derive(Clone, Copy)]
struct WaitLimits {
    authorization: Duration,
    step: Duration,
    total: Duration,
    poll: Duration,
}

fn wait_for_installation(
    mut child: Child,
    lease: InstallationLease,
    status_path: &Path,
    log_path: &Path,
    mut report: impl FnMut(&str),
    cancelled: impl Fn() -> bool,
    limits: WaitLimits,
) -> Result<ExitStatus, String> {
    let started = Instant::now();
    let mut step_started = started;
    let mut stage = String::new();
    let mut message = "正在启动 Windows 安装服务…".to_string();
    let mut last_report = started
        .checked_sub(Duration::from_secs(2))
        .unwrap_or(started);
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| format!("读取安装任务状态失败：{e}"))?
        {
            return Ok(status);
        }
        if let Ok(bytes) = fs::read(status_path) {
            if let Ok(status) = serde_json::from_slice::<InstallStatus>(&bytes) {
                if status.stage != stage {
                    stage = status.stage;
                    step_started = Instant::now();
                }
                message = status.message;
            }
        }
        if last_report.elapsed() >= Duration::from_secs(1) {
            report(&format!(
                "{}（本步骤已等待 {} 秒）",
                message,
                step_started.elapsed().as_secs()
            ));
            last_report = Instant::now();
        }
        let limit = if stage == "authorization" {
            limits.authorization
        } else {
            limits.step
        };
        let cancelled = cancelled();
        if cancelled || step_started.elapsed() >= limit || started.elapsed() >= limits.total {
            // Ending the wait must not kill Windows while it moves application data.
            // Keep the lease until the actual process exits so a retry cannot overlap.
            std::thread::spawn(move || {
                let _ = child.wait();
                drop(lease);
            });
            let reason = if cancelled {
                "已取消安装等待"
            } else {
                "等待 Windows 安装响应超时"
            };
            return Err(format!(
                "{reason}。当前步骤：{message}。Windows 可能仍在处理，请稍后重新检测；任务结束前无法重复安装。安装包已保留，无需重新下载。日志：{}",
                log_path.display()
            ));
        }
        std::thread::sleep(limits.poll);
    }
}

// Windows PowerShell 5.1 assumes the system ANSI encoding without a BOM.
fn script_bytes() -> Vec<u8> {
    let mut bytes = vec![0xef, 0xbb, 0xbf];
    bytes.extend_from_slice(SCRIPT.as_bytes());
    bytes
}

fn installation_error(
    code: Option<i32>,
    log: &str,
    stderr: &str,
    stdout: &str,
    log_path: &Path,
) -> String {
    if code == Some(1223) {
        return "已取消管理员授权，Codex 安装或迁移尚未完成。安装包已保留，请再次点击安装并允许 Windows 授权提示。".into();
    }
    // Earlier phases can fail as part of a successful recovery (for example,
    // creating a missing target volume). Report the final phase's failure.
    let final_log = log.rsplit_once("\n[").map_or(log, |(_, last)| last);
    let details = [final_log, stderr, stdout]
        .into_iter()
        .map(|text| text.trim().trim_start_matches('\u{feff}').trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if let Some((_, reason)) = details.split_once("UNSUPPORTED_WINDOWS:") {
        return reason.lines().next().unwrap_or_default().trim().to_string();
    }
    if let Some((_, reason)) = details.split_once("INSTALL_LOCATION_USER_MISMATCH:") {
        return format!(
            "{} 安装包已保留。日志：{}",
            reason.lines().next().unwrap_or_default().trim(),
            log_path.display()
        );
    }
    if details.contains("CODEX_STAGE_LOCATION_FAILED:") {
        return format!(
            "Windows 没有把 Codex 安装文件暂存到所选磁盘，已停止本次安装，未继续注册 C 盘中的旧版本。请确认所选磁盘在线、为 NTFS 且空间充足后重试。安装包已保留。错误日志：{}",
            log_path.display()
        );
    }
    if details.contains("CODEX_STAGE_REGISTRATION_FAILED:") {
        return format!(
            "Codex 安装文件已暂存到所选磁盘，但 Windows 注册后仍指向其他位置。本次安装已停止，未把 C 盘位置误报为成功。请保留日志以便排查。安装包已保留。错误日志：{}",
            log_path.display()
        );
    }
    if details.contains("CODEX_APP_DATA_VOLUME_CONFLICT:") {
        return format!(
            "Codex 的旧应用数据仍绑定在原磁盘，Windows 因此拒绝安装到所选磁盘。助手已在备份后清理旧绑定，请重新点击安装；会话、技能与插件会从备份恢复。安装包已保留。错误日志：{}",
            log_path.display()
        );
    }
    if details.contains("CODEX_MIGRATION_") || log.contains("CODEX_MIGRATION_BACKUP:") {
        if details.contains("CODEX_MIGRATION_BACKUP_FAILED:") {
            return format!(
                "Codex 原数据备份校验失败，尚未卸载旧应用。请检查磁盘空间和文件占用后重试。安装包已保留。错误日志：{}",
                log_path.display()
            );
        }
        if details.contains("CODEX_MIGRATION_OTHER_USERS:") {
            return format!(
                "Codex 仍安装在其他 Windows 用户账户中。为保护其他用户，助手没有删除旧版本；请将 Codex 安装位置改回原磁盘后重试。安装包已保留。错误日志：{}",
                log_path.display()
            );
        }
        let backup = log
            .split_once("CODEX_MIGRATION_BACKUP:")
            .and_then(|(_, path)| path.lines().next())
            .map(str::trim)
            .filter(|path| !path.is_empty());
        let location = backup.map_or_else(
            || "请保留错误日志以便排查。".to_string(),
            |path| format!("旧 Codex 数据备份仍保留在：{path}。"),
        );
        return format!(
            "Codex 未能完成卸载重装到所选磁盘。{location}请重试安装；助手会沿用已下载的安装包并继续恢复数据。错误日志：{}",
            log_path.display()
        );
    }
    // Report the latest failure, not the earlier system-package error that led
    // into recovery. Access denied after recovery is not another 0x80073D0B.
    if details.to_ascii_lowercase().contains("0x80070005") {
        return format!(
            "Windows 拒绝访问 Codex 安装文件（0x80070005）。请允许当前 Windows 用户的管理员授权，并确认 Codex 已关闭。安装包已保留，无需重新下载。日志：{}",
            log_path.display()
        );
    }
    if details.to_ascii_lowercase().contains("0x80073d0b") {
        return format!(
            "Windows 将 Codex 识别为系统预装应用，无法移动到所选磁盘（0x80073D0B）。已尝试修复预装登记，但未完成移动；请保留日志供排查，或选择 Codex 当前所在磁盘使用。安装包已保留，无需重新下载。错误日志：{}",
            log_path.display()
        );
    }
    if details.contains("INSTALL_LOCATION_MOVE_FAILED:")
        || details.contains("INSTALL_LOCATION_REPAIR_FAILED:")
    {
        return format!(
            "Codex 已注册，但未能迁移到所选磁盘。请关闭 Codex，并在 Windows“设置 → 应用”中尝试移动；如需先使用，可将助手安装位置改回应用当前所在磁盘。安装包已保留。错误日志：{}",
            log_path.display()
        );
    }
    if details.contains("INSTALL_LOCATION_FAILED:") {
        return format!(
            "无法将 Codex 安装到设置的磁盘，请检查磁盘是否在线并允许管理员授权。安装包已保留。错误日志：{}",
            log_path.display()
        );
    }
    if details.to_ascii_lowercase().contains("0x80073d02") {
        return format!(
            "Windows 检测到 Codex 仍在运行，无法替换应用文件。请退出 Codex 后重试；安装包已保留，无需重新下载。错误日志：{}",
            log_path.display()
        );
    }
    if details.to_ascii_lowercase().contains("0x80073d28") {
        return format!(
            "此版本 Codex 的后台服务注册需要管理员授权。请在 Windows 授权提示中允许安装；如果使用了其他管理员账户，请改用当前登录账户对应的管理员授权后重试。安装包已保留，无需重新下载。错误日志：{}",
            log_path.display()
        );
    }
    if details.to_ascii_lowercase().contains("0x80073cf9") {
        return format!(
            "Windows 应用数据与目标磁盘不一致（0x80073CF9），未能完成迁移。请检查日志中的数据卷和目标卷；如需继续安装，可先选择 Codex 当前所在的原磁盘。安装包已保留，无需重新下载。错误日志：{}",
            log_path.display()
        );
    }
    // Full PowerShell diagnostics stay in the file, not in an unbounded UI panel.
    let reason = if let Some(start) = details.find("0x") {
        let code: String = details[start..].chars().take(10).collect();
        format!("Windows 错误 {code}，请检查系统版本、应用依赖和安装权限。")
    } else if details.is_empty() {
        "Windows 未返回详细错误，请检查系统版本与应用安装策略。".into()
    } else {
        details
            .lines()
            .next()
            .unwrap_or_default()
            .chars()
            .take(180)
            .collect::<String>()
    };
    format!(
        "Codex 安装失败（退出码：{}）。{} 安装包已保留，重试时无需重新下载。错误日志：{}",
        code.map_or_else(|| "未知".into(), |value| value.to_string()),
        reason,
        log_path.display(),
    )
}

pub fn install(
    package: &Path,
    license: &Path,
    location: Option<&Path>,
    report: impl FnMut(&str),
    cancelled: impl Fn() -> bool,
) -> Result<(), String> {
    if INSTALLING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err(
            "上一次 Windows 安装任务仍在处理，请稍后重新检测。安装包已保留，请勿重复安装。".into(),
        );
    }
    let lease = InstallationLease(Arc::clone(&INSTALLING));
    let directory = package.parent().ok_or("安装包路径无效")?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let attempt = directory.join(format!("codex-install-{}-{stamp}", std::process::id()));
    fs::create_dir(&attempt).map_err(|e| format!("无法创建本次安装日志目录：{e}"))?;
    let script_path = attempt.join("install-chatgpt-offline.ps1");
    let error_path = attempt.join("install-chatgpt-offline-error.txt");
    let status_path = error_path.with_extension("txt.status.json");
    let stdout_path = attempt.join("stdout.txt");
    let stderr_path = attempt.join("stderr.txt");
    // Truncate diagnostics before every attempt; never report an earlier failure.
    fs::write(&error_path, []).map_err(|error| format!("无法创建安装日志：{error}"))?;
    fs::write(&script_path, script_bytes())
        .map_err(|error| format!("无法写入安装脚本：{error}"))?;
    let powershell = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "C:\\Windows".into())
        .join("System32\\WindowsPowerShell\\v1.0\\powershell.exe");
    let mut command = Command::new(powershell);
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&script_path)
        .arg("-PackagePath")
        .arg(package)
        .arg("-LicensePath")
        .arg(license)
        .arg("-ErrorPath")
        .arg(&error_path)
        .arg("-StatusPath")
        .arg(&status_path);
    if let Some(location) = location {
        command.arg("-InstallLocation").arg(location);
    }
    // Use files instead of pipes so verbose Windows diagnostics cannot fill a
    // pipe and block installation while the UI waits for process completion.
    command.stdout(Stdio::from(
        fs::File::create(&stdout_path).map_err(|e| e.to_string())?,
    ));
    command.stderr(Stdio::from(
        fs::File::create(&stderr_path).map_err(|e| e.to_string())?,
    ));
    let child = command
        .creation_flags(0x08000000)
        .spawn()
        .map_err(|error| format!("无法启动 Windows PowerShell，安装包已保留：{error}"))?;
    let status = wait_for_installation(
        child,
        lease,
        &status_path,
        &error_path,
        report,
        cancelled,
        WaitLimits {
            authorization: Duration::from_secs(120),
            step: Duration::from_secs(600),
            total: Duration::from_secs(1200),
            poll: Duration::from_millis(250),
        },
    )?;
    if !status.success() {
        let log = fs::read_to_string(&error_path).unwrap_or_default();
        let stderr = fs::read_to_string(&stderr_path).unwrap_or_default();
        let stdout = fs::read_to_string(&stdout_path).unwrap_or_default();
        // A script parsing/policy error can occur before it opens the log.
        let details = installation_error(status.code(), &log, &stderr, &stdout, &error_path);
        let _ = fs::write(&error_path, format!("{log}\n{stderr}\n{stdout}"));
        return Err(details);
    }
    let _ = fs::remove_file(&script_path);
    let _ = fs::remove_file(&error_path);
    let _ = fs::remove_file(&status_path);
    let _ = fs::remove_file(error_path.with_extension("txt.helper-error.json"));
    let _ = fs::remove_file(&stdout_path);
    let _ = fs::remove_file(&stderr_path);
    let _ = fs::remove_dir(&attempt);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_is_utf8_with_bom_and_parses_in_windows_powershell() {
        let path = std::env::temp_dir().join(format!("codex-msix-中文-{}.ps1", std::process::id()));
        let bytes = script_bytes();
        assert!(bytes.starts_with(&[0xef, 0xbb, 0xbf]));
        fs::write(&path, bytes).unwrap();
        // Parse only: do not install packages or trigger UAC on the development PC.
        let output = Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command",
            "$e=$null; $t=$null; [System.Management.Automation.Language.Parser]::ParseFile($env:CODEX_MSIX_PARSE_PATH,[ref]$t,[ref]$e) | Out-Null; if ($e.Count) { $e | Out-String | Write-Output; exit 1 }"])
            .env("CODEX_MSIX_PARSE_PATH", &path).creation_flags(0x08000000).output().unwrap();
        let _ = fs::remove_file(path);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }

    #[test]
    fn selected_volume_is_staged_before_machine_license_provisioning() {
        let stage = SCRIPT
            .find("Add-AppxPackage -Stage -Path $PackagePath -Volume (Get-InstallVolume)")
            .unwrap();
        let provision = SCRIPT
            .find("Add-AppxProvisionedPackage -Online -PackagePath $PackagePath")
            .unwrap();
        assert!(
            stage < provision,
            "Codex must be staged on the selected disk before Windows registers its offline license"
        );
    }

    #[test]
    fn selected_volume_registers_the_staged_manifest_and_never_falls_back_to_c() {
        assert!(SCRIPT.contains("Register-StagedCodexForCurrentUser $staged"));
        assert!(SCRIPT.contains("Path = $StagedPackage.ManifestPath"));
        assert!(SCRIPT.contains("CODEX_STAGE_LOCATION_FAILED:"));
        assert!(!SCRIPT.contains("$sourceParameters = @{ Path=$PackagePath"));
    }

    #[test]
    fn relocation_errors_explain_when_windows_stages_on_the_wrong_volume() {
        let path = Path::new("C:\\Temp\\install-error.txt");
        let error = installation_error(
            Some(1),
            "CODEX_STAGE_LOCATION_FAILED: Windows reused the C: staging payload. This sentence is intentionally long enough to prove the actionable message is selected.",
            "",
            "",
            path,
        );
        assert!(error.contains("没有把 Codex 安装文件暂存到所选磁盘"));
        assert!(error.contains("未继续注册 C 盘"));
        assert!(!error.contains("intentionally long enough"));
    }

    #[test]
    fn only_explicit_uac_cancellation_is_reported_as_cancelled() {
        let path = Path::new("C:\\Temp\\install-error.txt");
        let error = installation_error(Some(1), "", "ParserError: broken script", "", path);
        assert!(error.contains("ParserError"));
        assert!(!error.contains("已取消管理员授权"));
        let error = installation_error(
            Some(1),
            "HRESULT: 0x80073CF3 missing dependency",
            "",
            "",
            path,
        );
        assert!(error.contains("0x80073CF3"));
        assert!(installation_error(Some(1223), "", "", "", path).contains("已取消管理员授权"));
    }

    #[test]
    fn incompatible_windows_has_a_short_actionable_error() {
        let error = installation_error(
            Some(1603),
            "UNSUPPORTED_WINDOWS: 当前系统版本 10.0.18363.0，安装包要求 Windows 10.0.19041.0 或更高版本。请升级 Windows 后重试；安装包已保留。\nraw diagnostics",
            "",
            "",
            Path::new("error.txt"),
        );
        assert!(error.contains("19041"));
        assert!(error.contains("升级 Windows"));
        assert!(!error.contains("raw diagnostics"));
        let error = installation_error(
            Some(1),
            &"PowerShell diagnostic\n".repeat(1000),
            "",
            "",
            Path::new("error.txt"),
        );
        assert!(error.chars().count() < 400);
    }

    #[test]
    fn final_deployment_error_is_not_hidden_by_an_earlier_volume_error() {
        let log = "[Install for current user]\r\nINSTALL_LOCATION_FAILED: target needs administrator authorization\r\n\r\n[Complete installation]\r\nHRESULT: 0x80073CF9: app data is already installed on C:";
        let error = installation_error(Some(1), log, "", "", Path::new("error.txt"));
        assert!(error.contains("0x80073CF9"));
        assert!(error.contains("原磁盘"));
        assert!(!error.contains("允许管理员授权"));
        let log = log.replace("0x80073CF9", "0x80070005");
        let error = installation_error(Some(1), &log, "", "", Path::new("error.txt"));
        assert!(error.contains("0x80070005"));
        assert!(!error.contains("允许管理员授权"));
    }

    #[test]
    fn packaged_service_and_running_package_errors_have_specific_recovery_messages() {
        let path = Path::new("C:\\Temp\\install-error.txt");
        let error = installation_error(
            Some(1),
            "[Install for current user]\n0x80073D28 packaged service requires admin\n[Register on selected installation disk]\n0x80073CF9 app data already on C\n[Complete installation]\n0x80073D02 package in use",
            "",
            "",
            path,
        );
        assert!(
            error.contains("仍在运行"),
            "the final blocking error must take precedence"
        );
        assert!(!error.contains("管理员授权提示"));
        let error = installation_error(
            Some(1),
            "[Install for current user]\n0x80073D28 packaged service requires admin",
            "",
            "",
            path,
        );
        assert!(error.contains("后台服务注册需要管理员授权"));
        assert!(error.contains("无需重新下载"));
    }

    #[test]
    fn failed_move_is_reported_as_incomplete_installation() {
        let log = "[Register on selected installation disk]\r\nHRESULT 0x80073CF9\r\n[Install for current user]\r\nINSTALL_LOCATION_MOVE_FAILED: HRESULT 0x80070005: move denied";
        let error = installation_error(Some(1), log, "", "", Path::new("error.txt"));
        assert!(error.contains("0x80070005"));
        assert!(error.contains("拒绝访问"));
        assert!(!error.contains("0x80073CF9"));
    }

    #[test]
    fn system_provisioning_failure_explains_the_actual_move_blocker() {
        let log = "[Install for current user]\r\nINSTALL_LOCATION_FAILED: wrong disk\r\n[Complete installation]\r\nHRESULT 0x80073D0B: Codex is part of the system";
        let error = installation_error(Some(1), log, "", "", Path::new("error.txt"));
        assert!(error.contains("0x80073D0B"));
        assert!(error.contains("系统预装"));
        assert!(error.contains("无需重新下载"));
        let error = installation_error(
            Some(1),
            "[Install for current user]\nINSTALL_LOCATION_REPAIR_FAILED: denied",
            "",
            "",
            Path::new("error.txt"),
        );
        assert!(error.contains("未能迁移"));
        assert!(!error.contains("0x80073D0B"));
        assert!(!error.contains("检查磁盘是否在线"));
    }

    #[test]
    fn repair_access_denied_does_not_reuse_the_initial_system_app_error() {
        let log = "[Move package to selected installation disk]\nHRESULT 0x80073D0B: part of system\n[Install for current user]\nINSTALL_LOCATION_REPAIR_FAILED: HRESULT 0x80070005: access denied opening loose package";
        let error = installation_error(Some(1), log, "", "", Path::new("error.txt"));
        assert!(error.contains("0x80070005"));
        assert!(!error.contains("0x80073D0B"));
    }

    #[test]
    fn relocation_failure_points_to_preserved_user_data_backup() {
        let log = "CODEX_MIGRATION_BACKUP: C:\\Users\\Test\\CodexMoveBackup\\0123456789abcdef0123456789abcdef\r\n[Install for current user]\r\nCODEX_MIGRATION_RESTORE_FAILED: data restore failed";
        let error = installation_error(Some(1), log, "", "", Path::new("error.txt"));
        assert!(error.contains("数据备份仍保留"));
        assert!(error.contains("CodexMoveBackup\\0123456789abcdef0123456789abcdef"));
        assert!(error.contains("沿用已下载的安装包"));
    }

    #[test]
    fn relocation_never_removes_codex_used_by_another_windows_account() {
        let log = "[Install for current user]\r\nCODEX_MIGRATION_REMOVE_FAILED: CODEX_MIGRATION_OTHER_USERS: protect another user";
        let error = installation_error(Some(1), log, "", "", Path::new("error.txt"));
        assert!(error.contains("其他 Windows 用户"));
        assert!(error.contains("没有删除旧版本"));
        assert!(error.contains("改回原磁盘"));
    }

    fn monitor_fixture(name: &str) -> (std::path::PathBuf, Child) {
        let root = std::env::temp_dir().join(format!(
            "codex-install-monitor-{name}-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let status = root.join("status.json");
        let _ = fs::remove_file(&status);
        let child = Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command",
            "[IO.File]::WriteAllText($env:CODEX_INSTALL_TEST_STATUS, '{\"stage\":\"moving\",\"message\":\"moving Codex to F:\"}'); Start-Sleep -Milliseconds 600"])
            .env("CODEX_INSTALL_TEST_STATUS", &status).stdout(Stdio::null()).stderr(Stdio::null())
            .creation_flags(0x08000000).spawn().unwrap();
        let started = Instant::now();
        while !status.is_file() {
            assert!(started.elapsed() < Duration::from_secs(20));
            std::thread::sleep(Duration::from_millis(10));
        }
        (root, child)
    }

    #[test]
    fn installation_timeout_and_cancel_keep_the_lease_until_windows_finishes() {
        for cancel in [false, true] {
            let (root, child) = monitor_fixture(if cancel { "cancel" } else { "timeout" });
            let running = Arc::new(AtomicBool::new(true));
            let mut updates = vec![];
            let error = wait_for_installation(
                child,
                InstallationLease(Arc::clone(&running)),
                &root.join("status.json"),
                &root.join("error.txt"),
                |message| updates.push(message.to_string()),
                || cancel,
                WaitLimits {
                    authorization: Duration::from_millis(60),
                    step: Duration::from_millis(60),
                    total: Duration::from_secs(2),
                    poll: Duration::from_millis(10),
                },
            )
            .unwrap_err();
            assert!(error.contains(if cancel {
                "已取消安装等待"
            } else {
                "超时"
            }));
            assert!(error.contains("moving Codex to F:"));
            assert!(
                updates
                    .iter()
                    .any(|message| message.contains("本步骤已等待"))
            );
            assert!(
                running.load(Ordering::SeqCst),
                "retry must stay blocked while the original Windows task is alive"
            );
            let started = Instant::now();
            while running.load(Ordering::SeqCst) {
                assert!(started.elapsed() < Duration::from_secs(5));
                std::thread::sleep(Duration::from_millis(10));
            }
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn completed_installation_releases_the_lease_and_reports_its_step() {
        let (root, child) = monitor_fixture("complete");
        let running = Arc::new(AtomicBool::new(true));
        let mut updates = vec![];
        let status = wait_for_installation(
            child,
            InstallationLease(Arc::clone(&running)),
            &root.join("status.json"),
            &root.join("error.txt"),
            |message| updates.push(message.to_string()),
            || false,
            WaitLimits {
                authorization: Duration::from_secs(5),
                step: Duration::from_secs(5),
                total: Duration::from_secs(5),
                poll: Duration::from_millis(10),
            },
        )
        .unwrap();
        assert!(status.success());
        assert!(!running.load(Ordering::SeqCst));
        assert!(
            updates
                .iter()
                .any(|message| message.contains("moving Codex to F:"))
        );
        fs::remove_dir_all(root).unwrap();
    }
}
