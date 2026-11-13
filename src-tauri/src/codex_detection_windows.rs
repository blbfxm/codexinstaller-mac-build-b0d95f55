use std::{path::PathBuf, process::{Command, Stdio}, time::{Duration, Instant}};
use std::os::windows::process::CommandExt;

pub(crate) const SCRIPT: &str = include_str!("discover_codex_windows.ps1");

#[derive(serde::Deserialize)]
pub(crate) struct Installation {
    pub root: PathBuf,
    pub executable: PathBuf,
    pub version: String,
}

pub(crate) fn discover() -> Option<Installation> {
    // Use Windows PowerShell (Appx is unavailable in some PATH-selected PS hosts),
    // and explicitly emit UTF-8 so paths containing Chinese are not corrupted.
    let powershell = PathBuf::from(std::env::var_os("SystemRoot")?)
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let script = format!("[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)\n{SCRIPT}\nFind-CodexInstallation | ConvertTo-Json -Compress");
    let mut child = Command::new(powershell)
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null())
        .creation_flags(0x08000000).spawn().ok()?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => { let _ = child.kill(); let _ = child.wait(); return None; }
        }
    }
    let output = child.wait_with_output().ok()?;
    if !output.status.success() { return None; }
    let result: Installation = serde_json::from_slice(&output.stdout).ok()?;
    result.executable.is_file().then_some(result)
}
