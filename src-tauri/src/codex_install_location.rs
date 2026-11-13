//! Per-assistant preference. Never changes the Windows-wide default Appx volume.
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};
use tauri::Manager;

#[derive(Default, Deserialize, Serialize)]
struct Preference {
    path: String,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct Volume {
    path: String,
    label: String,
}

#[derive(Serialize)]
pub(crate) struct Options {
    path: String,
    volumes: Vec<Volume>,
}

fn preference_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app.path().app_config_dir().map_err(|e| format!("无法读取安装位置设置：{e}"))?
        .join("codex-install-location.json"))
}

fn load(path: &std::path::Path) -> Result<String, String> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice::<Preference>(&bytes).map(|p| p.path)
            .map_err(|_| "Codex 安装位置设置损坏，请重新保存安装位置".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("无法读取 Codex 安装位置：{e}")),
    }
}

fn valid_store_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && &bytes[1..3] == b":\\"
        && (path[3..].eq_ignore_ascii_case("WindowsApps")
            || path[3..].eq_ignore_ascii_case("Program Files\\WindowsApps"))
}

#[cfg(target_os = "windows")]
fn volumes() -> Result<Vec<Volume>, String> {
    use std::{os::windows::process::CommandExt, process::Command};
    let output = Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", r#"
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
$ErrorActionPreference = 'Stop'
$stores = @()
try { $stores = @(Get-AppxVolume -ErrorAction Stop) } catch {}
$items = @([IO.DriveInfo]::GetDrives() | Where-Object { $_.IsReady -and $_.DriveType -eq 'Fixed' -and $_.DriveFormat -eq 'NTFS' } | ForEach-Object {
    $drive = $_
    $store = $stores | Where-Object { $_.PackageStorePath -and $_.PackageStorePath.StartsWith($drive.Name, [StringComparison]::OrdinalIgnoreCase) } | Select-Object -First 1
    $path = if ($store) { $store.PackageStorePath } elseif ($drive.Name -eq ($env:SystemDrive + '\')) { Join-Path $drive.Name 'Program Files\WindowsApps' } else { Join-Path $drive.Name 'WindowsApps' }
    [PSCustomObject]@{ path=$path; label=('{0} · 可用 {1:N1} GB' -f $drive.Name, ($drive.AvailableFreeSpace / 1GB)) }
})
ConvertTo-Json -InputObject $items -Compress
"#]).creation_flags(0x08000000).output().map_err(|e| format!("无法读取安装磁盘：{e}"))?;
    if !output.status.success() { return Err("无法读取可用安装磁盘，请检查 Windows 应用安装服务".into()); }
    let mut items: Vec<Volume> = serde_json::from_slice(&output.stdout)
        .map_err(|_| "安装磁盘信息无效，请重试".to_string())?;
    items.retain(|v| valid_store_path(&v.path));
    Ok(items)
}

#[cfg(not(target_os = "windows"))]
fn volumes() -> Result<Vec<Volume>, String> { Ok(Vec::new()) }

pub(crate) fn selected(app: &tauri::AppHandle) -> Result<Option<PathBuf>, String> {
    let path = load(&preference_path(app)?)?;
    if path.is_empty() { return Ok(None); }
    resolve(&path, &volumes()?)
}

fn resolve(path: &str, available: &[Volume]) -> Result<Option<PathBuf>, String> {
    if path.is_empty() { return Ok(None); }
    if !valid_store_path(path) || !available.iter().any(|v| v.path.eq_ignore_ascii_case(path)) {
        return Err(format!("设置的 Codex 安装位置不可用：{path}。请在“安装配置选项”中重新选择安装磁盘。"));
    }
    Ok(Some(path.into()))
}

#[tauri::command]
pub async fn codex_install_options(app: tauri::AppHandle) -> Result<Options, String> {
    tauri::async_runtime::spawn_blocking(move || Ok(Options {
        path: load(&preference_path(&app)?)?, volumes: volumes()?,
    })).await.map_err(|e| format!("读取安装位置失败：{e}"))?
}

#[tauri::command]
pub async fn save_codex_install_location(app: tauri::AppHandle, path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = path.trim().trim_end_matches('\\').to_string();
        if !path.is_empty() && (!valid_store_path(&path) || !volumes()?.iter().any(|v| v.path.eq_ignore_ascii_case(&path))) {
            return Err("请选择可用的 NTFS 本地磁盘；Codex 必须安装到 Windows 管理的应用目录".into());
        }
        let bytes = serde_json::to_vec(&Preference { path: path.clone() }).map_err(|e| e.to_string())?;
        super::provider_config::commit(vec![(preference_path(&app)?, bytes)])?;
        Ok(path)
    }).await.map_err(|e| format!("保存安装位置失败：{e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_windows_managed_local_store_paths_are_accepted() {
        for path in [r"D:\WindowsApps", r"C:\Program Files\WindowsApps", r"e:\windowsapps"] { assert!(valid_store_path(path), "{path}"); }
        for path in [r"D:\Codex", r"D:\WindowsApps\..\Codex", r"\\server\WindowsApps", r"D:\WindowsApps\app", "", "中文目录", r"D:/WindowsApps"] { assert!(!valid_store_path(path), "{path}"); }
    }
    #[test]
    fn unavailable_saved_location_never_falls_back_to_system_disk() {
        let available = vec![Volume { path: r"D:\WindowsApps".into(), label: "D".into() }];
        assert_eq!(resolve("", &available).unwrap(), None);
        assert_eq!(resolve(r"d:\windowsapps", &available).unwrap(), Some(PathBuf::from(r"d:\windowsapps")));
        assert!(resolve(r"E:\WindowsApps", &available).is_err());
        assert!(resolve(r"D:\WindowsApps", &[]).is_err());
        assert!(resolve(r"D:\Codex", &available).is_err());
    }
    #[test]
    fn missing_preference_defaults_to_windows_but_corrupt_settings_do_not() {
        let root = std::env::temp_dir().join(format!("codex-install-options-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(&root).unwrap();
        let file = root.join("setting.json");
        assert_eq!(load(&file).unwrap(), "");
        fs::write(&file, br#"{"path":"D:\\WindowsApps"}"#).unwrap();
        assert_eq!(load(&file).unwrap(), r"D:\WindowsApps");
        fs::write(&file, b"broken").unwrap();
        assert!(load(&file).is_err());
        fs::remove_file(&file).unwrap();
        fs::remove_dir(&root).unwrap();
    }
}
