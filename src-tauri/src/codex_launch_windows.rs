use std::{path::Path, process::{Command, Stdio}, time::{Duration, Instant}};
use std::os::windows::process::CommandExt;

fn activation_script(port: u16) -> String {
    // Activate MSIX through Windows so Codex retains its package identity.
    let script = r#"
$ErrorActionPreference = 'Stop'
$exe = [IO.Path]::GetFullPath($env:NUT_CODEX_EXECUTABLE)
$package = Get-AppxPackage -Name 'OpenAI.Codex*' | Where-Object {
    $root = $_.InstallLocation
    $root -and $exe.StartsWith($root.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)
} | Select-Object -First 1
if (-not $package) { Write-Output 'NOT_PACKAGED'; exit 0 }
$manifest = [xml](Get-Content -LiteralPath (Join-Path $package.InstallLocation 'AppxManifest.xml'))
$application = $manifest.Package.Applications.Application | Select-Object -First 1
if (-not $application.Id) { throw 'Codex package application identity is missing' }
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
[ComImport, Guid("2e941141-7f97-4756-ba1d-9decde894a3d"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface INutApplicationActivationManager {
    [PreserveSig] int ActivateApplication([MarshalAs(UnmanagedType.LPWStr)] string appId, [MarshalAs(UnmanagedType.LPWStr)] string arguments, uint options, out uint processId);
    [PreserveSig] int ActivateForFile([MarshalAs(UnmanagedType.LPWStr)] string appId, IntPtr items, [MarshalAs(UnmanagedType.LPWStr)] string verb, out uint processId);
    [PreserveSig] int ActivateForProtocol([MarshalAs(UnmanagedType.LPWStr)] string appId, IntPtr items, out uint processId);
}
public static class NutCodexActivation {
    public static uint Start(string appId, string arguments) {
        var type = Type.GetTypeFromCLSID(new Guid("45BA127D-10A8-46EA-8AB7-56EA9078943C"));
        object instance = Activator.CreateInstance(type);
        try {
            uint processId;
            int result = ((INutApplicationActivationManager)instance).ActivateApplication(appId, arguments, 0, out processId);
            Marshal.ThrowExceptionForHR(result);
            return processId;
        } finally { Marshal.ReleaseComObject(instance); }
    }
}
'@
$appId = "$($package.PackageFamilyName)!$($application.Id)"
$started = [NutCodexActivation]::Start($appId, '--remote-debugging-address=127.0.0.1 --remote-debugging-port=__PORT__ --lang=zh-CN')
Write-Output "ACTIVATED=$started"
"#;
    script.replace("__PORT__", &port.to_string())
}

/// Returns false only for an application outside a registered Codex MSIX package.
pub(crate) fn activate(executable: &Path, port: u16) -> Result<bool, String> {
    let mut command = Command::new("powershell.exe");
    command.args(["-NoProfile", "-NonInteractive", "-Command", &activation_script(port)])
        .env("NUT_CODEX_EXECUTABLE", executable)
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .creation_flags(0x08000000);
    for name in ["ELECTRON_RUN_AS_NODE", "ELECTRON_NO_ASAR", "NODE_OPTIONS"] {
        command.env_remove(name);
    }
    let mut child = command.spawn().map_err(|_| "无法启动 Windows 应用激活服务".to_string())?;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Windows 启动 Codex 超时，请关闭 Codex 错误窗口后重试。".into());
            }
        }
    }
    let output = child.wait_with_output().map_err(|_| "读取 Windows 应用启动结果失败".to_string())?;
    if !output.status.success() {
        return Err("Windows 未能激活 Codex 桌面应用。请先从开始菜单启动一次 Codex，确认安装完整后，再通过助手启动。".into());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    if text.lines().any(|line| line.starts_with("ACTIVATED=")) { return Ok(true); }
    if text.lines().any(|line| line.trim() == "NOT_PACKAGED") { return Ok(false); }
    Err("Windows 未返回 Codex 的启动结果，请关闭 Codex 错误窗口后重试。".into())
}

#[cfg(test)]
mod tests {
    #[test]
    fn activates_registered_package_with_chinese_control_arguments() {
        let script = super::activation_script(49885);
        assert!(script.contains("Get-AppxPackage"));
        assert!(script.contains("InstallLocation"));
        assert!(script.contains("ActivateApplication(appId, arguments"));
        assert!(script.contains("--remote-debugging-port=49885 --lang=zh-CN"));
        assert!(!script.contains("__PORT__"));
        assert!(!script.contains("Invoke-CommandInDesktopPackage"));
        assert!(!script.contains("taskkill"));
    }
}
