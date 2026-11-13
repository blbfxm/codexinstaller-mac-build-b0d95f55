use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU16, Ordering},
    },
    time::{Duration, Instant},
};
use tauri::{Emitter, Manager, State};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::Mutex as AsyncMutex;
mod account_portal;
mod codex_chinese;
#[cfg(target_os = "windows")]
mod codex_detection_windows;
mod codex_install_location;
#[cfg(target_os = "windows")]
mod codex_launch_windows;
#[cfg(target_os = "windows")]
mod codex_msix;
mod codex_permissions;
mod codex_reset;
mod folder_picker;
mod gateway;
pub mod image_plugin;
mod installer_cache;
#[cfg(any(target_os = "macos", test))]
mod macos_apps;
#[cfg(any(target_os = "macos", test))]
mod macos_localization;
#[cfg(any(target_os = "macos", test))]
mod macos_proxy;
mod model_catalog;
mod plugin_market;
mod provider_config;
mod public_plugin_market;
mod skill_archive;
mod usage;
mod vector_plugin;
mod video_plugin;
mod wallet;
mod wechat;
static PROVIDER_CONFIG_LOCK: Mutex<()> = Mutex::new(());
static CODEX_CONTROL_PORT: AtomicU16 = AtomicU16::new(0);

#[derive(Default)]
struct DownloadControl {
    paused: AtomicBool,
    cancelled: AtomicBool,
}

#[tauri::command]
fn set_download_paused(control: State<'_, DownloadControl>, paused: bool) {
    control.paused.store(paused, Ordering::SeqCst);
}

#[tauri::command]
fn set_download_cancelled(control: State<'_, DownloadControl>, cancelled: bool) {
    control.cancelled.store(cancelled, Ordering::SeqCst);
}

#[derive(serde::Serialize, Clone)]
struct DownloadProgress {
    product: String,
    phase: &'static str,
    downloaded: u64,
    total: Option<u64>,
    bytes_per_second: u64,
    resumable: bool,
    status: String,
}

struct DownloadProgressReporter {
    emit: Box<dyn Fn(DownloadProgress) + Send>,
    product: String,
    phase: &'static str,
    total: Option<u64>,
    resumable: bool,
    status: String,
    last_report: Instant,
    last_bytes: u64,
}

impl DownloadProgressReporter {
    fn new(app: tauri::AppHandle, product: &str, total: Option<u64>, resumable: bool) -> Self {
        Self {
            emit: Box::new(move |progress| {
                let _ = app.emit("installer-download-progress", progress);
            }),
            product: product.to_string(),
            phase: "download",
            total,
            resumable,
            status: String::new(),
            last_report: Instant::now(),
            last_bytes: 0,
        }
    }

    fn report(&mut self, downloaded: u64, force: bool) {
        let elapsed = self.last_report.elapsed();
        if !force && elapsed < Duration::from_millis(500) {
            return;
        }
        let bytes_per_second = if elapsed.is_zero() {
            0
        } else {
            (downloaded.saturating_sub(self.last_bytes) as u128 * 1_000_000_000
                / elapsed.as_nanos())
            .min(u64::MAX as u128) as u64
        };
        let progress = DownloadProgress {
            product: self.product.clone(),
            phase: self.phase,
            downloaded,
            total: self.total,
            bytes_per_second,
            resumable: self.resumable,
            status: self.status.clone(),
        };
        (self.emit)(progress);
        self.last_report = Instant::now();
        self.last_bytes = downloaded;
    }
}

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
struct WindowsProxySnapshot {
    enabled: Option<String>,
    server: Option<String>,
    bypass: Option<String>,
    #[serde(default)]
    assistant_server: Option<String>,
    #[serde(default)]
    mode: Option<String>,
}

fn run_hidden(mut command: Command) -> Result<std::process::Output, String> {
    #[cfg(target_os = "windows")]
    command.creation_flags(0x08000000);
    command.output().map_err(|error| error.to_string())
}

#[cfg(target_os = "windows")]
fn registry_value(name: &str) -> Option<String> {
    let output = run_hidden({
        let mut command = Command::new("reg.exe");
        command.args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings",
            "/v",
            name,
        ]);
        command
    })
    .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| {
            let marker = if name == "ProxyEnable" {
                "REG_DWORD"
            } else {
                "REG_SZ"
            };
            line.find(marker)
                .map(|index| line[index + marker.len()..].trim().to_string())
        })
}

#[cfg(target_os = "windows")]
fn set_registry_value(name: &str, value_type: &str, value: &str) -> Result<(), String> {
    let output = run_hidden({
        let mut command = Command::new("reg.exe");
        command.args([
            "add",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings",
            "/v",
            name,
            "/t",
            value_type,
            "/d",
            value,
            "/f",
        ]);
        command
    })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

#[cfg(target_os = "windows")]
fn delete_registry_value(name: &str) -> Result<(), String> {
    let output = run_hidden({
        let mut command = Command::new("reg.exe");
        command.args([
            "delete",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings",
            "/v",
            name,
            "/f",
        ]);
        command
    })?;
    if output.status.success() || registry_value(name).is_none() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

#[cfg(target_os = "windows")]
fn refresh_windows_proxy() {
    let _ = run_hidden({
        let mut command = Command::new("RUNDLL32.EXE");
        command.args(["USER32.DLL,UpdatePerUserSystemParameters"]);
        command
    });
}

fn proxy_snapshot_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_local_data_dir()
        .map_err(|error| error.to_string())?
        .join("v2rayN");
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    Ok(dir.join(if cfg!(target_os = "macos") {
        "macos-system-proxy.json"
    } else {
        "system-proxy.json"
    }))
}

fn update_proxy_tray(app: &tauri::AppHandle, enabled: bool) {
    let Some(tray) = app.tray_by_id("codex-installer-tray") else {
        return;
    };
    if let Some(original) = app.default_window_icon() {
        let icon = if enabled {
            let mut rgba = original.rgba().to_vec();
            for pixel in rgba.chunks_exact_mut(4) {
                pixel[..3].copy_from_slice(&[22, 163, 74]);
            }
            tauri::image::Image::new_owned(rgba, original.width(), original.height())
        } else {
            original.clone()
        };
        if let Err(error) = tray.set_icon(Some(icon)) {
            eprintln!("Failed to update proxy tray icon: {error}");
        }
    }
    let tooltip = if enabled {
        "Codex 使用助手 - 网络代理已开启"
    } else {
        "Codex 使用助手 - 网络代理已关闭"
    };
    if let Err(error) = tray.set_tooltip(Some(tooltip)) {
        eprintln!("Failed to update proxy tray tooltip: {error}");
    }
}

#[tauri::command]
async fn system_proxy_state(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || system_proxy_state_impl(app))
        .await
        .map_err(|e| format!("读取系统代理状态失败：{e}"))?
}

fn system_proxy_state_impl(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    #[cfg(target_os = "windows")]
    {
        let raw_enabled = registry_value("ProxyEnable");
        let enabled = raw_enabled.as_deref() == Some("0x1") || raw_enabled.as_deref() == Some("1");
        let server = registry_value("ProxyServer").unwrap_or_default();
        let snapshot_path = proxy_snapshot_path(&app)?;
        let snapshot = fs::read(&snapshot_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<WindowsProxySnapshot>(&bytes).ok());
        let active = snapshot.as_ref().is_some_and(|snapshot| {
            if let Some(assistant_server) = snapshot.assistant_server.as_deref() {
                return (enabled && server == assistant_server)
                    || (snapshot.mode.as_deref() == Some("clear") && !enabled);
            }
            (snapshot.server.is_none()
                && snapshot
                    .enabled
                    .as_deref()
                    .is_some_and(|value| value == "0x0" || value == "0")
                && !enabled)
                || (enabled
                    && snapshot
                        .server
                        .as_deref()
                        .is_some_and(|value| value == server))
        });
        update_proxy_tray(&app, active && enabled);
        return Ok(serde_json::json!({
            "supported": true,
            "enabled": active && enabled,
            "assistantManaged": active,
            "systemEnabled": enabled,
            "server": server,
        }));
    }
    #[cfg(target_os = "macos")]
    {
        let state = macos_proxy::state(&proxy_snapshot_path(&app)?)?;
        update_proxy_tray(&app, state["enabled"].as_bool().unwrap_or(false));
        return Ok(state);
    }
    #[allow(unreachable_code)]
    Ok(serde_json::json!({
        "supported": false,
        "enabled": false,
        "assistantManaged": false,
        "systemEnabled": false,
        "server": "",
    }))
}

#[tauri::command]
async fn set_system_proxy(
    app: tauri::AppHandle,
    enabled: bool,
    port: u16,
) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || set_system_proxy_impl(app, enabled, port))
        .await
        .map_err(|e| format!("切换系统代理失败：{e}"))?
}

fn set_system_proxy_impl(
    app: tauri::AppHandle,
    enabled: bool,
    port: u16,
) -> Result<serde_json::Value, String> {
    if port == 0 {
        return Err("代理端口无效".into());
    }
    #[cfg(target_os = "windows")]
    {
        let snapshot_path = proxy_snapshot_path(&app)?;
        if enabled {
            {
                let mut snapshot = fs::read(&snapshot_path)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<WindowsProxySnapshot>(&bytes).ok())
                    .unwrap_or_else(|| WindowsProxySnapshot {
                        enabled: registry_value("ProxyEnable"),
                        server: registry_value("ProxyServer"),
                        bypass: registry_value("ProxyOverride"),
                        assistant_server: None,
                        mode: None,
                    });
                snapshot.assistant_server = Some(format!("127.0.0.1:{port}"));
                snapshot.mode = Some("auto".into());
                fs::write(
                    &snapshot_path,
                    serde_json::to_vec_pretty(&snapshot).map_err(|error| error.to_string())?,
                )
                .map_err(|error| format!("保存原代理设置失败：{error}"))?;
            }
            set_registry_value("ProxyEnable", "REG_DWORD", "1")?;
            set_registry_value("ProxyServer", "REG_SZ", &format!("127.0.0.1:{port}"))?;
            set_registry_value("ProxyOverride", "REG_SZ", "<local>;127.*;localhost")?;
            refresh_windows_proxy();
        } else if snapshot_path.is_file() {
            let snapshot: WindowsProxySnapshot = serde_json::from_slice(
                &fs::read(&snapshot_path).map_err(|error| error.to_string())?,
            )
            .map_err(|error| format!("读取原代理设置失败：{error}"))?;
            match snapshot.enabled {
                Some(value) => set_registry_value("ProxyEnable", "REG_DWORD", &value)?,
                None => delete_registry_value("ProxyEnable")?,
            }
            match snapshot.server {
                Some(value) => set_registry_value("ProxyServer", "REG_SZ", &value)?,
                None => delete_registry_value("ProxyServer")?,
            }
            match snapshot.bypass {
                Some(value) => set_registry_value("ProxyOverride", "REG_SZ", &value)?,
                None => delete_registry_value("ProxyOverride")?,
            }
            fs::remove_file(&snapshot_path).map_err(|error| error.to_string())?;
            refresh_windows_proxy();
        }
        update_proxy_tray(&app, enabled);
        return Ok(serde_json::json!({ "enabled": enabled, "port": port }));
    }
    #[cfg(target_os = "macos")]
    {
        let state =
            macos_proxy::change(&proxy_snapshot_path(&app)?, enabled.then_some(port), false)?;
        update_proxy_tray(&app, state["enabled"].as_bool().unwrap_or(false));
        return Ok(state);
    }
    #[allow(unreachable_code)]
    Err("当前系统暂不支持自动设置系统代理".into())
}

#[tauri::command]
async fn clear_system_proxy(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || clear_system_proxy_impl(app))
        .await
        .map_err(|e| format!("清理系统代理失败：{e}"))?
}

fn clear_system_proxy_impl(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    #[cfg(target_os = "windows")]
    {
        let snapshot_path = proxy_snapshot_path(&app)?;
        {
            let mut snapshot = fs::read(&snapshot_path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<WindowsProxySnapshot>(&bytes).ok())
                .unwrap_or_else(|| WindowsProxySnapshot {
                    enabled: registry_value("ProxyEnable"),
                    server: registry_value("ProxyServer"),
                    bypass: registry_value("ProxyOverride"),
                    assistant_server: None,
                    mode: None,
                });
            snapshot.assistant_server = Some("127.0.0.1:0".into());
            snapshot.mode = Some("clear".into());
            fs::write(
                &snapshot_path,
                serde_json::to_vec_pretty(&snapshot).map_err(|error| error.to_string())?,
            )
            .map_err(|error| format!("保存原代理设置失败：{error}"))?;
        }
        set_registry_value("ProxyEnable", "REG_DWORD", "0")?;
        refresh_windows_proxy();
        update_proxy_tray(&app, false);
        return Ok(serde_json::json!({ "enabled": false, "mode": "clear" }));
    }
    #[cfg(target_os = "macos")]
    {
        let state = macos_proxy::change(&proxy_snapshot_path(&app)?, None, true)?;
        update_proxy_tray(&app, false);
        return Ok(state);
    }
    #[allow(unreachable_code)]
    Err("当前系统暂不支持清理系统代理".into())
}

#[cfg(target_os = "windows")]
fn restore_system_proxy(app: &tauri::AppHandle) -> Result<(), String> {
    let snapshot_path = proxy_snapshot_path(app)?;
    if snapshot_path.is_file() {
        let snapshot: WindowsProxySnapshot = serde_json::from_slice(
            &fs::read(&snapshot_path)
                .map_err(|error| format!("读取退出时代理快照失败：{error}"))?,
        )
        .map_err(|error| format!("退出时代理快照无效：{error}"))?;
        let current_enabled = registry_value("ProxyEnable");
        let current_server = registry_value("ProxyServer").unwrap_or_default();
        let assistant_proxy_active = match snapshot.mode.as_deref() {
            Some("clear") => {
                current_enabled.as_deref() == Some("0x0") || current_enabled.as_deref() == Some("0")
            }
            _ => {
                snapshot.assistant_server.as_deref() == Some(current_server.as_str())
                    && (current_enabled.as_deref() == Some("0x1")
                        || current_enabled.as_deref() == Some("1"))
            }
        };
        if assistant_proxy_active {
            set_system_proxy_impl(app.clone(), false, 1)?;
        } else {
            fs::remove_file(&snapshot_path)
                .map_err(|error| format!("清理退出时代理快照失败：{error}"))?;
        }
    }
    Ok(())
}

struct V2raynService {
    current: Mutex<Option<ManagedV2rayn>>,
    startup: AsyncMutex<()>,
    session: AsyncMutex<Option<(String, Instant)>>,
}

struct ManagedV2rayn {
    child: Option<Child>,
    base_url: String,
    management_key: String,
}

const V2RAYN_WEB_PORT: u16 = 47821;

fn v2rayn_data_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_local_data_dir()
        .map(|path| path.join("v2rayN-assistant"))
        .map_err(|error| error.to_string())
}

fn copy_directory(source: &std::path::Path, destination: &std::path::Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let target = destination.join(entry.file_name());
        if entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            copy_directory(&entry.path(), &target)?;
        } else if !target.exists() {
            fs::copy(entry.path(), target).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn packaged_v2rayn_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let platform = if cfg!(target_os = "macos") {
        "macos"
    } else {
        "windows"
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    };
    let package_arch = match (platform, arch) {
        ("windows", "x64") => "win-x64",
        ("windows", "arm64") => "win-arm64",
        ("macos", "x64") => "osx-x64",
        ("macos", "arm64") => "osx-arm64",
        _ => return Err("当前设备架构暂不支持 v2rayN".into()),
    };
    let resource = app
        .path()
        .resolve(
            format!("bundled/v2rayn/{package_arch}"),
            tauri::path::BaseDirectory::Resource,
        )
        .ok();
    if let Some(path) = resource.filter(|path| path.is_dir()) {
        return Ok(path);
    }
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let portable_resources = executable
        .parent()
        .ok_or("无法定位绿色版目录")?
        .join("resources")
        .join("bundled")
        .join("v2rayn")
        .join(package_arch);
    if portable_resources.is_dir() {
        return Ok(portable_resources);
    }
    let development_resources = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("bundled")
        .join("v2rayn")
        .join(package_arch);
    if development_resources.is_dir() {
        return Ok(development_resources);
    }
    Err(format!(
        "绿色包中未包含 v2rayN 管理服务和核心文件（{package_arch}）"
    ))
}

async fn ensure_v2rayn_service(
    app: &tauri::AppHandle,
    service: &V2raynService,
) -> Result<(String, String), String> {
    let _startup_guard = service.startup.lock().await;
    let existing = {
        let mut managed = service
            .current
            .lock()
            .map_err(|_| "v2rayN 服务状态不可用")?;
        if let Some(current) = managed.as_mut() {
            let child_running = match current.child.as_mut() {
                Some(child) => child
                    .try_wait()
                    .map_err(|error| error.to_string())?
                    .is_none(),
                None => true,
            };
            if child_running {
                Some((current.base_url.clone(), current.management_key.clone()))
            } else {
                *managed = None;
                None
            }
        } else {
            None
        }
    };
    if let Some((base_url, management_key)) = existing {
        if v2rayn_service_healthy(&base_url).await {
            return Ok((base_url, management_key));
        }
        let failed = service
            .current
            .lock()
            .map_err(|_| "v2rayN 服务状态不可用")?
            .take();
        if let Some(mut failed) = failed {
            shutdown_v2rayn(&mut failed).await;
        }
    }

    let packaged = packaged_v2rayn_dir(app)?;
    let data_dir = v2rayn_data_dir(app)?;
    fs::create_dir_all(&data_dir).map_err(|error| error.to_string())?;
    let key_path = data_dir.join("management.key");
    let management_key = if key_path.is_file() {
        fs::read_to_string(&key_path)
            .map_err(|error| error.to_string())?
            .trim()
            .to_string()
    } else {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|error| error.to_string())?;
        let key = bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        fs::write(&key_path, &key).map_err(|error| error.to_string())?;
        key
    };
    if management_key.len() < 32 {
        return Err("本机代理服务密钥无效".into());
    }

    let base_url = format!("http://127.0.0.1:{V2RAYN_WEB_PORT}");
    if v2rayn_service_ready(&base_url, &management_key).await {
        let mut managed = service
            .current
            .lock()
            .map_err(|_| "v2rayN 服务状态不可用")?;
        *managed = Some(ManagedV2rayn {
            child: None,
            base_url: base_url.clone(),
            management_key: management_key.clone(),
        });
        return Ok((base_url, management_key));
    }

    let bin_source = packaged.join("bin");
    if !bin_source.is_dir() {
        return Err("绿色包缺少 v2rayN Core 数据目录".into());
    }

    let executable_name = if cfg!(target_os = "windows") {
        "v2rayN.WebAPI.exe"
    } else {
        "v2rayN.WebAPI"
    };
    let packaged_api = packaged.join("api");
    let packaged_executable = packaged_api.join(executable_name);
    if !packaged_executable.is_file() {
        return Err("绿色包缺少 v2rayN 本机管理服务".into());
    }
    let runtime_dir = data_dir.join("runtime");
    fs::create_dir_all(&runtime_dir).map_err(|error| error.to_string())?;
    let executable = runtime_dir.join(executable_name);
    let refresh_executable = match (
        fs::metadata(&packaged_executable),
        fs::metadata(&executable),
    ) {
        (Ok(source), Ok(current)) => {
            source.len() != current.len() || source.modified().ok() != current.modified().ok()
        }
        _ => true,
    };
    if refresh_executable {
        fs::copy(&packaged_executable, &executable)
            .map_err(|error| format!("无法准备助手专用的 v2rayN 管理服务：{error}"))?;
    }
    let launcher_source = packaged_api.join("Launcher");
    if launcher_source.is_dir() {
        copy_directory(&launcher_source, &runtime_dir.join("Launcher"))?;
    }
    let static_assets = packaged_api.join("v2rayN.WebAPI.staticwebassets.endpoints.json");
    if static_assets.is_file() {
        let destination = runtime_dir.join("v2rayN.WebAPI.staticwebassets.endpoints.json");
        if !destination.is_file()
            || fs::metadata(&static_assets)
                .ok()
                .and_then(|m| m.modified().ok())
                != fs::metadata(&destination)
                    .ok()
                    .and_then(|m| m.modified().ok())
        {
            fs::copy(static_assets, destination).map_err(|error| error.to_string())?;
        }
    }
    let api_bin = executable
        .parent()
        .ok_or("无法定位 v2rayN 管理服务目录")?
        .join("bin");
    if !api_bin
        .join("xray")
        .join(if cfg!(target_os = "windows") {
            "xray.exe"
        } else {
            "xray"
        })
        .is_file()
    {
        copy_directory(&bin_source, &api_bin)?;
    }
    if !api_bin
        .join("xray")
        .join(if cfg!(target_os = "windows") {
            "xray.exe"
        } else {
            "xray"
        })
        .is_file()
    {
        return Err("v2rayN Core 文件不完整，缺少 Xray 核心".into());
    }
    let mut command = Command::new(executable);
    command
        .args(["--foreground", "--no-open"])
        .current_dir(&runtime_dir)
        .env("ASPNETCORE_URLS", &base_url)
        .env("V2RAYN_WEB_API_KEY", &management_key)
        .env("V2RAYN_LOCAL_APPLICATION_DATA_V2", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    command.creation_flags(0x08000000);
    let mut child = command
        .spawn()
        .map_err(|error| format!("无法启动内置 v2rayN 管理服务：{error}"))?;
    let mut ready = false;
    for _ in 0..40 {
        if v2rayn_service_ready(&base_url, &management_key).await {
            ready = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    if !ready {
        let _ = child.kill();
        let _ = child.wait();
        return Err("v2rayN 本机管理服务未能完成认证和状态检查".into());
    }
    let owns_child = child
        .try_wait()
        .map_err(|error| error.to_string())?
        .is_none();
    let mut managed = service
        .current
        .lock()
        .map_err(|_| "v2rayN 服务状态不可用")?;
    *managed = Some(ManagedV2rayn {
        child: owns_child.then_some(child),
        base_url: base_url.clone(),
        management_key: management_key.clone(),
    });
    Ok((base_url, management_key))
}

async fn v2rayn_service_ready(base_url: &str, management_key: &str) -> bool {
    if !v2rayn_service_healthy(base_url).await {
        return false;
    }
    let Ok(client) = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(3))
        .no_proxy()
        .build()
    else {
        return false;
    };
    let Ok(login) = client
        .post(format!("{base_url}/api/auth/login"))
        .json(&serde_json::json!({ "key": management_key }))
        .send()
        .await
    else {
        return false;
    };
    let Ok(json) = login.json::<serde_json::Value>().await else {
        return false;
    };
    let Some(token) = json
        .pointer("/data/token")
        .and_then(serde_json::Value::as_str)
    else {
        return false;
    };
    client
        .get(format!("{base_url}/api/status"))
        .bearer_auth(token)
        .send()
        .await
        .is_ok_and(|response| response.status().is_success())
}

async fn v2rayn_service_healthy(base_url: &str) -> bool {
    let Ok(client) = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(3))
        .no_proxy()
        .build()
    else {
        return false;
    };
    client
        .get(format!("{base_url}/api/health"))
        .send()
        .await
        .is_ok_and(|response| response.status().is_success())
}

async fn v2rayn_session(
    service: &V2raynService,
    base_url: &str,
    management_key: &str,
) -> Result<String, String> {
    let mut session = service.session.lock().await;
    if let Some((token, expires_at)) = session.as_ref() {
        if *expires_at > Instant::now() + Duration::from_secs(15) {
            return Ok(token.clone());
        }
    }
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(10))
        .no_proxy()
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .post(format!("{base_url}/api/auth/login"))
        .json(&serde_json::json!({ "key": management_key }))
        .send()
        .await
        .map_err(|error| format!("无法连接 v2rayN 本机服务：{error}"))?;
    if !response.status().is_success() {
        return Err("v2rayN 本机服务认证失败".into());
    }
    let json = response
        .json::<serde_json::Value>()
        .await
        .map_err(|error| error.to_string())?;
    let token = json
        .pointer("/data/token")
        .and_then(serde_json::Value::as_str)
        .ok_or("v2rayN 本机服务未返回会话")?
        .to_string();
    *session = Some((token.clone(), Instant::now() + Duration::from_secs(25 * 60)));
    Ok(token)
}

#[tauri::command]
async fn v2rayn_api(
    app: tauri::AppHandle,
    service: tauri::State<'_, V2raynService>,
    method: String,
    path: String,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    if !path.starts_with("/api/")
        || path.contains("..")
        || !["GET", "POST", "PUT", "DELETE"].contains(&method.as_str())
    {
        return Err("v2rayN 请求无效".into());
    }
    let (base_url, management_key) = ensure_v2rayn_service(&app, &service).await?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(30))
        .no_proxy()
        .build()
        .map_err(|error| error.to_string())?;
    let token = v2rayn_session(&service, &base_url, &management_key).await?;
    let method =
        reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| "v2rayN 请求方法无效")?;
    let mut request = client
        .request(method, format!("{base_url}{path}"))
        .bearer_auth(&token);
    if let Some(payload) = body {
        request = request.json(&payload);
    }
    let response = request.send().await.map_err(|error| error.to_string())?;
    let status = response.status();
    let response_text = response.text().await.unwrap_or_default();
    let payload = serde_json::from_str::<serde_json::Value>(&response_text)
        .unwrap_or(serde_json::Value::Null);
    let operation_failed = payload
        .get("success")
        .or_else(|| payload.get("Success"))
        .and_then(serde_json::Value::as_bool)
        == Some(false);
    if !status.is_success() || operation_failed {
        let code = payload
            .get("code")
            .or_else(|| payload.get("Code"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("operation_failed");
        let message_key = payload
            .get("messageKey")
            .or_else(|| payload.get("MessageKey"))
            .and_then(serde_json::Value::as_str);
        let detail = payload
            .get("data")
            .or_else(|| payload.get("Data"))
            .and_then(|data| {
                data.get("detail")
                    .or_else(|| data.get("Detail"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .or_else(|| data.as_str().map(str::to_owned))
                    .or_else(|| {
                        data.get("port")
                            .or_else(|| data.get("Port"))
                            .and_then(serde_json::Value::as_u64)
                            .map(|port| format!("端口 {port}"))
                    })
            });
        let suffix = if let Some(detail) = detail {
            format!("：{detail}")
        } else if code == "proxy_port_in_use" {
            let port = payload
                .pointer("/data/configuredProxyPort")
                .and_then(serde_json::Value::as_u64)
                .map(|port| format!("（端口 {port}）"))
                .unwrap_or_default();
            format!("：本地代理端口被占用{port}")
        } else if let Some(message_key) = message_key {
            format!("：{message_key}")
        } else if !response_text.trim().is_empty() {
            format!(
                "：HTTP {} {}",
                status.as_u16(),
                response_text.chars().take(240).collect::<String>()
            )
        } else {
            format!("：HTTP {}", status.as_u16())
        };
        return Err(format!("v2rayN 操作失败 [{code}]{suffix}"));
    }
    Ok(payload)
}

#[tauri::command]
async fn select_proxy_profile(
    app: tauri::AppHandle,
    service: tauri::State<'_, V2raynService>,
    profile_id: String,
) -> Result<(), String> {
    let (base_url, management_key) = ensure_v2rayn_service(&app, &service).await?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(30))
        .no_proxy()
        .build()
        .map_err(|error| error.to_string())?;
    let token = v2rayn_session(&service, &base_url, &management_key).await?;
    let settings_response = client
        .get(format!("{base_url}/api/settings"))
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|error| format!("读取代理监听配置失败：{error}"))?;
    let status = settings_response.status();
    let settings: serde_json::Value = settings_response
        .json()
        .await
        .map_err(|error| format!("代理服务设置响应无效：{error}"))?;
    if !status.is_success() {
        return Err(format!("读取代理监听配置失败（HTTP {}）", status.as_u16()));
    }
    let envelope = settings
        .get("data")
        .or_else(|| settings.get("Data"))
        .unwrap_or(&settings);
    let inbound = envelope
        .get("inbound")
        .or_else(|| envelope.get("Inbound"))
        .ok_or("代理服务未返回 inbound 配置")?;
    let current_port = inbound
        .get("localPort")
        .or_else(|| inbound.get("LocalPort"))
        .and_then(serde_json::Value::as_u64)
        .and_then(|port| u16::try_from(port).ok())
        .ok_or("代理服务返回的监听端口无效")?;
    if !loopback_port_available(current_port) {
        let replacement = (20000u16..=45000u16)
            .find(|port| loopback_port_available(*port))
            .ok_or("未找到可用的本地代理端口")?;
        let mut updated_inbound = inbound.clone();
        let inbound_object = updated_inbound
            .as_object_mut()
            .ok_or("代理监听配置格式无效")?;
        inbound_object.insert("localPort".into(), serde_json::json!(replacement));
        let response = client
            .put(format!("{base_url}/api/settings/inbound"))
            .bearer_auth(&token)
            .json(&updated_inbound)
            .send()
            .await
            .map_err(|error| format!("修改本机代理端口失败：{error}"))?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        let payload = serde_json::from_str::<serde_json::Value>(&body).unwrap_or_default();
        if !status.is_success()
            || payload.get("success").or_else(|| payload.get("Success"))
                == Some(&serde_json::Value::Bool(false))
        {
            let code = payload
                .get("code")
                .or_else(|| payload.get("Code"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("port_update_failed");
            return Err(format!(
                "自动更换代理端口失败 [{code}] HTTP {} {}",
                status.as_u16(),
                body.chars().take(180).collect::<String>()
            ));
        }
    }

    let response = client
        .post(format!(
            "{base_url}/api/profiles/{}/select",
            urlencoding_component(&profile_id)
        ))
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|error| format!("连接 v2rayN 节点切换接口失败：{error}"))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let payload = serde_json::from_str::<serde_json::Value>(&body).unwrap_or_default();
    let failed = payload.get("success").or_else(|| payload.get("Success"))
        == Some(&serde_json::Value::Bool(false));
    if !status.is_success() || failed {
        let code = payload
            .get("code")
            .or_else(|| payload.get("Code"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("operation_failed");
        let detail = payload
            .pointer("/data/detail")
            .or_else(|| payload.pointer("/Data/Detail"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        return Err(format!(
            "v2rayN 操作失败 [{code}] HTTP {} {} {}",
            status.as_u16(),
            detail,
            body.chars().take(180).collect::<String>()
        ));
    }
    Ok(())
}

fn urlencoding_component(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn loopback_port_available(port: u16) -> bool {
    std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).is_ok()
}

#[tauri::command]
async fn v2rayn_stop(service: tauri::State<'_, V2raynService>) -> Result<(), String> {
    let managed = {
        let mut guard = service
            .current
            .lock()
            .map_err(|_| "v2rayN 服务状态不可用")?;
        guard.take()
    };
    if let Some(mut current) = managed {
        shutdown_v2rayn(&mut current).await;
    }
    Ok(())
}

async fn shutdown_v2rayn(current: &mut ManagedV2rayn) {
    let Some(child) = current.child.as_mut() else {
        return;
    };
    let client = match reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(3))
        .no_proxy()
        .build()
    {
        Ok(client) => client,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return;
        }
    };
    if let Ok(login) = client
        .post(format!("{}/api/auth/login", current.base_url))
        .json(&serde_json::json!({ "key": current.management_key }))
        .send()
        .await
    {
        if let Ok(json) = login.json::<serde_json::Value>().await {
            if let Some(token) = json
                .pointer("/data/token")
                .and_then(serde_json::Value::as_str)
            {
                let _ = client
                    .post(format!("{}/api/core/stop", current.base_url))
                    .bearer_auth(token)
                    .send()
                    .await;
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[derive(serde::Serialize, Default)]
struct InstalledStatus {
    installed: bool,
    path: String,
    version: String,
}

#[derive(serde::Serialize, Default)]
struct InstallationDetectionResult {
    codex: InstalledStatus,
    codexpp: InstalledStatus,
}

#[cfg(target_os = "windows")]
fn detect_codex_windows() -> InstalledStatus {
    codex_detection_windows::discover()
        .map(|app| InstalledStatus {
            installed: true,
            path: app.executable.to_string_lossy().into_owned(),
            version: app.version,
        })
        .unwrap_or_default()
}

fn codex_model_catalog_engine() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let local_app_data = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?;
        let engine_dir = local_app_data.join("OpenAI").join("Codex").join("bin");
        if let Ok(entries) = fs::read_dir(&engine_dir) {
            let mut candidates = entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .collect::<Vec<_>>();
            candidates.sort_by_key(|path| {
                std::cmp::Reverse(
                    fs::metadata(path)
                        .and_then(|metadata| metadata.modified())
                        .ok(),
                )
            });
            if let Some(engine) = candidates
                .into_iter()
                .map(|path| path.join("codex.exe"))
                .find(|path| path.is_file())
            {
                return Some(engine);
            }
        }

        if let Some(app) = codex_detection_windows::discover() {
            for candidate in [
                app.root.join("app/resources/codex.exe"),
                app.root.join("resources/codex.exe"),
                app.root.join("app/codex.exe"),
                app.root.join("codex.exe"),
            ] {
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
        None
    }

    #[cfg(target_os = "macos")]
    {
        let desktop = macos_apps::discover(false);
        let home = std::env::var_os("HOME").map(PathBuf::from);
        macos_apps::session_cli_candidates(
            desktop.as_ref().map(|app| app.executable.as_path()),
            home.as_deref(),
            std::env::var_os("PATH").as_deref(),
        )
        .into_iter()
        .find(|path| macos_apps::executable_file(path))
    }
}

fn prepare_codex_model_catalog(
    home: &std::path::Path,
    model_ids: &[String],
) -> Result<(PathBuf, Vec<u8>), String> {
    let destination = home.join("model-catalog.json");

    let native_catalog = codex_model_catalog_engine()
        .and_then(|engine| {
            let mut command = Command::new(engine);
            command.args(["debug", "models", "--bundled"]);
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(0x08000000);
            }
            let output = command.output().ok()?;
            output.status.success().then_some(output.stdout)
        })
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|catalog| model_catalog::merge_upstream_models(catalog, model_ids).ok());
    let catalog = match native_catalog {
        Some(catalog) => catalog,
        None => match model_catalog::from_cache(home, model_ids) {
            Ok(catalog) => catalog,
            Err(_) => model_catalog::fallback_catalog(model_ids)?,
        },
    };
    let bytes = serde_json::to_vec(&catalog)
        .map_err(|error| format!("生成 Codex 模型目录失败：{error}"))?;
    Ok((destination, bytes))
}

#[cfg(target_os = "windows")]
fn detect_codexpp_windows() -> InstalledStatus {
    use std::os::windows::process::CommandExt;

    // 1. Registry path is useful only when it contains the actual application executable.
    if let Ok(output) = Command::new("reg.exe")
        .args(["query", r"HKCU\SOFTWARE\Codex++", "/v", "InstallDir"])
        .creation_flags(0x08000000)
        .output()
    {
        if output.status.success() {
            let out_str = String::from_utf8_lossy(&output.stdout);
            for line in out_str.lines() {
                if line.contains("REG_SZ") || line.contains("REG_EXPAND_SZ") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 3 {
                        let dir = parts[2..].join(" ");
                        let p = PathBuf::from(&dir);
                        if p.join("codex-plus-plus.exe").is_file()
                            || p.join("codex-plus-plus-manager.exe").is_file()
                            || p.join("CodexPlusPlus.exe").is_file()
                        {
                            return InstalledStatus {
                                installed: true,
                                path: p.to_string_lossy().to_string(),
                                version: String::new(),
                            };
                        }
                    }
                }
            }
        }
    }

    // 2. Known executable locations
    let local_app_data = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let program_files = std::env::var("ProgramFiles").unwrap_or_default();
    let program_files_x86 = std::env::var("ProgramFiles(x86)").unwrap_or_default();

    let exe_candidates = [
        format!("{local_app_data}\\Programs\\CodexPlusPlus\\CodexPlusPlus.exe"),
        format!("{local_app_data}\\Programs\\codex-plus-plus\\CodexPlusPlus.exe"),
        format!("{local_app_data}\\CodexPlusPlus\\CodexPlusPlus.exe"),
        format!("{program_files}\\CodexPlusPlus\\CodexPlusPlus.exe"),
        format!("{program_files}\\Codex++\\codex-plus-plus.exe"),
        format!("{program_files_x86}\\CodexPlusPlus\\CodexPlusPlus.exe"),
    ];

    for candidate in &exe_candidates {
        let p = PathBuf::from(candidate);
        if p.is_file() {
            return InstalledStatus {
                installed: true,
                path: p.to_string_lossy().to_string(),
                version: String::new(),
            };
        }
    }

    // 3. Programs dir search; require a matching executable, not just a directory name.
    let programs_dir = PathBuf::from(&local_app_data).join("Programs");
    if programs_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(programs_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if name.contains("codex") && (name.contains("plus") || name.contains("++")) {
                    for exe in [
                        "codex-plus-plus.exe",
                        "codex-plus-plus-manager.exe",
                        "CodexPlusPlus.exe",
                    ] {
                        let path = entry.path().join(exe);
                        if path.is_file() {
                            return InstalledStatus {
                                installed: true,
                                path: path.to_string_lossy().to_string(),
                                version: String::new(),
                            };
                        }
                    }
                }
            }
        }
    }

    InstalledStatus::default()
}

#[cfg(target_os = "macos")]
fn detect_codex_macos() -> InstalledStatus {
    macos_apps::discover(false)
        .map(|app| InstalledStatus {
            installed: true,
            path: app.executable.to_string_lossy().into_owned(),
            version: app.version,
        })
        .unwrap_or_default()
}

#[cfg(target_os = "macos")]
fn detect_codexpp_macos() -> InstalledStatus {
    macos_apps::discover(true)
        .map(|app| InstalledStatus {
            installed: true,
            path: app.executable.to_string_lossy().into_owned(),
            version: app.version,
        })
        .unwrap_or_default()
}

#[tauri::command]
async fn check_installations() -> Result<InstallationDetectionResult, String> {
    tauri::async_runtime::spawn_blocking(check_installations_impl)
        .await
        .map_err(|e| format!("检测安装状态失败：{e}"))
}

fn check_installations_impl() -> InstallationDetectionResult {
    #[cfg(target_os = "windows")]
    {
        InstallationDetectionResult {
            codex: detect_codex_windows(),
            codexpp: detect_codexpp_windows(),
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        InstallationDetectionResult {
            codex: detect_codex_macos(),
            codexpp: detect_codexpp_macos(),
        }
    }
}

#[tauri::command]
fn product_process_running(product: String) -> Result<bool, String> {
    if !["codex", "codexpp"].contains(&product.as_str()) {
        return Err("不支持的产品".into());
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let names = if product == "codex" {
            "'Codex','ChatGPT','codex'"
        } else {
            "'CodexPlusPlus','codex-plus-plus','codex-plus-plus-manager'"
        };
        let script = format!(
            "if (Get-Process -Name {names} -ErrorAction SilentlyContinue) {{ exit 0 }} else {{ exit 1 }}"
        );
        let output = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .creation_flags(0x08000000)
            .output()
            .map_err(|error| format!("检查 {product} 运行状态失败：{error}"))?;
        Ok(output.status.success())
    }
    #[cfg(target_os = "macos")]
    {
        let Some(app) = macos_apps::discover(product == "codexpp") else {
            return Ok(false);
        };
        Ok(!macos_apps::process_ids(&app.executable)?.is_empty())
    }
}

#[tauri::command]
fn stop_codex_processes() -> Result<(), String> {
    if !product_process_running("codex".into())? {
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        for image in ["ChatGPT.exe", "Codex.exe", "codex.exe"] {
            let output = Command::new("taskkill.exe")
                .args(["/F", "/T", "/IM", image])
                .creation_flags(0x08000000)
                .output();
            if let Err(error) = output {
                return Err(format!("无法结束 {image}：{error}"));
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(app) = macos_apps::discover(false) {
            macos_apps::stop(&app.executable)?;
        }
    }

    for _ in 0..20 {
        if !product_process_running("codex".into())? {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err("无法完全关闭 Codex/ChatGPT 后台进程，请检查权限后重试。".into())
}

#[tauri::command]
async fn reset_codex_initial_state(
    wechat_state: State<'_, wechat::WeChatState>,
) -> Result<codex_reset::ResetReport, String> {
    // Stop only the assistant-owned worker before closing Codex; no conversation data is touched.
    wechat::shutdown(&wechat_state);
    tauri::async_runtime::spawn_blocking(|| {
        let _guard = PROVIDER_CONFIG_LOCK
            .lock()
            .map_err(|_| "供应商配置锁不可用")?;
        if product_process_running("codexpp".into())? {
            return Err("请先从托盘完全退出 Codex++，避免它重新写入供应商配置，然后重试。".into());
        }
        let home = codex_home_dir()?;
        let settings = codexpp_settings_path()?;
        // Validate everything before closing the user's application or changing files.
        codex_reset::plan(&home, &settings)?;
        stop_codex_processes()?;
        let report = codex_reset::apply(codex_reset::plan(&home, &settings)?)?;
        gateway::clear_configuration();
        CODEX_CONTROL_PORT.store(0, Ordering::SeqCst);
        Ok(report)
    })
    .await
    .map_err(|e| format!("恢复 Codex 失败：{e}"))?
}

#[cfg(target_os = "windows")]
fn launch_codex_msix_app() -> Result<bool, String> {
    use std::os::windows::process::CommandExt;
    let script = "$p = Get-AppxPackage -Name 'OpenAI.Codex*' | Sort-Object Version -Descending | Select-Object -First 1; if ($p) { $xml = [xml](Get-Content -LiteralPath (Join-Path $p.InstallLocation 'AppxManifest.xml')); $app = $xml.Package.Applications.Application | Select-Object -First 1; if ($app) { \"$($p.PackageFamilyName)|$($app.Id)\" } }";
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(0x08000000)
        .output()
        .map_err(|error| format!("读取 Codex 应用入口失败：{error}"))?;
    if !output.status.success() {
        return Ok(false);
    }
    let identity = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let Some((family, app_id)) = identity.split_once('|') else {
        return Ok(false);
    };
    if family.is_empty() || app_id.is_empty() {
        return Ok(false);
    }
    Command::new("explorer.exe")
        .arg(format!("shell:AppsFolder\\{family}!{app_id}"))
        .spawn()
        .map_err(|error| format!("无法通过 Windows 应用入口启动 Codex：{error}"))?;
    Ok(true)
}

#[cfg(target_os = "windows")]
fn wait_for_codex_process() -> Result<bool, String> {
    for _ in 0..20 {
        if product_process_running("codex".into())? {
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Ok(false)
}

#[tauri::command]
async fn create_codex_desktop_shortcut() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    return tauri::async_runtime::spawn_blocking(|| {
        // Use the registered app identity so Store/MSIX updates do not break
        // the shortcut by changing the WindowsApps version directory.
        let script = r#"
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
$package = Get-AppxPackage -Name 'OpenAI.Codex*' | Sort-Object Version -Descending | Select-Object -First 1
if (-not $package) { throw 'Codex desktop app is not registered for the current user.' }
$manifest = [xml](Get-Content -LiteralPath (Join-Path $package.InstallLocation 'AppxManifest.xml') -Encoding UTF8)
$application = $manifest.Package.Applications.Application | Select-Object -First 1
if (-not $application.Id) { throw 'Codex desktop app entry was not found.' }
$desktop = [Environment]::GetFolderPath('DesktopDirectory')
if (-not $desktop) { throw 'Windows desktop directory is unavailable.' }
[IO.Directory]::CreateDirectory($desktop) | Out-Null
$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut((Join-Path $desktop 'ChatGPT.lnk'))
$shortcut.TargetPath = Join-Path $env:WINDIR 'explorer.exe'
$shortcut.Arguments = "shell:AppsFolder\$($package.PackageFamilyName)!$($application.Id)"
$shortcut.Description = 'ChatGPT'
$icon = Join-Path $package.InstallLocation 'app\ChatGPT.exe'
if (Test-Path -LiteralPath $icon -PathType Leaf) { $shortcut.IconLocation = "$icon,0" }
$shortcut.Save()
if (-not (Test-Path -LiteralPath (Join-Path $desktop 'ChatGPT.lnk') -PathType Leaf)) { throw 'Desktop shortcut was not created.' }
"#;
        let output = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .creation_flags(0x08000000)
            .output().map_err(|error| format!("创建 ChatGPT 桌面快捷方式失败：{error}"))?;
        if !output.status.success() {
            return Err(format!("创建 ChatGPT 桌面快捷方式失败：{}", String::from_utf8_lossy(&output.stderr).trim()));
        }
        Ok(())
    }).await.map_err(|error| format!("创建桌面快捷方式任务失败：{error}"))?;
    #[cfg(not(target_os = "windows"))]
    Ok(())
}

#[tauri::command]
async fn launch_installed_product(product: String, restart: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || launch_installed_product_impl(product, restart))
        .await
        .map_err(|e| format!("启动任务失败：{e}"))?
}

fn launch_installed_product_impl(product: String, restart: bool) -> Result<(), String> {
    launch_installed_product_with_chinese(product, restart, true)
}

fn launch_installed_product_with_chinese(
    product: String,
    restart: bool,
    chinese_in_background: bool,
) -> Result<(), String> {
    if !["codex", "codexpp"].contains(&product.as_str()) {
        return Err("不支持启动此产品".into());
    }
    let detected = if product == "codex" {
        #[cfg(target_os = "windows")]
        {
            detect_codex_windows()
        }
        #[cfg(target_os = "macos")]
        {
            detect_codex_macos()
        }
    } else {
        #[cfg(target_os = "windows")]
        {
            detect_codexpp_windows()
        }
        #[cfg(target_os = "macos")]
        {
            detect_codexpp_macos()
        }
    };
    if !detected.installed {
        return Err(format!(
            "未检测到已安装的 {}",
            if product == "codex" {
                "Codex"
            } else {
                "Codex++"
            }
        ));
    }

    if restart && product_process_running(product.clone())? {
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            let images: &[&str] = if product == "codex" {
                &["Codex.exe", "ChatGPT.exe", "codex.exe"]
            } else {
                &[
                    "CodexPlusPlus.exe",
                    "codex-plus-plus.exe",
                    "codex-plus-plus-manager.exe",
                ]
            };
            for image in images {
                let _ = Command::new("taskkill.exe")
                    .args(["/F", "/IM", image])
                    .creation_flags(0x08000000)
                    .output();
            }
        }
        #[cfg(target_os = "macos")]
        {
            macos_apps::stop(std::path::Path::new(&detected.path))?;
        }
        std::thread::sleep(Duration::from_millis(700));
    }

    let force_chinese = if product == "codex" {
        let home = codex_home_dir()?;
        if codex_chinese::configured(&home) {
            codex_chinese::enabled(&home)?
        } else {
            // Preserve the option selected in older assistant releases.
            fs::read(codexpp_settings_path()?)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                .is_some_and(|root| root["codexAppForceChineseLocale"] == true)
        }
    } else {
        false
    };
    let verify_access = product == "codex"
        && codex_home_dir()
            .ok()
            .and_then(|home| fs::read(home.join(".usage-assistant-full-access.json")).ok())
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .is_some_and(|value| value["enabled"] == true);
    if product == "codex" {
        CODEX_CONTROL_PORT.store(0, Ordering::SeqCst);
    }
    let chinese_port = if force_chinese || verify_access {
        if product_process_running("codex".into())? {
            return Err("Codex 仍在运行，请先退出再通过助手启动，以应用强制中文界面。".into());
        }
        Some(codex_chinese::reserve_port()?)
    } else {
        None
    };

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        if product == "codex" && chinese_port.is_none() {
            if launch_codex_msix_app()? {
                return Ok(());
            }
        }
        let path = PathBuf::from(detected.path);
        let executable = if path.is_file() {
            path
        } else {
            let names: &[&str] = if product == "codex" {
                &["Codex.exe", "codex.exe", "ChatGPT.exe"]
            } else {
                &[
                    "CodexPlusPlus.exe",
                    "codex-plus-plus.exe",
                    "codex-plus-plus-manager.exe",
                ]
            };
            names
                .iter()
                .map(|name| path.join(name))
                .find(|candidate| candidate.is_file())
                .ok_or("未找到可启动的应用程序文件")?
        };
        let mut command = Command::new(&executable);
        if let Some(directory) = executable.parent() {
            command.current_dir(directory);
        }
        command.creation_flags(0x08000000);
        if let Some(port) = chinese_port {
            match codex_launch_windows::activate(&executable, port) {
                Ok(true) if wait_for_codex_process()? => {
                    CODEX_CONTROL_PORT.store(port, Ordering::SeqCst);
                    if force_chinese && chinese_in_background {
                        codex_chinese::apply_in_background(port);
                    }
                    return Ok(());
                }
                Ok(true) => {
                    if launch_codex_msix_app()? && wait_for_codex_process()? {
                        return Ok(());
                    }
                    return Err("Windows 已接受 Codex 启动请求，但 Codex 进程未启动；请从开始菜单确认 Codex 可正常打开后重试。".into());
                }
                Err(activation_error) => {
                    if launch_codex_msix_app()? && wait_for_codex_process()? {
                        return Ok(());
                    }
                    return Err(format!(
                        "{activation_error}；Windows 应用入口回退启动也未成功，请从开始菜单确认 Codex 可正常打开后重试。"
                    ));
                }
                Ok(false) => {}
            }
            command.args([
                "--remote-debugging-address=127.0.0.1",
                &format!("--remote-debugging-port={port}"),
                "--lang=zh-CN",
            ]);
        }
        if product == "codex" {
            for name in ["ELECTRON_RUN_AS_NODE", "ELECTRON_NO_ASAR", "NODE_OPTIONS"] {
                command.env_remove(name);
            }
        }
        if let Err(error) = command.spawn() {
            return Err(format!("启动应用失败：{error}"));
        }
        if let Some(port) = chinese_port {
            CODEX_CONTROL_PORT.store(port, Ordering::SeqCst);
            if force_chinese && chinese_in_background {
                codex_chinese::apply_in_background(port);
            }
        }
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        let executable = PathBuf::from(detected.path);
        let app_bundle = executable.ancestors().find(|path| {
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
        });
        if let Some(app_bundle) = app_bundle {
            let mut command = Command::new("open");
            command.args(["-a"]).arg(app_bundle);
            if let Some(port) = chinese_port {
                command.args([
                    "--args",
                    "--remote-debugging-address=127.0.0.1",
                    &format!("--remote-debugging-port={port}"),
                    "--lang=zh-CN",
                ]);
            }
            command
                .spawn()
                .map_err(|error| format!("启动应用失败：{error}"))?;
        } else {
            let mut command = Command::new(executable);
            if let Some(port) = chinese_port {
                command.args([
                    "--remote-debugging-address=127.0.0.1",
                    &format!("--remote-debugging-port={port}"),
                    "--lang=zh-CN",
                ]);
            }
            command
                .spawn()
                .map_err(|error| format!("启动应用失败：{error}"))?;
        }
        if let Some(port) = chinese_port {
            CODEX_CONTROL_PORT.store(port, Ordering::SeqCst);
            if force_chinese && chinese_in_background {
                codex_chinese::apply_in_background(port);
            }
        }
        Ok(())
    }
}

#[tauri::command]
fn launch_codex_via_codexpp(restart: bool) -> Result<(), String> {
    let detected = {
        #[cfg(target_os = "windows")]
        {
            detect_codexpp_windows()
        }
        #[cfg(target_os = "macos")]
        {
            detect_codexpp_macos()
        }
    };
    if !detected.installed {
        return Err("未检测到 Codex++，请先安装 Codex++".into());
    }

    if restart && product_process_running("codexpp".into())? {
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            for image in [
                "CodexPlusPlus.exe",
                "codex-plus-plus.exe",
                "codex-plus-plus-manager.exe",
            ] {
                let _ = Command::new("taskkill.exe")
                    .args(["/F", "/IM", image])
                    .creation_flags(0x08000000)
                    .output();
            }
        }
        #[cfg(target_os = "macos")]
        {
            for name in ["Codex++", "CodexPlusPlus", "codex-plus-plus"] {
                let _ = Command::new("pkill").args(["-x", name]).output();
            }
        }
        std::thread::sleep(Duration::from_millis(700));
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let path = PathBuf::from(&detected.path);
        let executable = if path.is_file() {
            path
        } else {
            [
                "CodexPlusPlus.exe",
                "codex-plus-plus.exe",
                "codex-plus-plus-manager.exe",
            ]
            .iter()
            .map(|name| path.join(name))
            .find(|candidate| candidate.is_file())
            .ok_or("未找到 Codex++ 可执行文件")?
        };
        let mut command = Command::new(executable);
        // Keep Codex++'s helper launch in the background. Codex++ is still
        // responsible for applying its own Codex startup/configuration flow.
        command.arg("--start-codex").creation_flags(0x08000000);
        command
            .spawn()
            .map_err(|error| format!("无法通过 Codex++ 启动 Codex：{error}"))?;
        for _ in 0..12 {
            std::thread::sleep(Duration::from_millis(500));
            if product_process_running("codex".into())? {
                return Ok(());
            }
        }
        // Older Codex++ builds may accept --start-codex but fail to launch
        // the Store app. Preserve the intended sequence with a fallback.
        if !product_process_running("codex".into())? {
            launch_installed_product_impl("codex".into(), false)?;
        }
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        let executable = PathBuf::from(&detected.path);
        let app_bundle = executable
            .ancestors()
            .find(|path| {
                path.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
            })
            .ok_or("未找到 Codex++ 应用包")?;
        Command::new("open")
            .args(["-a"])
            .arg(app_bundle)
            .arg("--args")
            .arg("--start-codex")
            .spawn()
            .map_err(|error| format!("无法通过 Codex++ 启动 Codex：{error}"))?;
        for _ in 0..12 {
            std::thread::sleep(Duration::from_millis(500));
            if product_process_running("codex".into())? {
                return Ok(());
            }
        }
        if !product_process_running("codex".into())? {
            launch_installed_product_impl("codex".into(), false)?;
        }
        Ok(())
    }
}

#[tauri::command]
fn uninstall_product(product: String) -> Result<String, String> {
    if !["codex", "codexpp"].contains(&product.as_str()) {
        return Err("不支持删除此产品".into());
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let detected = if product == "codex" {
            detect_codex_windows()
        } else {
            detect_codexpp_windows()
        };
        if !detected.installed {
            return Err("未检测到已安装的程序".into());
        }

        if product == "codex" {
            // Keep the package data so sessions, skills, plugins and user settings
            // are available again when Codex is reinstalled.
            let script = "$p = Get-AppxPackage -Name 'OpenAI.Codex*' | Select-Object -First 1; if ($p) { Remove-AppxPackage -Package $p.PackageFullName -PreserveApplicationData -ErrorAction Stop; 'removed' }";
            let output = Command::new("powershell.exe")
                .args(["-NoProfile", "-NonInteractive", "-Command", script])
                .creation_flags(0x08000000)
                .output()
                .map_err(|error| format!("无法启动 Codex 卸载：{error}"))?;
            if output.status.success()
                && String::from_utf8_lossy(&output.stdout).contains("removed")
            {
                return Ok("removed".into());
            }
        }

        if let Some(product_code) = windows_msi_product_code(&product) {
            let output = Command::new("msiexec.exe")
                .args(["/x", &product_code, "/qn", "/norestart"])
                .creation_flags(0x08000000)
                .output()
                .map_err(|error| format!("无法启动卸载程序：{error}"))?;
            if output.status.success() {
                return Ok("removed".into());
            }
            return Err(format!(
                "卸载失败：{}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }

        let uninstall = windows_registered_uninstaller(&product, &detected.path)?;
        let mut command = Command::new(&uninstall.executable);
        command.args(&uninstall.arguments);
        command.creation_flags(0x08000000);
        let child = command
            .spawn()
            .map_err(|error| format!("无法启动卸载程序：{error}"))?;
        drop(child);
        Ok("launched".into())
    }
    #[cfg(target_os = "macos")]
    {
        let detected = if product == "codex" {
            detect_codex_macos()
        } else {
            detect_codexpp_macos()
        };
        if !detected.installed {
            return Err("未检测到已安装的程序".into());
        }
        let executable = PathBuf::from(&detected.path);
        let app = executable
            .ancestors()
            .find(|path| {
                path.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
            })
            .ok_or("检测到的不是受支持的 .app 安装，未删除文件")?;
        if macos_apps::inspect(app, product == "codexpp").is_none() {
            return Err("应用标识不在允许卸载的范围内".into());
        }
        let output = Command::new("osascript")
            .args([
                "-e",
                "on run argv",
                "tell application \"Finder\" to delete POSIX file (item 1 of argv)",
                "end run",
            ])
            .arg(app)
            .output()
            .map_err(|error| format!("无法启动安全移除：{error}"))?;
        if output.status.success() {
            Ok("removed".into())
        } else {
            Err(format!(
                "移除失败：{}",
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    }
}

#[cfg(target_os = "windows")]
fn windows_msi_product_code(product: &str) -> Option<String> {
    use std::os::windows::process::CommandExt;
    let display_name = if product == "codexpp" {
        "Codex++"
    } else {
        "Codex"
    };
    let script = format!(
        "$roots = @('HKLM:\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall', 'HKLM:\\SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall', 'HKCU:\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall'); foreach ($root in $roots) {{ Get-ChildItem $root -ErrorAction SilentlyContinue | ForEach-Object {{ $item = Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue; if ($item.DisplayName -like '*{display_name}*' -and $item.WindowsInstaller -eq 1 -and $_.PSChildName -match '^\\{{[0-9A-Fa-f-]{{36}}\\}}$') {{ $_.PSChildName }} }} }}"
    );
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(0x08000000)
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|value| {
            value.len() == 38
                && value.starts_with('{')
                && value.ends_with('}')
                && value[1..37]
                    .chars()
                    .all(|ch| ch.is_ascii_hexdigit() || ch == '-')
        })
        .map(str::to_string)
}

#[cfg(target_os = "windows")]
struct WindowsUninstaller {
    executable: PathBuf,
    arguments: Vec<String>,
}

#[cfg(target_os = "windows")]
fn windows_registered_uninstaller(
    product: &str,
    detected_path: &str,
) -> Result<WindowsUninstaller, String> {
    use std::os::windows::process::CommandExt;
    let display_name = if product == "codexpp" {
        "Codex++"
    } else {
        "Codex"
    };
    let script = format!(
        "$roots = @('HKCU:\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall', 'HKLM:\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall', 'HKLM:\\SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall'); foreach ($root in $roots) {{ Get-ChildItem $root -ErrorAction SilentlyContinue | ForEach-Object {{ $item = Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue; if ($item.DisplayName -like '*{display_name}*') {{ [pscustomobject]@{{ DisplayName=$item.DisplayName; UninstallString=$item.UninstallString; QuietUninstallString=$item.QuietUninstallString }} | ConvertTo-Json -Compress }} }} }}"
    );
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(0x08000000)
        .output()
        .map_err(|error| format!("无法读取卸载信息：{error}"))?;
    if !output.status.success() {
        return Err("无法读取 Windows 中登记的卸载程序".into());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let detected = PathBuf::from(detected_path);
    let detected_root = if detected.is_dir() {
        detected
    } else {
        detected.parent().unwrap_or(&detected).to_path_buf()
    };
    let entries: Vec<serde_json::Value> = text
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    for entry in entries {
        let registered_name = entry
            .get("DisplayName")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if !registered_name
            .to_ascii_lowercase()
            .contains(&display_name.to_ascii_lowercase())
        {
            continue;
        }
        let command_line = entry
            .get("QuietUninstallString")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                entry
                    .get("UninstallString")
                    .and_then(serde_json::Value::as_str)
            })
            .ok_or_else(|| format!("Windows 未登记 {display_name} 的卸载命令"))?;
        let (executable, arguments) = split_windows_command_line(command_line)?;
        let executable_path = PathBuf::from(&executable);
        let is_msi = executable_path
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("msiexec.exe"));
        if is_msi {
            continue;
        }
        if !executable_path.is_file() || !executable_path.starts_with(&detected_root) {
            return Err(format!(
                "{display_name} 卸载程序不在已识别的安装目录内，已停止以避免误删"
            ));
        }
        return Ok(WindowsUninstaller {
            executable: executable_path,
            arguments,
        });
    }
    Err(format!(
        "未找到 {display_name} 的有效卸载程序；请从 Windows 设置中的已安装应用卸载"
    ))
}

#[cfg(target_os = "windows")]
fn split_windows_command_line(command_line: &str) -> Result<(String, Vec<String>), String> {
    let trimmed = command_line.trim();
    let (executable, remainder) = if let Some(quoted) = trimmed.strip_prefix('"') {
        let end = quoted.find('"').ok_or("卸载命令格式无效")?;
        (&quoted[..end], quoted[end + 1..].trim())
    } else {
        let executable_end = trimmed
            .to_ascii_lowercase()
            .find(".exe")
            .map(|index| index + 4)
            .ok_or("卸载命令不是受支持的 EXE 程序")?;
        (&trimmed[..executable_end], trimmed[executable_end..].trim())
    };
    if executable.is_empty() {
        return Err("卸载命令中没有可执行文件".into());
    }
    let arguments = remainder.split_whitespace().map(str::to_owned).collect();
    Ok((executable.to_owned(), arguments))
}

#[tauri::command]
fn device_info() -> serde_json::Value {
    let platform = if cfg!(target_os = "macos") {
        "macos"
    } else {
        "windows"
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    };
    serde_json::json!({ "platform": platform, "arch": arch })
}

#[tauri::command]
async fn fetch_site_config(
    base_url: String,
) -> Result<std::collections::HashMap<String, String>, String> {
    let client = reqwest::Client::builder()
        .https_only(true)
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .get(format!(
            "{}/api/v1/site-config",
            base_url.trim_end_matches('/')
        ))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("后台配置读取失败（HTTP {}）", response.status()));
    }
    response.json().await.map_err(|error| error.to_string())
}

#[tauri::command]
fn open_external_url(url: String) -> Result<(), String> {
    const ALLOWED_URLS: &[&str] = &["https://api.jianguo.ltd/llm.html"];
    if !ALLOWED_URLS.contains(&url.as_str()) {
        return Err("不允许打开此外部地址".into());
    }
    #[cfg(target_os = "windows")]
    {
        Command::new("rundll32.exe")
            .args(["url.dll,FileProtocolHandler", url.as_str()])
            .spawn()
            .map_err(|error| format!("启动默认浏览器失败：{error}"))?;
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(url)
            .spawn()
            .map_err(|error| format!("启动默认浏览器失败：{error}"))?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err("当前平台暂不支持打开外部链接".into())
}

#[cfg(target_os = "macos")]
const AUTO_LOGIN_SERVICE: &str = "com.jianguo.codexinstaller";

#[cfg(target_os = "windows")]
fn auto_login_kami_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_local_data_dir()
        .map_err(|error| error.to_string())?
        .join("auto-login-kami.bin"))
}

#[cfg(target_os = "windows")]
fn protect_auto_login_kami(data: &[u8], protect: bool) -> Result<Vec<u8>, String> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
        },
    };

    let size = u32::try_from(data.len()).map_err(|_| "卡密长度无效")?;
    let input = CRYPT_INTEGER_BLOB {
        cbData: size,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: null_mut(),
    };
    let succeeded = unsafe {
        if protect {
            CryptProtectData(
                &input,
                null(),
                null(),
                null_mut(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                null_mut(),
                null(),
                null_mut(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if succeeded == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let result =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        LocalFree(output.pbData as *mut _);
    }
    Ok(result)
}

#[cfg(target_os = "macos")]
fn keychain_auto_login_kami(action: &str, kami: Option<&str>) -> Result<Option<String>, String> {
    let mut command = Command::new("/usr/bin/security");
    match action {
        "save" => {
            command.args([
                "add-generic-password",
                "-U",
                "-s",
                AUTO_LOGIN_SERVICE,
                "-a",
                "kami",
                "-w",
                kami.ok_or("缺少卡密")?,
            ]);
        }
        "load" => {
            command.args([
                "find-generic-password",
                "-s",
                AUTO_LOGIN_SERVICE,
                "-a",
                "kami",
                "-w",
            ]);
        }
        "clear" => {
            command.args([
                "delete-generic-password",
                "-s",
                AUTO_LOGIN_SERVICE,
                "-a",
                "kami",
            ]);
        }
        _ => return Err("自动登录凭据操作无效".into()),
    }
    let output = command.output().map_err(|error| error.to_string())?;
    if action == "load" {
        if !output.status.success() {
            return Ok(None);
        }
        return Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        ));
    }
    if output.status.success() {
        Ok(None)
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

#[tauri::command]
fn save_auto_login_kami(app: tauri::AppHandle, kami: String) -> Result<(), String> {
    let kami = kami.trim();
    if kami.is_empty() || kami.len() > 512 {
        return Err("卡密无效".into());
    }
    #[cfg(target_os = "windows")]
    {
        let path = auto_login_kami_path(&app)?;
        let parent = path.parent().ok_or("自动登录目录无效")?;
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let encrypted = protect_auto_login_kami(kami.as_bytes(), true)?;
        fs::write(path, encrypted).map_err(|error| error.to_string())?;
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        let _ = app;
        keychain_auto_login_kami("save", Some(kami)).map(|_| ())
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = (app, kami);
        Err("当前系统不支持安全保存自动登录卡密".into())
    }
}

#[tauri::command]
fn load_auto_login_kami(app: tauri::AppHandle) -> Result<Option<String>, String> {
    #[cfg(target_os = "windows")]
    {
        let path = auto_login_kami_path(&app)?;
        let encrypted = match fs::read(path) {
            Ok(data) => data,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        let decrypted = protect_auto_login_kami(&encrypted, false)?;
        return String::from_utf8(decrypted)
            .map(Some)
            .map_err(|error| error.to_string());
    }
    #[cfg(target_os = "macos")]
    {
        let _ = app;
        keychain_auto_login_kami("load", None)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = app;
        Err("当前系统不支持安全读取自动登录卡密".into())
    }
}

#[tauri::command]
fn clear_auto_login_kami(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let path = auto_login_kami_path(&app)?;
        match fs::remove_file(path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.to_string()),
        }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = app;
        match keychain_auto_login_kami("clear", None) {
            Ok(_) => Ok(()),
            Err(_) => Ok(()),
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = app;
        Err("当前系统不支持清除自动登录卡密".into())
    }
}

#[tauri::command]
async fn account_login(
    base_url: String,
    login_type: String,
    kami: String,
    username: String,
    password_hash: String,
) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::builder()
        .https_only(true)
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|error| error.to_string())?;
    let url = format!("{}/installer/login", installer_api_base(&base_url)?);
    let response = client
        .post(url)
        .json(&serde_json::json!({
            "loginType": login_type,
            "kami": kami,
            "username": username,
            "passwordHash": password_hash
        }))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("登录服务返回 HTTP {}", response.status()));
    }
    let mut result = response
        .json::<serde_json::Value>()
        .await
        .map_err(|error| error.to_string())?;
    if let Some(account) = result
        .get_mut("account")
        .and_then(serde_json::Value::as_object_mut)
    {
        account.remove("mainKami");
        account.remove("passwordHash");
    }
    Ok(result)
}

async fn installer_api_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .https_only(true)
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| error.to_string())
}

fn installer_api_base(base_url: &str) -> Result<&str, String> {
    let base = base_url.trim_end_matches('/');
    if base == "https://api.jianguo.ltd" || base == "https://api.jianguo.ltd/api/v1" {
        Ok("https://api.jianguo.ltd/api/v1")
    } else {
        return Err("账户服务地址无效".into());
    }
}

#[tauri::command]
async fn account_balance(base_url: String, kami: String) -> Result<f64, String> {
    let client = installer_api_client().await?;
    let response = client
        .get(format!("{}/balance", installer_api_base(&base_url)?))
        .query(&[("kami", kami.trim())])
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("余额查询失败（HTTP {}）", response.status()));
    }
    let body = response.text().await.map_err(|error| error.to_string())?;
    body.trim()
        .parse::<f64>()
        .map_err(|_| "服务器返回的余额格式无效".into())
}

#[tauri::command]
async fn account_usage(
    base_url: String,
    kami: String,
    page: u32,
    size: u32,
) -> Result<serde_json::Value, String> {
    let client = installer_api_client().await?;
    let response = client
        .get(format!("{}/usage", installer_api_base(&base_url)?))
        .query(&[
            ("kami", kami.trim()),
            ("page", &page.max(1).to_string()),
            ("size", &size.clamp(1, 100).to_string()),
        ])
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("使用记录读取失败（HTTP {}）", response.status()));
    }
    response
        .json()
        .await
        .map_err(|error| format!("使用记录格式无效：{error}"))
}

#[tauri::command]
async fn account_redeem_kami(
    base_url: String,
    account_id: u64,
    main_kami: String,
    redeem_kami: String,
) -> Result<f64, String> {
    if main_kami.trim().is_empty() || redeem_kami.trim().is_empty() {
        return Err("请输入当前登录卡密和充值卡密".into());
    }
    let client = installer_api_client().await?;
    let response = client
        .post(format!(
            "{}/account/redeem-kami",
            installer_api_base(&base_url)?
        ))
        .json(&serde_json::json!({
            "accountId": account_id,
            "mainKami": main_kami.trim(),
            "redeemKami": redeem_kami.trim(),
        }))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let status = response.status();
    let payload = response
        .json::<serde_json::Value>()
        .await
        .map_err(|error| format!("充值服务响应无效：{error}"))?;
    if !status.is_success() || payload.get("ok").and_then(|value| value.as_bool()) != Some(true) {
        return Err(payload
            .get("message")
            .or_else(|| payload.get("error"))
            .and_then(|value| value.as_str())
            .unwrap_or("卡密充值失败，请确认充值卡密有效")
            .to_owned());
    }
    payload
        .get("balance")
        .and_then(|value| value.as_f64())
        .ok_or_else(|| "充值成功，但服务器未返回有效余额".into())
}

#[tauri::command]
async fn open_account_center(
    base_url: String,
    kami: String,
    tab: Option<String>,
) -> Result<(), String> {
    let base = installer_api_base(&base_url)?;
    let client = installer_api_client().await?;
    let response = client
        .post(format!("{base}/account/create-portal-login-ticket"))
        .json(&serde_json::json!({ "kami": kami.trim() }))
        .send()
        .await
        .map_err(|error| format!("账户中心登录票据创建失败：{error}"))?;
    if !response.status().is_success() {
        return Err(format!("账户中心返回 HTTP {}", response.status()));
    }
    let payload = response
        .json::<serde_json::Value>()
        .await
        .map_err(|error| error.to_string())?;
    let ticket = payload
        .get("ticket")
        .and_then(|value| value.as_str())
        .filter(|_| payload.get("ok").and_then(|value| value.as_bool()) == Some(true))
        .ok_or_else(|| {
            payload
                .get("message")
                .and_then(|value| value.as_str())
                .unwrap_or("未能创建账户中心登录票据")
                .to_owned()
        })?;
    let mut url = reqwest::Url::parse("https://api.jianguo.ltd/llm.html")
        .map_err(|error| error.to_string())?;
    url.query_pairs_mut().append_pair("ticket", ticket);
    if let Some(tab) = tab.filter(|value| !value.trim().is_empty()) {
        url.query_pairs_mut().append_pair("tab", tab.trim());
    }
    let url = url.to_string();
    #[cfg(target_os = "windows")]
    {
        Command::new("rundll32.exe")
            .args(["url.dll,FileProtocolHandler", &url])
            .spawn()
            .map_err(|error| format!("启动默认浏览器失败：{error}"))?;
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(url)
            .spawn()
            .map_err(|error| format!("启动默认浏览器失败：{error}"))?;
        Ok(())
    }
}

#[tauri::command]
async fn check_nut_subscription_access(base_url: String, kami: String) -> Result<bool, String> {
    let client = reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(20))
        .no_proxy()
        .build()
        .map_err(|error| format!("无法初始化订阅权限校验：{error}"))?;
    let url = format!(
        "{}/installer/subscription-access",
        installer_api_base(&base_url)?
    );
    let response = client
        .post(url)
        .json(&serde_json::json!({ "kami": kami }))
        .send()
        .await
        .map_err(|error| {
            let is_timeout = error.is_timeout();
            let is_connect = error.is_connect();
            let detail = error.without_url().to_string();
            if is_timeout {
                format!("连接坚果订阅权限服务超时。请检查网络后重试。（{detail}）")
            } else if is_connect {
                format!(
                    "无法连接坚果订阅权限服务。请检查网络连接、DNS 或防火墙设置后重试。（{detail}）"
                )
            } else {
                format!("坚果订阅权限请求失败，请检查网络后重试。（{detail}）")
            }
        })?;
    if !response.status().is_success() {
        return Err(format!("订阅权限服务返回 HTTP {}", response.status()));
    }
    let result = response
        .json::<serde_json::Value>()
        .await
        .map_err(|error| error.to_string())?;
    Ok(result
        .get("allowed")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false))
}

#[tauri::command]
async fn installer_proxy_billing(
    base_url: String,
    kami: String,
    action: String,
    cumulative_bytes: u64,
) -> Result<serde_json::Value, String> {
    if kami.trim().is_empty() || !matches!(action.as_str(), "start" | "report") {
        return Err("坚果订阅计费请求无效".into());
    }
    let client = installer_api_client().await?;
    let response = client
        .post(format!(
            "{}/installer/proxy/{action}",
            installer_api_base(&base_url)?
        ))
        .json(&serde_json::json!({
            "kami": kami.trim(),
            "cumulativeBytes": cumulative_bytes,
        }))
        .send()
        .await
        .map_err(|error| format!("无法连接坚果订阅计费服务：{}", error.without_url()))?;
    let status = response.status();
    let payload = response
        .json::<serde_json::Value>()
        .await
        .map_err(|_| "坚果订阅计费服务响应无效".to_string())?;
    if !status.is_success() {
        return Err(payload
            .get("message")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("坚果订阅计费服务返回 HTTP {status}")));
    }
    Ok(payload)
}

fn installer_download_client(
    proxy_url: Option<&str>,
    timeout: Duration,
) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .read_timeout(Duration::from_secs(20))
        .timeout(timeout)
        .https_only(true)
        .user_agent("CodexUsageAssistant");
    if let Some(proxy_url) = proxy_url.filter(|value| !value.trim().is_empty()) {
        let proxy = reqwest::Url::parse(proxy_url.trim()).map_err(|_| "本机代理地址无效")?;
        let host = proxy.host_str().ok_or("本机代理地址无效")?;
        let loopback = host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback());
        if proxy.scheme() != "http" || !loopback || proxy.port().is_none() {
            return Err("下载代理必须是带端口的本机 HTTP 代理地址".into());
        }
        builder =
            builder.proxy(reqwest::Proxy::all(proxy.as_str()).map_err(|error| error.to_string())?);
    } else {
        builder = builder.no_proxy();
    }
    builder.build().map_err(|error| error.to_string())
}

#[tauri::command]
async fn install_product(
    app: tauri::AppHandle,
    control: State<'_, DownloadControl>,
    product: String,
    platform: String,
    arch: String,
    url: String,
    mode: String,
    silent_args: String,
    sha256: String,
    proxy_url: Option<String>,
) -> Result<String, String> {
    control.cancelled.store(false, Ordering::SeqCst);
    control.paused.store(false, Ordering::SeqCst);
    if !["codex", "codexpp", "v2rayn"].contains(&product.as_str())
        || !["windows", "macos"].contains(&platform.as_str())
        || !["x64", "arm64"].contains(&arch.as_str())
    {
        return Err("不支持的产品或设备平台".into());
    }
    #[cfg(target_os = "windows")]
    if product == "codex" && platform == "windows" {
        let location_app = app.clone();
        let location = tauri::async_runtime::spawn_blocking(move || {
            codex_install_location::selected(&location_app)
        })
        .await
        .map_err(|e| format!("读取 Codex 安装位置失败：{e}"))??;
        if location.is_some() {
            // The Store bootstrapper has no per-app directory argument. Use
            // the same signed official MSIX installer for an explicit target.
            return Box::pin(install_codex_windows_fallback(
                app, control, arch, proxy_url,
            ))
            .await;
        }
    }
    let resource_name = match (product.as_str(), platform.as_str(), arch.as_str()) {
        ("codex", "windows", "x64") => "bundled/codex/windows-x64.exe",
        ("codex", "macos", "x64") => "bundled/codex/macos-x64.dmg",
        ("codex", "macos", "arm64") => "bundled/codex/macos-arm64.dmg",
        ("codexpp", "windows", "x64") => "bundled/codexpp/windows-x64.exe",
        ("codexpp", "macos", "x64") => "bundled/codexpp/macos-x64.dmg",
        ("codexpp", "macos", "arm64") => "bundled/codexpp/macos-arm64.dmg",
        ("v2rayn", "windows", "x64") => "bundled/v2rayn/windows-x64.zip",
        ("v2rayn", "macos", "x64") => "bundled/v2rayn/macos-x64.dmg",
        ("v2rayn", "macos", "arm64") => "bundled/v2rayn/macos-arm64.dmg",
        _ => return Err("当前平台暂无对应内置安装包".into()),
    };
    let bundled = packaged_product_path(&app, resource_name);
    let download_url = if product == "codexpp" && bundled.is_none() {
        latest_codexpp_release_asset(&platform, &arch, proxy_url.as_deref()).await?
    } else {
        url
    };
    let parsed = reqwest::Url::parse(&download_url).map_err(|_| "下载地址无效".to_string())?;
    if parsed.scheme() != "https" && parsed.scheme() != "http" {
        return Err("仅支持通过 HTTP 或 HTTPS 下载安装程序".into());
    }
    if product == "codexpp" && !["silent", "interactive"].contains(&mode.as_str()) {
        return Err("安装方式无效".into());
    }
    let temp_dir = std::env::temp_dir().join("JianGuo").join("Installers");
    fs::create_dir_all(&temp_dir).map_err(|error| error.to_string())?;
    let extension = parsed
        .path()
        .rsplit('.')
        .next()
        .filter(|ext| {
            ["exe", "msi", "dmg", "pkg", "zip"].contains(&ext.to_ascii_lowercase().as_str())
        })
        .map(|ext| format!(".{}", ext.to_ascii_lowercase()))
        .unwrap_or_else(|| {
            if platform == "windows" && product != "v2rayn" {
                ".exe".into()
            } else if platform == "macos" && product != "v2rayn" {
                ".dmg".into()
            } else {
                String::new()
            }
        });
    let filename = format!("{product}-{platform}-{arch}-installer{extension}");
    let path = temp_dir.join(filename);
    let partial_id = format!("{:x}", Sha256::digest(download_url.as_bytes()));
    let partial_path = temp_dir.join(format!("{product}-{platform}-{arch}-{partial_id}.part"));
    let mut digest = Sha256::new();
    let mut total_bytes: u64 = 0;
    let mut reporter = DownloadProgressReporter::new(
        app.clone(),
        if product == "codexpp" {
            "Codex++"
        } else {
            "Codex"
        },
        if let Some(source) = bundled.as_ref() {
            fs::metadata(source).ok().map(|metadata| metadata.len())
        } else {
            None
        },
        bundled.is_none(),
    );
    reporter.report(0, true);
    if let Some(source) = bundled {
        let mut output = tokio::fs::File::create(&path)
            .await
            .map_err(|error| error.to_string())?;
        let mut input = fs::File::open(source).map_err(|error| error.to_string())?;
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let count = input.read(&mut buffer).map_err(|error| error.to_string())?;
            if count == 0 {
                break;
            }
            output
                .write_all(&buffer[..count])
                .await
                .map_err(|error| error.to_string())?;
            digest.update(&buffer[..count]);
            total_bytes += count as u64;
            reporter.report(total_bytes, false);
        }
    } else {
        let client = installer_download_client(proxy_url.as_deref(), Duration::from_secs(600))?;
        total_bytes = Box::pin(download_with_resume(
            &client,
            parsed,
            &partial_path,
            &path,
            &control,
            &mut reporter,
            &mut digest,
        ))
        .await?;
    }
    reporter.report(total_bytes, true);
    if total_bytes == 0 {
        return Err("安装包为空".into());
    }
    let expected = if product == "codexpp" {
        String::new()
    } else {
        sha256.trim().to_ascii_lowercase()
    };
    if !expected.is_empty() {
        if !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) || expected.len() != 64 {
            return Err("SHA-256 格式无效".into());
        }
        let actual = format!("{:x}", digest.finalize());
        if actual != expected {
            return Err("安装包 SHA-256 校验失败".into());
        }
    }
    if control.cancelled.load(Ordering::SeqCst) {
        return Err("DOWNLOAD_CANCELLED".into());
    }
    reporter.phase = "install";
    reporter.resumable = false;
    reporter.status = if platform == "macos" {
        "下载完成，正在打开安装镜像。请将应用拖入“应用程序”文件夹，助手会自动检测安装结果。".into()
    } else {
        "下载完成，正在启动安装程序…".into()
    };
    reporter.report(total_bytes, true);
    tauri::async_runtime::spawn_blocking(move || {
        launch_installer(&path, &product, &platform, &mode, &silent_args)
    })
    .await
    .map_err(|e| format!("启动安装程序失败：{e}"))?
}

async fn download_with_resume(
    client: &reqwest::Client,
    url: reqwest::Url,
    partial_path: &std::path::Path,
    destination: &std::path::Path,
    control: &DownloadControl,
    reporter: &mut DownloadProgressReporter,
    digest: &mut Sha256,
) -> Result<u64, String> {
    let download = async {
        let cache_path = destination.to_owned();
        let cache_url = url.as_str().to_owned();
        let cached = tauri::async_runtime::spawn_blocking(move || {
            installer_cache::load(&cache_path, &cache_url)
        })
        .await
        .map_err(|error| format!("读取安装包缓存失败：{error}"))?;
        if let Some((bytes, cached_digest)) = cached {
            *digest = cached_digest;
            reporter.phase = "cached";
            reporter.total = Some(bytes);
            reporter.status = "已找到完整安装包，正在复用，无需重新下载".into();
            reporter.report(bytes, true);
            return Ok(bytes);
        }
        let bytes = download_with_resume_inner(
            client,
            url.clone(),
            partial_path,
            destination,
            control,
            reporter,
            digest,
        )
        .await?;
        // Only a fully downloaded and renamed file receives a completion receipt.
        // Cache errors must not prevent installation of an otherwise valid file.
        let _ = installer_cache::save(destination, url.as_str(), bytes, digest);
        Ok(bytes)
    };
    tokio::pin!(download);
    let mut cancellation = tokio::time::interval(Duration::from_millis(100));
    loop {
        tokio::select! {
            biased;
            _ = cancellation.tick() => {
                if control.cancelled.load(Ordering::SeqCst) {
                    return Err("DOWNLOAD_CANCELLED".into());
                }
            }
            result = &mut download => return result,
        }
    }
}

async fn download_with_resume_inner(
    client: &reqwest::Client,
    url: reqwest::Url,
    partial_path: &std::path::Path,
    destination: &std::path::Path,
    control: &DownloadControl,
    reporter: &mut DownloadProgressReporter,
    digest: &mut Sha256,
) -> Result<u64, String> {
    let cancelled = || control.cancelled.load(Ordering::SeqCst);
    if cancelled() {
        return Err("DOWNLOAD_CANCELLED".into());
    }
    let mut output = tokio::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(partial_path)
        .await
        .map_err(|error| error.to_string())?;
    output
        .seek(std::io::SeekFrom::End(0))
        .await
        .map_err(|error| error.to_string())?;
    let mut existing = tokio::fs::File::open(partial_path)
        .await
        .map_err(|error| error.to_string())?;
    let mut downloaded = 0u64;
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let count = existing
            .read(&mut buffer)
            .await
            .map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
        downloaded += count as u64;
    }
    drop(existing);
    reporter.report(downloaded, true);

    let mut retries = 0;
    loop {
        if cancelled() {
            return Err("DOWNLOAD_CANCELLED".into());
        }
        while control.paused.load(Ordering::SeqCst) {
            if cancelled() {
                return Err("DOWNLOAD_CANCELLED".into());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let mut request = client.get(url.clone());
        if downloaded > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={downloaded}-"));
        }
        let send = request.try_clone().ok_or("无法恢复下载请求")?.send();
        tokio::pin!(send);
        let response = loop {
            tokio::select! {
                result = &mut send => match result {
                    Ok(response) => break Some(response),
                    Err(error) => {
                        retry_download(&mut retries, &error.to_string(), downloaded, reporter).await?;
                        break None;
                    }
                },
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    if cancelled() {
                        return Err("DOWNLOAD_CANCELLED".into());
                    }
                    if control.paused.load(Ordering::SeqCst) {
                        break None;
                    }
                }
            }
        };
        let Some(response) = response else { continue };
        if response.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE && downloaded > 0 {
            output.set_len(0).await.map_err(|error| error.to_string())?;
            output
                .seek(std::io::SeekFrom::Start(0))
                .await
                .map_err(|error| error.to_string())?;
            downloaded = 0;
            *digest = Sha256::new();
            continue;
        }
        if !response.status().is_success() {
            return Err(format!("安装包下载失败（HTTP {}）", response.status()));
        }
        if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
            let content_range = response
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .ok_or("下载服务器返回了无效的续传范围")?;
            let range_start = content_range
                .strip_prefix("bytes ")
                .and_then(|range| range.split_once('-'))
                .and_then(|(start, _)| start.parse::<u64>().ok())
                .ok_or("下载服务器返回了无效的续传范围")?;
            if range_start != downloaded {
                return Err("下载服务器返回的续传位置不匹配，已保留下载文件，请重试".into());
            }
        }
        if downloaded > 0 && response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            output.set_len(0).await.map_err(|error| error.to_string())?;
            output
                .seek(std::io::SeekFrom::Start(0))
                .await
                .map_err(|error| error.to_string())?;
            downloaded = 0;
            *digest = Sha256::new();
        }
        reporter.total = response.content_length().map(|length| length + downloaded);
        reporter.status.clear();
        let mut stream = response.bytes_stream();
        let mut finished = false;
        let mut stream_error = None;
        loop {
            if cancelled() {
                return Err("DOWNLOAD_CANCELLED".into());
            }
            if control.paused.load(Ordering::SeqCst) {
                break;
            }
            tokio::select! {
                biased;
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    if cancelled() {
                        return Err("DOWNLOAD_CANCELLED".into());
                    }
                    if control.paused.load(Ordering::SeqCst) {
                        break;
                    }
                },
                _ = tokio::time::sleep(Duration::from_millis(750)) => reporter.report(downloaded, true),
                chunk = stream.next() => match chunk {
                    Some(Err(error)) => {
                        stream_error = Some(error.to_string());
                        break;
                    }
                    Some(Ok(chunk)) => {
                        output.write_all(&chunk).await.map_err(|error| error.to_string())?;
                        digest.update(&chunk);
                        downloaded += chunk.len() as u64;
                        reporter.report(downloaded, false);
                    }
                    None => { finished = true; break; }
                }
            }
        }
        output.flush().await.map_err(|error| error.to_string())?;
        drop(stream);
        if finished && reporter.total.is_some_and(|total| downloaded < total) {
            finished = false;
            stream_error = Some("下载连接提前结束".into());
        }
        if let Some(error) = stream_error {
            retry_download(&mut retries, &error, downloaded, reporter).await?;
        }
        if finished {
            drop(output);
            reporter.report(downloaded, true);
            if destination.exists() {
                fs::remove_file(destination).map_err(|error| error.to_string())?;
            }
            fs::rename(partial_path, destination).map_err(|error| error.to_string())?;
            return Ok(downloaded);
        }
        while control.paused.load(Ordering::SeqCst) {
            if cancelled() {
                return Err("DOWNLOAD_CANCELLED".into());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

async fn retry_download(
    retries: &mut u32,
    reason: &str,
    downloaded: u64,
    reporter: &mut DownloadProgressReporter,
) -> Result<(), String> {
    if *retries >= 3 {
        return Err(format!(
            "下载连接中断，自动重连 3 次仍未完成。已保留下载进度，请检查网络或选择可用代理节点后重试。详情：{reason}"
        ));
    }
    *retries += 1;
    reporter.status = format!("连接中断，正在重新连接（{retries}/3），已下载内容会保留");
    reporter.report(downloaded, true);
    tokio::time::sleep(Duration::from_secs(1)).await;
    Ok(())
}

#[tauri::command]
async fn install_codex_windows_fallback(
    app: tauri::AppHandle,
    control: State<'_, DownloadControl>,
    arch: String,
    proxy_url: Option<String>,
) -> Result<String, String> {
    control.cancelled.store(false, Ordering::SeqCst);
    control.paused.store(false, Ordering::SeqCst);
    if !["x64", "arm64"].contains(&arch.as_str()) {
        return Err("当前 Windows 设备架构暂不支持 Codex 备用安装".into());
    }
    #[cfg(target_os = "windows")]
    {
        let location_app = app.clone();
        let install_location = tauri::async_runtime::spawn_blocking(move || {
            codex_install_location::selected(&location_app)
        })
        .await
        .map_err(|e| format!("读取 Codex 安装位置失败：{e}"))??;
        // The frontend can still hold the previous detection snapshot after
        // Windows finishes registering the MSIX package.
        let mut installed_check = Command::new("powershell.exe");
        installed_check
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "if (Test-Path -LiteralPath $env:CODEX_MIGRATION_PENDING) { exit 1 }; $p = Get-AppxPackage -Name 'OpenAI.Codex' | Sort-Object Version -Descending | Select-Object -First 1; if ($p -and (-not $env:CODEX_INSTALL_LOCATION_EXPECTED -or $p.InstallLocation.StartsWith(($env:CODEX_INSTALL_LOCATION_EXPECTED.TrimEnd('\\') + '\\'), [StringComparison]::OrdinalIgnoreCase))) { exit 0 } else { exit 1 }",
            ]);
        installed_check.env(
            "CODEX_INSTALL_LOCATION_EXPECTED",
            install_location
                .as_deref()
                .unwrap_or_else(|| std::path::Path::new("")),
        );
        installed_check.env(
            "CODEX_MIGRATION_PENDING",
            std::env::temp_dir()
                .join("JianGuo")
                .join("Installers")
                .join("codex-migration-pending.json"),
        );
        let already_installed = installed_check
            .creation_flags(0x08000000)
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if already_installed {
            return Ok("already-installed".into());
        }
        let package_url =
            format!("https://persistent.oaistatic.com/codex-app-prod/ChatGPT-{arch}.msix");
        let license_url = "https://persistent.oaistatic.com/codex-app-prod/ChatGPT-License.xml";
        let temp_dir = std::env::temp_dir().join("JianGuo").join("Installers");
        fs::create_dir_all(&temp_dir).map_err(|error| error.to_string())?;
        let package_path = temp_dir.join(format!("ChatGPT-{arch}.msix"));
        let license_path = temp_dir.join("ChatGPT-License.xml");
        let client = installer_download_client(proxy_url.as_deref(), Duration::from_secs(1800))?;
        download_codex_fallback_file(
            &app,
            &control,
            &client,
            &package_url,
            &package_path,
            "Codex MSIX 安装包",
        )
        .await?;
        if let Err(error) = download_codex_fallback_file(
            &app,
            &control,
            &client,
            license_url,
            &license_path,
            "Codex 安装授权文件",
        )
        .await
        {
            return Err(error);
        }

        if control.cancelled.load(Ordering::SeqCst) {
            return Err("DOWNLOAD_CANCELLED".into());
        }
        let install_package = package_path.clone();
        let install_license = license_path.clone();
        let mut reporter = DownloadProgressReporter::new(app.clone(), "Codex", None, false);
        reporter.phase = "install-wait";
        reporter.status = "下载已完成，正在安装 Codex；如出现 Windows 授权提示，请允许安装".into();
        reporter.report(0, true);
        let install_app = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            codex_msix::install(
                &install_package,
                &install_license,
                install_location.as_deref(),
                |status| {
                    reporter.status = status.to_string();
                    reporter.report(0, true);
                },
                || {
                    install_app
                        .state::<DownloadControl>()
                        .cancelled
                        .load(Ordering::SeqCst)
                },
            )
        })
        .await
        .map_err(|error| format!("Codex 安装任务失败：{error}"))??;
        // Keep completed downloads for retries, repair installs and reconfiguration.
        return Ok("installed".into());
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = proxy_url;
        Err("Codex MSIX 备用安装仅支持 Windows".into())
    }
}

#[cfg(target_os = "windows")]
async fn download_codex_fallback_file(
    app: &tauri::AppHandle,
    control: &DownloadControl,
    client: &reqwest::Client,
    url: &str,
    destination: &std::path::Path,
    description: &str,
) -> Result<(), String> {
    let partial_path = destination.with_extension(format!(
        "{}.download",
        destination
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or_default()
    ));
    let parsed = reqwest::Url::parse(url).map_err(|_| "Codex 备用下载地址无效".to_string())?;
    let mut reporter = DownloadProgressReporter::new(app.clone(), description, None, true);
    reporter.report(0, true);
    let mut digest = Sha256::new();
    Box::pin(download_with_resume(
        client,
        parsed,
        &partial_path,
        destination,
        control,
        &mut reporter,
        &mut digest,
    ))
    .await
    .map(|_| ())
    .map_err(|error| format!("{description}下载失败：{error}"))
}

async fn latest_codexpp_release_asset(
    platform: &str,
    arch: &str,
    proxy_url: Option<&str>,
) -> Result<String, String> {
    let asset_suffix = match (platform, arch) {
        ("windows", "x64") => "-windows-x64-setup.exe",
        ("macos", "x64" | "arm64") => "-macos-universal.dmg",
        _ => return Err("Codex++ 上游暂不提供此系统架构的安装包".into()),
    };
    let response = installer_download_client(proxy_url, Duration::from_secs(30))
        .map_err(|error| format!("无法创建 Codex++ 版本查询：{error}"))?
        .get("https://api.github.com/repos/BigPizzaV3/CodexPlusPlus/releases/latest")
        .send()
        .await
        .map_err(|error| format!("查询 Codex++ 最新版本失败：{error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "查询 Codex++ 最新版本失败（HTTP {}）",
            response.status()
        ));
    }
    let release: serde_json::Value = response
        .json()
        .await
        .map_err(|error| format!("Codex++ 版本信息格式无效：{error}"))?;
    release
        .get("assets")
        .and_then(serde_json::Value::as_array)
        .and_then(|assets| {
            assets.iter().find(|asset| {
                asset
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|name| name.ends_with(asset_suffix))
            })
        })
        .and_then(|asset| asset.get("browser_download_url"))
        .and_then(serde_json::Value::as_str)
        .filter(|url| {
            url.starts_with("https://github.com/BigPizzaV3/CodexPlusPlus/releases/download/")
        })
        .map(str::to_owned)
        .ok_or_else(|| format!("Codex++ 最新发行版中没有找到 {platform} {arch} 安装包"))
}

fn packaged_product_path(app: &tauri::AppHandle, resource_name: &str) -> Option<PathBuf> {
    let resource = app
        .path()
        .resolve(resource_name, tauri::path::BaseDirectory::Resource)
        .ok()
        .filter(|path| path.is_file());
    if resource.is_some() {
        return resource;
    }
    let executable = std::env::current_exe().ok()?;
    let portable = executable
        .parent()?
        .join("resources")
        .join(resource_name.replace('/', std::path::MAIN_SEPARATOR_STR));
    portable.is_file().then_some(portable)
}

#[tauri::command]
async fn fetch_codexpp_models(base_url: String, api_key: String) -> Result<UpstreamModels, String> {
    if api_key.trim().is_empty() {
        return Err("API Key 不能为空".into());
    }
    let mut base = reqwest::Url::parse(base_url.trim()).map_err(|_| "供应商接口地址无效")?;
    if base.scheme() != "https" {
        return Err("供应商接口必须使用 HTTPS".into());
    }
    if !base.path().trim_end_matches('/').ends_with("/v1") {
        let path = format!("{}/v1", base.path().trim_end_matches('/'));
        base.set_path(&path);
    }
    base.set_path(&format!("{}/models", base.path().trim_end_matches('/')));
    base.set_query(None);
    base.set_fragment(None);
    let client = reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| format!("无法创建模型查询请求：{error}"))?;
    let response = match client
        .get(base.clone())
        .bearer_auth(api_key.trim())
        .send()
        .await
    {
        Ok(response) => response,
        Err(primary_error) => {
            let direct_client = reqwest::Client::builder()
                .https_only(true)
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(20))
                .no_proxy()
                .build()
                .map_err(|error| format!("无法创建直连模型查询请求：{error}"))?;
            match direct_client
                .get(base)
                .bearer_auth(api_key.trim())
                .send()
                .await
            {
                Ok(response) => response,
                Err(direct_error) => {
                    let primary_timeout = primary_error.is_timeout();
                    let primary_connect = primary_error.is_connect();
                    let direct_timeout = direct_error.is_timeout();
                    let direct_connect = direct_error.is_connect();
                    let primary_detail = primary_error.without_url().to_string();
                    let direct_detail = direct_error.without_url().to_string();
                    if primary_timeout || direct_timeout {
                        return Err(format!(
                            "连接供应商超时。已尝试当前网络代理和直连，请检查网络或代理节点后重试。（代理：{primary_detail}；直连：{direct_detail}）"
                        ));
                    }
                    if primary_connect || direct_connect {
                        return Err(format!(
                            "无法连接供应商。已尝试当前网络代理和直连，请检查网络、DNS、防火墙或代理节点。（代理：{primary_detail}；直连：{direct_detail}）"
                        ));
                    }
                    return Err(format!(
                        "供应商模型列表请求失败。已尝试当前网络代理和直连，请检查网络设置后重试。（代理：{primary_detail}；直连：{direct_detail}）"
                    ));
                }
            }
        }
    };
    let status = response.status();
    if !status.is_success() {
        return Err(match status.as_u16() {
            401 | 403 => "上游拒绝 API Key，请检查密钥权限".into(),
            404 => "上游未提供兼容的 /models 接口".into(),
            _ => format!("上游模型接口返回 HTTP {}", status.as_u16()),
        });
    }
    let payload: serde_json::Value = response
        .json()
        .await
        .map_err(|_| "上游模型列表响应不是有效 JSON".to_string())?;
    parse_upstream_models(&payload)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UpstreamModels {
    models: Vec<String>,
    default_model: Option<String>,
}

fn parse_upstream_models(payload: &serde_json::Value) -> Result<UpstreamModels, String> {
    let entries = payload
        .get("data")
        .and_then(serde_json::Value::as_array)
        .ok_or("上游响应缺少 data 模型列表")?;
    let models = entries
        .iter()
        .filter_map(|item| item.get("id").and_then(serde_json::Value::as_str))
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .fold(Vec::new(), |mut models, model| {
            if !models.contains(&model) {
                models.push(model);
            }
            models
        });
    if models.is_empty() {
        return Err("上游没有返回可用模型 ID".into());
    }

    let default_model = [
        "defaultModel",
        "default_model",
        "default_model_id",
        "default",
    ]
    .iter()
    .find_map(|key| payload.get(*key).and_then(serde_json::Value::as_str))
    .map(str::trim)
    .filter(|model| models.iter().any(|available| available == model))
    .map(str::to_owned)
    .or_else(|| {
        entries.iter().find_map(|entry| {
            let is_default = ["default", "is_default", "isDefault"]
                .iter()
                .any(|key| entry.get(*key).and_then(serde_json::Value::as_bool) == Some(true));
            if is_default {
                entry
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|model| models.iter().any(|available| available == model))
                    .map(str::to_owned)
            } else {
                None
            }
        })
    });

    Ok(UpstreamModels {
        models,
        default_model,
    })
}

#[tauri::command]
async fn check_codexpp_provider(
    base_url: String,
    api_key: String,
) -> Result<UpstreamModels, String> {
    fetch_codexpp_models(base_url, api_key).await
}

#[tauri::command]
async fn probe_provider_protocol(
    base_url: String,
    api_key: String,
    model: String,
) -> Result<serde_json::Value, String> {
    if api_key.trim().is_empty() {
        return Err("API Key 不能为空".into());
    }
    if model.trim().is_empty() {
        return Err("请先填写或选择模型".into());
    }
    let base = reqwest::Url::parse(base_url.trim()).map_err(|_| "供应商接口地址无效")?;
    if base.scheme() != "https" {
        return Err("供应商接口必须使用 HTTPS".into());
    }
    let path = base.path().trim_end_matches('/');
    let prefix = if path.ends_with("/v1") {
        path.to_owned()
    } else {
        format!("{path}/v1")
    };
    let client = reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(18))
        .build()
        .map_err(|error| format!("无法创建协议检测请求：{error}"))?;

    async fn probe(
        client: &reqwest::Client,
        base: &reqwest::Url,
        path: &str,
        body: serde_json::Value,
        api_key: &str,
    ) -> (bool, u16) {
        let mut endpoint = base.clone();
        endpoint.set_path(path);
        endpoint.set_query(None);
        endpoint.set_fragment(None);
        match client
            .post(endpoint)
            .bearer_auth(api_key)
            .json(&body)
            .send()
            .await
        {
            Ok(response) => {
                let status = response.status().as_u16();
                // 400/422 means the endpoint understood the request but rejected
                // its content; it is still useful protocol evidence.  404/405
                // means this route is not exposed by the provider.
                (status < 500 && status != 404 && status != 405, status)
            }
            Err(_) => (false, 0),
        }
    }

    let responses_body = serde_json::json!({
        "model": model.trim(),
        "input": "ping",
        "stream": false,
        "max_output_tokens": 1
    });
    let chat_body = serde_json::json!({
        "model": model.trim(),
        "messages": [{"role": "user", "content": "ping"}],
        "stream": false,
        "max_tokens": 1
    });
    let responses_path = format!("{prefix}/responses");
    let chat_path = format!("{prefix}/chat/completions");
    let (responses, responses_status) = probe(
        &client,
        &base,
        &responses_path,
        responses_body,
        api_key.trim(),
    )
    .await;
    let (chat_completions, chat_status) =
        probe(&client, &base, &chat_path, chat_body, api_key.trim()).await;
    Ok(serde_json::json!({
        "responses": responses,
        "responsesStatus": responses_status,
        "chatCompletions": chat_completions,
        "chatCompletionsStatus": chat_status,
        "recommendedMode": if responses { "direct" } else if chat_completions { "gateway" } else { "unavailable" }
    }))
}

#[tauri::command]
fn local_gateway_status() -> gateway::GatewayStatus {
    gateway::status()
}

#[tauri::command]
fn list_codexpp_providers() -> Result<Vec<serde_json::Value>, String> {
    let path = codexpp_settings_path()?;
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let root: serde_json::Value = serde_json::from_slice(
        &fs::read(&path).map_err(|error| format!("读取 Codex++ 配置失败：{error}"))?,
    )
    .map_err(|error| format!("Codex++ 配置格式无效：{error}"))?;
    let Some(profiles) = root
        .get("relayProfiles")
        .and_then(serde_json::Value::as_array)
    else {
        return Ok(Vec::new());
    };
    let active_id = root
        .get("activeRelayId")
        .and_then(serde_json::Value::as_str);
    // Older Codex++ versions may persist only relayBaseUrl. Use it as a
    // fallback, but never infer the active provider from list order.
    let active_base_url = root
        .get("relayBaseUrl")
        .and_then(serde_json::Value::as_str)
        .map(|value| value.trim_end_matches('/').to_owned());
    let gateway_provider = (active_id == Some("codex-plus-gateway"))
        .then(|| codexpp_gateway_current_provider(&path))
        .flatten();
    Ok(profiles
        .iter()
        .filter_map(|profile| {
            let id = profile.get("id")?.as_str()?;
            let name = profile.get("name")?.as_str()?;
            let base_url = profile
                .get("upstreamBaseUrl")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let models = codexpp_profile_models(profile);
            let model = profile
                .get("testModel")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .or_else(|| models.first().cloned())
                .unwrap_or_default();
            Some(serde_json::json!({
                "id": id,
                "name": name,
                "baseUrl": base_url,
                "model": model,
                "models": models,
                "apiKey": profile.get("authContents").and_then(serde_json::Value::as_str)
                    .and_then(|contents| serde_json::from_str::<serde_json::Value>(contents).ok())
                    .and_then(|auth| auth.get("OPENAI_API_KEY").and_then(serde_json::Value::as_str).map(str::to_owned))
                    .unwrap_or_default(),
                "hasApiKey": profile.get("authContents").and_then(serde_json::Value::as_str)
                    .is_some_and(|value| !value.trim().is_empty()),
                "isActive": active_id == Some(id)
                    || (active_id.is_none()
                        && !base_url.is_empty()
                        && active_base_url.as_deref() == Some(base_url.trim_end_matches('/'))),
                "isCurrent": gateway_provider.as_deref().is_some_and(|current| {
                    current == name
                        || profile
                            .get("id")
                            .and_then(serde_json::Value::as_str)
                            .is_some_and(|profile_id| current == profile_id)
                }),
                "isGateway": id == "codex-plus-gateway" || name == "Codex++ 网关",
                "protocol": profile.get("protocol").and_then(serde_json::Value::as_str).unwrap_or("responses"),
                "relayMode": profile.get("relayMode").and_then(serde_json::Value::as_str).unwrap_or("pureApi"),
            }))
        })
        .collect())
}

fn codexpp_gateway_current_provider(settings_path: &std::path::Path) -> Option<String> {
    let usage_path = settings_path.parent()?.join("gateway-usage.json");
    let root: serde_json::Value = serde_json::from_slice(&fs::read(usage_path).ok()?).ok()?;
    let days = root.get("days")?.as_object()?;
    let (_, entries) = days.iter().max_by_key(|(date, _)| *date)?;
    let entries = entries.as_array()?;
    let entry = entries
        .iter()
        .rev()
        .find(|entry| entry.get("agent").and_then(serde_json::Value::as_str) == Some("codex"))?;
    entry
        .get("provider")
        .and_then(serde_json::Value::as_str)
        .or_else(|| entry.get("providerId").and_then(serde_json::Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

#[tauri::command]
fn set_codexpp_active_provider(provider_id: String) -> Result<(), String> {
    let _guard = PROVIDER_CONFIG_LOCK
        .lock()
        .map_err(|_| "供应商配置锁不可用")?;
    activate_codexpp_provider_at(&codexpp_settings_path()?, &codex_home_dir()?, &provider_id)
}

fn activate_codexpp_provider_at(
    path: &std::path::Path,
    codex_home: &std::path::Path,
    provider_id: &str,
) -> Result<(), String> {
    if provider_id.trim().is_empty() {
        return Err("供应商编号无效".into());
    }
    if !path.is_file() {
        // Codex++ may have been removed or may not have created its settings
        // file yet. The provider is already absent in that state, so deletion
        // is idempotent and should still let the UI refresh successfully.
        return Ok(());
    }
    let mut root: serde_json::Value = serde_json::from_slice(
        &fs::read(&path).map_err(|error| format!("读取 Codex++ 配置失败：{error}"))?,
    )
    .map_err(|error| format!("Codex++ 配置格式无效：{error}"))?;
    let profile = root
        .get("relayProfiles")
        .and_then(serde_json::Value::as_array)
        .and_then(|profiles| {
            profiles
                .iter()
                .find(|profile| profile["id"].as_str() == Some(provider_id))
        })
        .ok_or("所选供应商已不存在")?;
    let base_url = profile
        .get("upstreamBaseUrl")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or("所选供应商缺少接口地址")?
        .to_owned();
    let mut models = codexpp_profile_models(profile);
    if models.is_empty() {
        if let Some(model) = profile["testModel"]
            .as_str()
            .filter(|model| !model.trim().is_empty())
        {
            models.push(model.trim().to_owned());
        }
    }
    let catalog_file = prepare_codex_model_catalog(codex_home, &models)?;
    let model_catalog_json = catalog_file.0.to_string_lossy().into_owned();
    let model_catalog_url = gateway::configure_catalog(models)?;
    if provider_config::gateway_active(&root) && profile["relayMode"].as_str() != Some("gateway") {
        return Err(
            "当前由 Codex++ 网关管理供应商，请先在 Codex++ 中断开网关后再切换直连供应商".into(),
        );
    }
    let mut files = if profile["relayMode"].as_str() == Some("gateway") {
        let api_key = profile
            .get("authContents")
            .and_then(serde_json::Value::as_str)
            .and_then(|contents| serde_json::from_str::<serde_json::Value>(contents).ok())
            .and_then(|auth| auth["OPENAI_API_KEY"].as_str().map(str::to_owned))
            .ok_or("助手网关供应商缺少 API Key")?;
        let endpoint = gateway::configure(gateway::GatewayConfig {
            upstream_base_url: base_url.clone(),
            api_key,
        })?;
        provider_config::gateway_files(
            codex_home,
            profile["name"].as_str().unwrap_or("供应商"),
            &endpoint.base_url,
            profile["testModel"].as_str().unwrap_or(""),
            &endpoint.api_key,
            false,
            false,
            Some(&model_catalog_url),
            Some(&model_catalog_json),
        )?
    } else {
        provider_config::live_files(
            codex_home,
            profile,
            false,
            false,
            Some(&model_catalog_url),
            Some(&model_catalog_json),
        )?
    };
    root["activeRelayId"] = serde_json::json!(provider_id);
    root["relayBaseUrl"] = serde_json::json!(base_url);
    root["relayProfilesEnabled"] = serde_json::Value::Bool(true);
    files.push(catalog_file);
    files.push((
        path.to_path_buf(),
        serde_json::to_vec_pretty(&root).map_err(|e| e.to_string())?,
    ));
    provider_config::commit(files)
}

fn codexpp_profile_models(profile: &serde_json::Value) -> Vec<String> {
    let mut models = Vec::new();
    let add_model = |models: &mut Vec<String>, value: &str| {
        let value = value.trim();
        if !value.is_empty() && !models.iter().any(|existing| existing == value) {
            models.push(value.to_owned());
        }
    };
    match profile.get("modelList") {
        Some(serde_json::Value::String(value)) => {
            for model in value.lines() {
                add_model(&mut models, model);
            }
        }
        Some(serde_json::Value::Array(values)) => {
            for value in values.iter().filter_map(serde_json::Value::as_str) {
                add_model(&mut models, value);
            }
        }
        _ => {}
    }
    if let Some(model) = profile.get("testModel").and_then(serde_json::Value::as_str) {
        add_model(&mut models, model);
    }
    models
}

#[tauri::command]
fn delete_codexpp_provider(provider_id: String) -> Result<(), String> {
    let _guard = PROVIDER_CONFIG_LOCK
        .lock()
        .map_err(|_| "供应商配置锁不可用")?;
    delete_codexpp_provider_at(&codexpp_settings_path()?, &codex_home_dir()?, &provider_id)
}

fn delete_codexpp_provider_at(
    path: &std::path::Path,
    home: &std::path::Path,
    provider_id: &str,
) -> Result<(), String> {
    if provider_id.trim().is_empty() {
        return Err("供应商编号无效".into());
    }
    if !path.is_file() {
        // The settings file may be absent after Codex++ was removed or after
        // it cleaned up the provider. Deletion is intentionally idempotent.
        return Ok(());
    }
    let mut root: serde_json::Value = serde_json::from_slice(
        &fs::read(path).map_err(|error| format!("读取 Codex++ 配置失败：{error}"))?,
    )
    .map_err(|error| format!("Codex++ 配置格式无效：{error}"))?;
    let was_active = root
        .get("activeRelayId")
        .and_then(serde_json::Value::as_str)
        == Some(provider_id);
    let profiles = root
        .get_mut("relayProfiles")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or("Codex++ 供应商列表格式无效")?;
    let Some(index) = profiles
        .iter()
        .position(|profile| profile["id"].as_str() == Some(provider_id))
    else {
        // A stale UI entry can outlive a provider removed by Codex++.
        return Ok(());
    };
    let removed = profiles.remove(index);
    let mut files = provider_config::deletion_files(home, &removed)?;
    if was_active {
        let object = root.as_object_mut().ok_or("Codex++ 配置格式无效")?;
        object.remove("activeRelayId");
        object.remove("relayBaseUrl");
        object.remove("relayApiKey");
        object.insert(
            "relayProfilesEnabled".into(),
            serde_json::Value::Bool(false),
        );
    }
    files.push((
        path.to_path_buf(),
        serde_json::to_vec_pretty(&root).map_err(|error| error.to_string())?,
    ));
    provider_config::commit(files)
}

fn codexpp_settings_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("CODEX_PLUS_STATE_DIR").filter(|s| !s.is_empty()) {
        return Ok(PathBuf::from(path).join("settings.json"));
    }
    #[cfg(target_os = "windows")]
    let candidates = {
        let home = std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .ok_or("无法定位用户目录")?;
        let roaming = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Roaming"));
        let local = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Local"));
        vec![
            home.join(".codex-session-delete/settings.json"),
            roaming.join("com.bigpizzav3.codexplusplus.manager/settings.json"),
            roaming.join("Codex++/settings.json"),
            roaming.join("CodexPlusPlus/settings.json"),
            local.join("com.bigpizzav3.codexplusplus.manager/settings.json"),
            local.join("Codex++/settings.json"),
            local.join("CodexPlusPlus/settings.json"),
        ]
    };
    #[cfg(target_os = "macos")]
    let candidates = {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or("无法定位用户目录")?;
        let support = home.join("Library/Application Support");
        vec![
            home.join(".codex-session-delete/settings.json"),
            support.join("com.bigpizzav3.codexplusplus.manager/settings.json"),
            support.join("Codex++/settings.json"),
            support.join("CodexPlusPlus/settings.json"),
        ]
    };
    candidates
        .iter()
        .find(|candidate| candidate.is_file())
        .cloned()
        .or_else(|| candidates.first().cloned())
        .ok_or_else(|| "无法确定 Codex++ 配置路径".into())
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct SkillCatalogEntry {
    id: String,
    source: String,
    name: String,
    display_name: String,
    description: String,
    version: String,
    author: String,
    tags: Vec<String>,
    license: String,
    path: String,
    tree: String,
    bytes: u64,
    skills: u64,
    requires_auth: bool,
    category: String,
    name_zh: String,
    description_zh: String,
    installed: bool,
}

fn codex_home_dir() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("CODEX_HOME").filter(|s| !s.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    #[cfg(target_os = "windows")]
    let home = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .ok_or("无法定位用户目录")?;
    #[cfg(target_os = "macos")]
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("无法定位用户目录")?;
    Ok(home.join(".codex"))
}

fn codex_plugin_dir(name: &str) -> Result<PathBuf, String> {
    let safe_name = name.trim();
    if safe_name.is_empty()
        || safe_name.contains(['/', '\\', ':'])
        || safe_name == "."
        || safe_name == ".."
    {
        return Err("技能名称无效".into());
    }
    Ok(codex_home_dir()?.join("plugins").join(safe_name))
}

fn skill_installed(name: &str, source: &str) -> bool {
    if source == "composio" {
        return codex_home_dir()
            .map(|path| path.join("skills").join(name).join("SKILL.md").is_file())
            .unwrap_or(false);
    }
    codex_plugin_dir(name)
        .map(|path| path.join(".codex-plugin").join("plugin.json").is_file())
        .unwrap_or(false)
}

fn skill_category(tags: &[String]) -> String {
    let text = tags.join(" ").to_ascii_lowercase();
    if text.contains("design")
        || text.contains("ui")
        || text.contains("frontend")
        || text.contains("graphics")
    {
        return "设计与前端".into();
    }
    if text.contains("research")
        || text.contains("paper")
        || text.contains("citation")
        || text.contains("science")
    {
        return "研究与知识".into();
    }
    if text.contains("video")
        || text.contains("audio")
        || text.contains("image")
        || text.contains("media")
    {
        return "图像与媒体".into();
    }
    if text.contains("writing")
        || text.contains("translation")
        || text.contains("content")
        || text.contains("document")
    {
        return "写作与文档".into();
    }
    if text.contains("productivity")
        || text.contains("memory")
        || text.contains("planning")
        || text.contains("calendar")
    {
        return "效率与工作流".into();
    }
    if text.contains("security") || text.contains("privacy") || text.contains("compliance") {
        return "安全与合规".into();
    }
    if text.contains("mcp")
        || text.contains("api")
        || text.contains("developer")
        || text.contains("coding")
        || text.contains("engineering")
    {
        return "开发与工具".into();
    }
    "其他技能".into()
}

fn skill_tag_zh(tag: &str) -> Option<&'static str> {
    match tag.to_ascii_lowercase().as_str() {
        "ai" | "agent" | "agents" | "agent-skills" => Some("智能体协作"),
        "coding" | "code" | "software-engineering" | "engineering" => Some("编程开发"),
        "testing" | "debugging" | "code-review" | "review" => Some("测试与代码审查"),
        "web" | "browser" | "browser-automation" => Some("网页与浏览器操作"),
        "api" | "mcp" | "integration" => Some("接口与工具集成"),
        "design" | "ui-design" | "web-design" | "frontend" => Some("界面与视觉设计"),
        "research" | "search" | "papers" | "citation" => Some("资料检索与研究"),
        "writing" | "copywriting" | "content" => Some("写作与内容创作"),
        "translation" | "translate" => Some("翻译与语言处理"),
        "document" | "documents" | "markdown" => Some("文档处理"),
        "video" | "video-editing" | "audio" | "image" | "graphics" | "media" => {
            Some("图片、音视频处理")
        }
        "productivity" | "planning" | "workflow" | "automation" => Some("效率与自动化工作流"),
        "security" | "privacy" | "compliance" => Some("安全与合规"),
        "database" | "sql" | "data" | "analytics" => Some("数据与数据库"),
        "finance" | "billing" | "payments" => Some("财务与业务管理"),
        "memory" | "knowledge-management" => Some("记忆与知识管理"),
        _ => None,
    }
}

fn generated_skill_description(category: &str, tags: &[String], display_name: &str) -> String {
    let translated = tags
        .iter()
        .filter_map(|tag| skill_tag_zh(tag))
        .take(3)
        .collect::<Vec<_>>();
    let focus = if translated.is_empty() {
        match category {
            "开发与工具" => "开发、工具调用和自动化任务",
            "设计与前端" => "界面设计、前端开发和视觉创作",
            "研究与知识" => "资料检索、分析和知识整理",
            "图像与媒体" => "图像、音频、视频及多媒体内容处理",
            "写作与文档" => "写作、翻译和文档处理",
            "效率与工作流" => "日常效率提升和工作流自动化",
            "安全与合规" => "安全检查、隐私保护和合规工作",
            _ => "相关的 Codex 辅助工作",
        }
        .to_owned()
    } else {
        translated.join("、")
    };
    format!("{display_name} 是一个用于{focus}的 Codex 技能，可帮助你更高效地完成相关任务。")
}

fn localize_skill(entry: &serde_json::Value) -> SkillCatalogEntry {
    let name = entry
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let display_name = entry
        .get("displayName")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(&name)
        .to_owned();
    let description = entry
        .get("description")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let tags: Vec<String> = entry
        .get("tags")
        .and_then(serde_json::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let category = skill_category(&tags);
    let name_zh = match name.as_str() {
        "agent-skills" => "智能体工程技能".to_owned(),
        "browser" => "浏览器自动化".to_owned(),
        "computer-use" => "电脑操作".to_owned(),
        "visualize" => "数据可视化".to_owned(),
        "writer" => "写作助手".to_owned(),
        "zotero" => "Zotero 文献管理".to_owned(),
        _ => display_name.clone(),
    };
    let description_zh = match name.as_str() {
        "agent-skills" => "覆盖规划、开发、测试、审查和发布的工程化技能集合。".to_owned(),
        "browser" => "让 Codex 能够安全地浏览网页并完成浏览器自动化任务。".to_owned(),
        "computer-use" => "通过桌面操作完成需要图形界面的工作。".to_owned(),
        "visualize" => "创建图表、地图、交互式可视化和数据分析工具。".to_owned(),
        "writer" => "生成可直接使用的邮件、文章、说明和营销文案。".to_owned(),
        _ => generated_skill_description(&category, &tags, &name_zh),
    };
    let installed = skill_installed(&name, "codexpp");
    SkillCatalogEntry {
        id: entry
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&name)
            .to_owned(),
        source: "codexpp".into(),
        name,
        display_name,
        description,
        version: entry
            .get("version")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        author: entry
            .get("author")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        tags,
        license: entry
            .get("license")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        path: entry
            .get("path")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        tree: entry
            .get("tree")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        bytes: entry
            .get("bytes")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default(),
        skills: entry
            .get("skills")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default(),
        requires_auth: entry
            .get("requiresAuth")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        category,
        name_zh,
        description_zh,
        installed,
    }
}

async fn skill_catalog_json() -> Result<serde_json::Value, String> {
    let local = codex_home_dir()?
        .join(".tmp")
        .join("codex-plus-plugin-market")
        .join("index-public.json");
    if local.is_file() {
        if let Ok(bytes) = fs::read(&local) {
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                if value
                    .get("plugins")
                    .is_some_and(serde_json::Value::is_array)
                {
                    return Ok(value);
                }
            }
        }
    }
    let client = reqwest::Client::builder()
        .https_only(true)
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| "SKILL_CATALOG_NETWORK".to_owned())?;
    let response = client.get("https://raw.githubusercontent.com/BigPizzaV3/CodexPlusPlusPluginCache/main/plugin-index.json")
        .send().await.map_err(|_| "SKILL_CATALOG_NETWORK".to_owned())?;
    if !response.status().is_success() {
        return Err(if matches!(response.status().as_u16(), 404 | 410) {
            "SKILL_CATALOG_UNAVAILABLE"
        } else {
            "SKILL_CATALOG_NETWORK"
        }
        .to_owned());
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| "SKILL_CATALOG_NETWORK".to_owned())?;
    let root: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "SKILL_CATALOG_INVALID".to_owned())?;
    if !root.get("plugins").is_some_and(serde_json::Value::is_array) {
        return Err("SKILL_CATALOG_INVALID".into());
    }
    Ok(root)
}

fn composio_skill_localization(name: &str, category: &str) -> (String, String) {
    let (name_zh, purpose) = match name {
        "codebase-migrate" => (
            "代码库迁移",
            "将大型代码库迁移和多文件重构拆分为可审查、可验证的阶段",
        ),
        "create-plan" => ("计划制定", "为复杂任务整理清晰、精简且可执行的步骤"),
        "deploy-pipeline" => ("部署流水线", "组织应用部署流程，并支持验证与回滚"),
        "gh-address-comments" => ("处理 PR 评论", "汇总并处理当前 GitHub 拉取请求中的审查意见"),
        "gh-fix-ci" => ("修复 CI", "检查 GitHub Actions 失败原因并协助修复"),
        "mcp-builder" => ("MCP 服务构建", "构建和评估遵循最佳实践的 MCP 服务"),
        "pr-review-ci-fix" => ("代码审查与 CI 修复", "协助审查代码变更并处理持续集成问题"),
        "sentry-triage" => ("Sentry 错误分诊", "分析 Sentry 错误并定位相关代码"),
        "webapp-testing" => ("Web 应用测试", "为 Web 应用设计和执行有针对性的测试"),
        "connect" => (
            "应用连接",
            "通过 Composio CLI 将 Codex 连接到常用应用并执行操作",
        ),
        "connect-apps" => ("应用连接配置", "配置 Composio CLI 应用连接并启动工作流"),
        "issue-triage" => ("问题分诊", "整理 Linear 或 Jira 待办并识别优先处理事项"),
        "linear" => ("Linear 项目管理", "管理 Linear 中的问题、项目和团队工作流"),
        "meeting-insights-analyzer" => ("会议洞察分析", "从会议记录中提炼主题、风险和后续行动"),
        "meeting-notes-and-actions" => (
            "会议纪要与行动项",
            "将会议记录整理为摘要、决策和负责人明确的行动项",
        ),
        "internal-comms" => ("内部沟通文案", "撰写清楚、得体的内部公告和进度更新"),
        "invoice-organizer" => ("发票整理", "提取并规范发票信息，便于归档和跟踪"),
        "notion-knowledge-capture" => (
            "Notion 知识整理",
            "将对话或笔记整理为结构化且互相关联的 Notion 页面",
        ),
        "notion-meeting-intelligence" => {
            ("Notion 会议准备", "结合 Notion 资料准备会议材料和背景信息")
        }
        "notion-research-documentation" => (
            "Notion 资料研究",
            "整合多个 Notion 来源并生成带引用的研究文档",
        ),
        "notion-spec-to-implementation" => {
            ("Notion 需求落地", "将 Notion 需求转换为实施计划和任务")
        }
        "support-ticket-triage" => ("客服工单分诊", "为客服工单分类、定级并拟定处理建议"),
        "file-organizer" => ("文件整理", "按规则整理、重命名和归档文件"),
        "paperjsx" => (
            "Office 文档生成",
            "根据结构化数据生成演示文稿、文档、表格和报告",
        ),
        "skill-share" => ("技能共享", "在团队成员之间整理和共享技能及可复用指令"),
        "email-draft-polish" => ("邮件起草与润色", "按收件对象和语气要求起草、改写或精简邮件"),
        "changelog-generator" => ("更新日志生成", "根据提交记录或变更摘要生成清晰的更新日志"),
        "content-research-writer" => ("资料研究与写作", "检索资料并撰写带来源依据的内容"),
        "tailored-resume-generator" => ("简历定制", "根据目标职位调整简历并突出可量化成果"),
        "spreadsheet-formula-helper" => {
            ("表格公式助手", "编写和排查表格公式、数据透视表及数组公式")
        }
        "competitive-ads-extractor" => ("竞品广告分析", "分析竞争对手广告并提取结构化信息"),
        "datadog-logs" => ("Datadog 日志分析", "筛选和整理 Datadog 日志并生成摘要"),
        "developer-growth-analysis" => {
            ("开发成长分析", "分析 Codex 对话中的编程习惯和学习提升空间")
        }
        "lead-research-assistant" => ("潜在客户研究", "研究潜在客户并补充企业背景信息"),
        "domain-name-brainstormer" => ("域名创意助手", "根据条件构思并检查可用域名"),
        "raffle-winner-picker" => ("抽奖名单选择", "以可追溯方式随机抽取活动获奖者"),
        "langsmith-fetch" => (
            "LangSmith 数据获取",
            "获取 LangSmith 项目和测试数据以供分析",
        ),
        "helium-mcp" => ("实时信息检索", "通过 MCP 检索新闻、市场数据并整理多方信息"),
        "brand-guidelines" => ("品牌规范应用", "将指定品牌的色彩和字体规范应用到创作内容"),
        "agent-deep-links" => (
            "智能体深层链接",
            "创建并校验 Codex、Cursor 和 VS Code 深层链接",
        ),
        "canvas-design" => ("画布设计", "创建结构清晰的画布布局和视觉设计"),
        "image-enhancer" => ("图片增强", "按设定参数放大并优化图片"),
        "slack-gif-creator" => ("Slack 动图制作", "制作适合 Slack 分享的 GIF 动图"),
        "theme-factory" => ("主题样式工坊", "创建可复用的主题色和样式变量"),
        "video-downloader" => ("视频下载整理", "下载视频并为离线查看做好整理"),
        "template-skill" => ("技能模板", "基于模板创建新的 Codex 技能"),
        "skill-installer" => ("技能安装助手", "从精选目录或 GitHub 路径安装技能"),
        "skill-creator" => ("技能创建指南", "指导创建结构清晰、按需加载的 Codex 技能"),
        _ => {
            return (
                name.replace('-', " "),
                format!("属于“{category}”分类的 Codex 技能，可按其内置流程协助完成相关任务"),
            );
        }
    };
    (name_zh.to_owned(), purpose.to_owned())
}

async fn composio_skill_catalog() -> Result<Vec<SkillCatalogEntry>, String> {
    let client = reqwest::Client::builder()
        .https_only(true)
        .timeout(Duration::from_secs(30))
        .user_agent("Codex-Usage-Assistant")
        .build()
        .map_err(|_| "SKILL_CATALOG_NETWORK".to_owned())?;
    let commit: serde_json::Value = client
        .get("https://api.github.com/repos/composio-community/awesome-codex-skills/commits/HEAD")
        .send()
        .await
        .map_err(|_| "SKILL_CATALOG_NETWORK".to_owned())?
        .error_for_status()
        .map_err(|_| "SKILL_CATALOG_NETWORK".to_owned())?
        .json()
        .await
        .map_err(|_| "SKILL_CATALOG_INVALID".to_owned())?;
    let tree = commit
        .get("sha")
        .and_then(serde_json::Value::as_str)
        .filter(|sha| sha.len() == 40 && sha.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or("SKILL_CATALOG_INVALID")?
        .to_owned();
    let readme_url = format!(
        "https://raw.githubusercontent.com/composio-community/awesome-codex-skills/{tree}/README.md"
    );
    let readme = client
        .get(readme_url)
        .send()
        .await
        .map_err(|_| "SKILL_CATALOG_NETWORK".to_owned())?
        .error_for_status()
        .map_err(|_| "SKILL_CATALOG_NETWORK".to_owned())?
        .text()
        .await
        .map_err(|_| "SKILL_CATALOG_NETWORK".to_owned())?;

    let mut category_en = String::new();
    let mut skills = Vec::new();
    for line in readme.lines() {
        let line = line.trim();
        if let Some(heading) = line.strip_prefix("### ") {
            category_en = heading.to_owned();
            continue;
        }
        let Some(rest) = line.strip_prefix("- [") else {
            continue;
        };
        let Some((title, rest)) = rest.split_once("](./") else {
            continue;
        };
        let Some((path, _description)) = rest.split_once(") - ") else {
            continue;
        };
        let path = path.trim_end_matches('/');
        let Some(name) = path.rsplit('/').next().filter(|name| !name.is_empty()) else {
            continue;
        };
        if skill_archive::validate_codex_skill_request(name, &tree, path).is_err() {
            continue;
        }
        let category = match category_en.as_str() {
            "Development & Code Tools" => "开发与代码工具",
            "Productivity & Collaboration" => "效率与协作",
            "Communication & Writing" => "沟通与写作",
            "Data & Analysis" => "数据与分析",
            "Meta & Utilities" => "元技能与实用工具",
            _ => "其他技能",
        };
        let (name_zh, purpose_zh) = composio_skill_localization(name, category);
        let description_zh = format!("{purpose_zh}。由 Composio 社区收录。");
        skills.push(SkillCatalogEntry {
            id: format!("composio:{name}"),
            source: "composio".into(),
            name: name.to_owned(),
            display_name: title.to_owned(),
            description: description_zh.clone(),
            version: tree[..7].to_owned(),
            author: "Composio 社区".into(),
            tags: vec![category_en.clone()],
            license: "以各技能仓库声明为准".into(),
            path: path.to_owned(),
            tree: tree.clone(),
            bytes: 0,
            skills: 1,
            requires_auth: false,
            category: category.into(),
            name_zh,
            description_zh,
            installed: skill_installed(name, "composio"),
        });
    }
    if skills.is_empty() {
        return Err("SKILL_CATALOG_INVALID".into());
    }
    Ok(skills)
}

#[tauri::command]
async fn list_codex_skills() -> Result<Vec<SkillCatalogEntry>, String> {
    let mut skills = Vec::new();
    let mut failures = Vec::new();
    match skill_catalog_json().await {
        Ok(root) => match root.get("plugins").and_then(serde_json::Value::as_array) {
            Some(entries) => skills.extend(entries.iter().map(localize_skill)),
            None => failures.push("SKILL_CATALOG_INVALID".to_owned()),
        },
        Err(error) => failures.push(error),
    }
    match composio_skill_catalog().await {
        Ok(entries) => skills.extend(entries),
        Err(error) => failures.push(error),
    }
    if skills.is_empty() {
        return Err(failures
            .into_iter()
            .next()
            .unwrap_or_else(|| "SKILL_CATALOG_INVALID".into()));
    }
    Ok(skills)
}

#[tauri::command]
fn list_installed_codex_skills() -> Result<Vec<String>, String> {
    let dir = codex_home_dir()?.join("plugins");
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    Ok(fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            entry
                .path()
                .join(".codex-plugin")
                .join("plugin.json")
                .is_file()
                .then_some(name)
        })
        .collect())
}

#[tauri::command]
async fn install_codex_skill(
    name: String,
    tree: String,
    path: String,
    source: String,
) -> Result<(), String> {
    match source.as_str() {
        "codexpp" => skill_archive::validate_request(&name, &tree, &path)?,
        "composio" => skill_archive::validate_codex_skill_request(&name, &tree, &path)?,
        _ => return Err("技能来源无效，请刷新技能目录后重试".into()),
    }
    let codex = tauri::async_runtime::spawn_blocking(|| {
        #[cfg(target_os = "windows")]
        {
            detect_codex_windows()
        }
        #[cfg(not(target_os = "windows"))]
        {
            detect_codex_macos()
        }
    })
    .await
    .map_err(|error| format!("检测 Codex 安装状态失败：{error}"))?;
    if !codex.installed {
        return Err("尚未检测到已安装的 Codex，请先安装 Codex，再安装技能。".into());
    }
    let destination = if source == "composio" {
        codex_home_dir()?.join("skills").join(&name)
    } else {
        codex_plugin_dir(&name)?
    };
    let archive_url = if source == "composio" {
        format!("https://codeload.github.com/composio-community/awesome-codex-skills/zip/{tree}")
    } else {
        format!("https://codeload.github.com/BigPizzaV3/CodexPlusPlusPluginCache/zip/{tree}")
    };
    let client = reqwest::Client::builder()
        .https_only(true)
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;
    let bytes = client
        .get(archive_url)
        .send()
        .await
        .map_err(|e| format!("技能下载失败：{e}"))?
        .error_for_status()
        .map_err(|e| format!("技能下载失败：{e}"))?
        .bytes()
        .await
        .map_err(|e| format!("技能包读取失败：{e}"))?;
    tauri::async_runtime::spawn_blocking(move || {
        if source == "composio" {
            skill_archive::install_codex_skill(bytes.to_vec(), &path, &name, &destination)
        } else {
            skill_archive::install(bytes.to_vec(), &path, &name, &destination)
        }
    })
    .await
    .map_err(|error| format!("安装技能失败：{error}"))?
}

#[tauri::command]
async fn configure_codexpp_provider(
    name: String,
    base_url: String,
    model: String,
    models: Vec<String>,
    api_key: String,
    provider_id: Option<String>,
    protocol: String,
    relay_mode: String,
    configure_codex_access: bool,
    configure_codex_chinese: bool,
    configure_codexpp_chinese: bool,
    configure_codexpp_start_codex: bool,
) -> Result<String, String> {
    let _guard = PROVIDER_CONFIG_LOCK
        .lock()
        .map_err(|_| "供应商配置锁不可用")?;
    if product_process_running("codex".into())? {
        return Err("请先完全退出 Codex（包括后台进程），再执行一键配置，避免运行中的设置覆盖供应商、模型或权限配置".into());
    }
    if name.trim().is_empty() || model.trim().is_empty() {
        return Err("供应商名称或模型未填写".into());
    }
    if api_key.is_empty() && provider_id.is_none() {
        return Err("新建供应商时必须填写 API Key".into());
    }
    if !matches!(protocol.as_str(), "responses" | "chatCompletions") {
        return Err("不支持的上游协议".into());
    }
    if !matches!(
        relay_mode.as_str(),
        "officialMixApi" | "pureApi" | "gateway"
    ) {
        return Err("请选择纯 API、助手网关或官方登录 + API".into());
    }
    let parsed = reqwest::Url::parse(&base_url).map_err(|_| "供应商接口地址无效".to_string())?;
    if parsed.scheme() != "https" {
        return Err("供应商接口必须使用 HTTPS".into());
    }
    let path = codexpp_settings_path()?;
    let mut root: serde_json::Value = if path.is_file() {
        serde_json::from_slice(
            &fs::read(&path).map_err(|error| format!("读取 Codex++ 配置失败：{error}"))?,
        )
        .map_err(|error| format!("Codex++ 配置格式无效：{error}"))?
    } else {
        serde_json::json!({})
    };
    if !root.is_object() {
        return Err("Codex++ 配置根节点格式无效".into());
    }
    if configure_codexpp_chinese {
        enable_codexpp_force_chinese_locale(&mut root);
    }
    let preserve_gateway = provider_config::gateway_active(&root);
    let profiles = root
        .as_object_mut()
        .unwrap()
        .entry("relayProfiles")
        .or_insert_with(|| serde_json::json!([]));
    if !profiles.is_array() {
        return Err("Codex++ 供应商列表格式无效".into());
    }
    let profiles = profiles.as_array_mut().unwrap();
    let requested_id = provider_id.as_deref().filter(|id| !id.trim().is_empty());
    let normalized_url = base_url.trim_end_matches('/');
    // The UI can hold an ID from an older Codex++ settings file. Resolve the
    // existing record by stable provider identity before falling back to a
    // fresh record, so one-click configuration remains idempotent after an
    // upgrade or a settings-file rebuild.
    let existing_index = requested_id
        .and_then(|id| {
            profiles
                .iter()
                .position(|profile| profile["id"].as_str() == Some(id))
        })
        .or_else(|| {
            profiles.iter().position(|profile| {
                let same_url = profile["upstreamBaseUrl"]
                    .as_str()
                    .is_some_and(|url| url.trim_end_matches('/') == normalized_url);
                let same_name = profile["name"]
                    .as_str()
                    .is_some_and(|value| value.trim() == name.trim());
                same_url || same_name
            })
        });
    let mut profile = existing_index
        .map(|index| profiles[index].clone())
        .unwrap_or_else(|| serde_json::json!({}));
    if profile["id"].as_str() == Some("codex-plus-gateway") {
        return Err("该供应商由 Codex++ 网关管理，请在 Codex++ 中编辑".into());
    }
    let effective_api_key = if api_key.is_empty() {
        let existing_auth = profile
            .get("authContents")
            .and_then(serde_json::Value::as_str)
            .ok_or("该供应商没有已保存的 API Key，请输入密钥后保存")?;
        let parsed_auth: serde_json::Value = serde_json::from_str(existing_auth)
            .map_err(|_| "已有 API Key 配置格式无效，请重新输入密钥")?;
        parsed_auth
            .get("OPENAI_API_KEY")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or("该供应商没有已保存的 API Key，请输入密钥后保存")?
            .to_owned()
    } else {
        api_key
    };
    let profile_id = profile["id"]
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            format!(
                "relay-{:x}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
            )
        });
    if existing_index.is_none()
        && profiles.iter().any(|profile| {
            profile["name"]
                .as_str()
                .is_some_and(|value| value.trim() == name.trim())
        })
    {
        return Err("已存在同名供应商，请从供应商列表编辑该条目".into());
    }
    // Codex always speaks Responses; Codex++ converts other upstream protocols.
    let gateway_endpoint = if relay_mode == "gateway" {
        Some(gateway::configure(gateway::GatewayConfig {
            upstream_base_url: base_url.trim_end_matches('/').to_owned(),
            api_key: effective_api_key.clone(),
        })?)
    } else {
        None
    };
    let effective_base_url = gateway_endpoint
        .as_ref()
        .map(|endpoint| endpoint.base_url.as_str())
        .unwrap_or(base_url.trim_end_matches('/'));
    let effective_key = gateway_endpoint
        .as_ref()
        .map(|endpoint| endpoint.api_key.as_str())
        .unwrap_or(effective_api_key.as_str());
    let toml = provider_config::profile_config(
        name.trim(),
        base_url.trim_end_matches('/'),
        model.trim(),
        &effective_api_key,
        relay_mode == "officialMixApi",
    );
    let mut profile_document = toml
        .parse::<toml_edit::DocumentMut>()
        .map_err(|_| "生成供应商配置失败")?;
    if configure_codex_access {
        profile_document["sandbox_mode"] = toml_edit::value("danger-full-access");
        profile_document["approval_policy"] = toml_edit::value("never");
        profile_document["default_permissions"] = toml_edit::value(":danger-full-access");
    }
    if configure_codex_chinese {
        profile_document["desktop"] = toml_edit::Item::Table(toml_edit::Table::new());
        profile_document["desktop"]["localeOverride"] = toml_edit::value("zh-CN");
    }
    let toml = profile_document.to_string();
    let mut auth = if relay_mode == "officialMixApi" {
        profile
            .get("authContents")
            .and_then(serde_json::Value::as_str)
            .and_then(|contents| serde_json::from_str::<serde_json::Value>(contents).ok())
            .filter(serde_json::Value::is_object)
            .unwrap_or_else(|| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };
    // A profile may be created before the user signs in to Codex. When the
    // official mixed mode is selected, carry the current Codex OAuth tokens
    // into the saved profile so switching providers does not drop the login
    // session. The source of truth remains Codex's auth.json; live_files will
    // still reject the configuration when no access token exists.
    if relay_mode == "officialMixApi" {
        let auth_path = codex_home_dir()?.join("auth.json");
        if let Ok(bytes) = fs::read(&auth_path) {
            if let Ok(saved_auth) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                if let (Some(target), Some(source)) = (auth.as_object_mut(), saved_auth.as_object())
                {
                    for (key, value) in source {
                        target.insert(key.clone(), value.clone());
                    }
                }
            }
        }
    }
    auth["OPENAI_API_KEY"] = serde_json::json!(effective_api_key);
    let profile_object = profile
        .as_object_mut()
        .ok_or("Codex++ 供应商 Profile 格式无效")?;
    profile_object.insert("id".into(), serde_json::json!(profile_id));
    profile_object.insert("name".into(), serde_json::json!(name.trim()));
    profile_object.insert(
        "upstreamBaseUrl".into(),
        serde_json::json!(base_url.trim_end_matches('/')),
    );
    profile_object.insert("protocol".into(), serde_json::json!(protocol));
    profile_object.insert("relayMode".into(), serde_json::json!(relay_mode));
    profile_object.insert(
        "officialMixApiKey".into(),
        serde_json::json!(relay_mode == "officialMixApi"),
    );
    profile_object.insert("testModel".into(), serde_json::json!(model.trim()));
    profile_object.insert("configContents".into(), serde_json::json!(toml));
    profile_object.insert(
        "authContents".into(),
        serde_json::json!(format!("{}\n", auth)),
    );
    profile_object.insert("useCommonConfig".into(), serde_json::json!(true));
    profile_object.insert(
        "contextSelectionInitialized".into(),
        serde_json::json!(true),
    );
    profile_object.insert("modelInsertMode".into(), serde_json::json!("patch"));
    let mut unique_models = Vec::new();
    for value in models {
        let value = value.trim();
        if !value.is_empty() && !unique_models.iter().any(|existing| existing == value) {
            unique_models.push(value.to_owned());
        }
    }
    if !unique_models
        .iter()
        .any(|existing| existing == model.trim())
    {
        unique_models.push(model.trim().to_owned());
    }
    let model_list = unique_models.join("\n");
    profile_object.insert("modelList".into(), serde_json::json!(model_list));
    let apply_direct = !preserve_gateway && provider_config::direct_supported(&profile);
    let catalog_file = if apply_direct || relay_mode == "gateway" {
        Some(prepare_codex_model_catalog(
            &codex_home_dir()?,
            &unique_models,
        )?)
    } else {
        None
    };
    let model_catalog_json = catalog_file
        .as_ref()
        .map(|(path, _)| path.to_string_lossy().into_owned());
    let model_catalog_url = gateway::configure_catalog(unique_models)?;
    let mut files = if apply_direct {
        provider_config::live_files(
            &codex_home_dir()?,
            &profile,
            configure_codex_access,
            configure_codex_chinese,
            Some(&model_catalog_url),
            model_catalog_json.as_deref(),
        )?
    } else if relay_mode == "gateway" {
        provider_config::gateway_files(
            &codex_home_dir()?,
            name.trim(),
            effective_base_url,
            model.trim(),
            effective_key,
            configure_codex_access,
            configure_codex_chinese,
            Some(&model_catalog_url),
            model_catalog_json.as_deref(),
        )?
    } else {
        Vec::new()
    };
    if !apply_direct && relay_mode != "gateway" {
        files.extend(provider_config::option_files(
            &codex_home_dir()?,
            configure_codex_access,
            configure_codex_chinese,
        )?);
    }
    if let Some(catalog_file) = catalog_file {
        files.push(catalog_file);
    }
    if apply_direct && relay_mode == "officialMixApi" {
        if let Some((_, contents)) = files
            .iter()
            .find(|(path, _)| path.file_name().is_some_and(|name| name == "auth.json"))
        {
            profile["authContents"] =
                serde_json::json!(std::str::from_utf8(contents).map_err(|_| "认证文件编码无效")?);
        }
    }
    store_codexpp_profile(profiles, profile, existing_index, &profile_id, name.trim())?;
    let object = root.as_object_mut().unwrap();
    object.remove("relayApiKey");
    if apply_direct {
        object.insert("relayProfilesEnabled".into(), serde_json::json!(true));
        object.insert(
            "relayBaseUrl".into(),
            serde_json::json!(base_url.trim_end_matches('/')),
        );
        object.insert("activeRelayId".into(), serde_json::json!(profile_id));
    }
    files.push((
        path,
        serde_json::to_vec_pretty(&root).map_err(|e| e.to_string())?,
    ));
    files.push(codex_chinese::preference_file(
        &codex_home_dir()?,
        configure_codexpp_chinese,
    ));
    files.push((
        codex_home_dir()?.join(".usage-assistant-full-access.json"),
        serde_json::to_vec(&serde_json::json!({"enabled":configure_codex_access}))
            .map_err(|e| e.to_string())?,
    ));
    provider_config::commit(files)?;
    if configure_codex_access {
        provider_config::verify_full_access(&codex_home_dir()?)?;
    }
    let state_dir = std::env::var("CODEX_PLUS_STATE_DIR")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(|value| PathBuf::from(value.trim()))
        .unwrap_or_else(|| {
            PathBuf::from(
                std::env::var_os(if cfg!(target_os = "windows") {
                    "USERPROFILE"
                } else {
                    "HOME"
                })
                .unwrap_or_default(),
            )
            .join(".codex-session-delete")
        });
    set_codexpp_start_codex_on_open(&state_dir, configure_codexpp_start_codex)?;
    Ok(if relay_mode == "gateway" {
        "供应商已配置到助手本地网关，Codex 将通过网关使用该模型".into()
    } else if apply_direct {
        "供应商与 Codex 认证已同步配置，重启 Codex 后生效".into()
    } else if preserve_gateway {
        "供应商已保存到直连列表；当前网关保持不变。如需在网关中使用，请在 Codex++ 中导入该供应商"
            .into()
    } else {
        "供应商已保存；该接入方式需要 Codex++ 后台服务，请在 Codex++ 中启用此供应商。当前 Codex 配置未改变".into()
    })
}

fn set_codexpp_start_codex_on_open(
    state_dir: &std::path::Path,
    enabled: bool,
) -> Result<(), String> {
    let flag = state_dir.join("watcher.start-codex-on-open");
    if enabled {
        fs::create_dir_all(state_dir)
            .and_then(|_| fs::write(&flag, b"enabled"))
            .map_err(|error| format!("开启 Codex++ 自动启动 Codex 失败：{error}"))
    } else {
        match fs::remove_file(&flag) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("关闭 Codex++ 自动启动 Codex 失败：{error}")),
        }
    }
}

fn store_codexpp_profile(
    profiles: &mut Vec<serde_json::Value>,
    profile: serde_json::Value,
    existing_index: Option<usize>,
    profile_id: &str,
    name: &str,
) -> Result<(), String> {
    if existing_index.is_some() {
        let mut replaced = false;
        profiles.retain_mut(|existing| {
            if existing["id"].as_str() == Some(profile_id) {
                if replaced {
                    return false;
                }
                *existing = profile.clone();
                replaced = true;
                return true;
            }
            existing["name"].as_str() != Some(name)
        });
        if !replaced {
            return Err("要编辑的供应商已不存在，请刷新供应商列表".into());
        }
    } else {
        if profiles
            .iter()
            .any(|existing| existing["name"].as_str() == Some(name))
        {
            return Err("已存在同名供应商，请从供应商列表编辑该条目".into());
        }
        profiles.push(profile);
    }
    Ok(())
}

fn enable_codexpp_force_chinese_locale(settings: &mut serde_json::Value) {
    if let Some(object) = settings.as_object_mut() {
        object.insert(
            "codexAppForceChineseLocale".into(),
            serde_json::Value::Bool(true),
        );
    }
}

#[tauri::command]
fn configure_codex_full_access() -> Result<(), String> {
    configure_codex_options(true, true)
}

#[tauri::command]
fn codex_full_access_enabled() -> bool {
    codex_home_dir()
        .and_then(|home| provider_config::verify_full_access(&home))
        .is_ok()
}

#[tauri::command]
async fn apply_codex_full_access() -> Result<(), String> {
    let port = CODEX_CONTROL_PORT.load(Ordering::SeqCst);
    tauri::async_runtime::spawn_blocking(move || codex_permissions::apply(port))
        .await
        .map_err(|e| format!("Codex 权限校验任务失败：{e}"))?
}

#[tauri::command]
async fn localize_codex() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let detected = {
            #[cfg(target_os = "windows")]
            {
                detect_codex_windows()
            }
            #[cfg(target_os = "macos")]
            {
                detect_codex_macos()
            }
        };
        if !detected.installed {
            return Err("未检测到 Codex 桌面应用，请先安装或重新检测。".into());
        }
        {
            let _guard = PROVIDER_CONFIG_LOCK
                .lock()
                .map_err(|_| "配置正在写入，请稍后重试")?;
            let home = codex_home_dir()?;
            // Validate and prepare first. This changes language only, leaving
            // providers, credentials, permissions and conversation history intact.
            let files = codex_chinese::localization_files(&home)?;
            stop_codex_processes()?;
            #[cfg(target_os = "macos")]
            macos_localization::install(std::path::Path::new(&detected.path), &home)?;
            provider_config::commit(files)?;
        }
        launch_installed_product_with_chinese("codex".into(), false, false)?;
        let port = CODEX_CONTROL_PORT.load(Ordering::SeqCst);
        let verification = if port == 0 {
            Err("未连接 Codex 汉化服务".into())
        } else {
            codex_chinese::apply(port)
        };
        Ok(codex_chinese::localization_result(
            verification,
            cfg!(target_os = "macos"),
        ))
    })
    .await
    .map_err(|e| format!("Codex 汉化任务失败：{e}"))?
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CodexSetupState {
    onboarding_complete: bool,
    full_access_enabled: bool,
}

#[tauri::command]
fn codex_setup_state() -> Result<CodexSetupState, String> {
    let home = codex_home_dir()?;
    Ok(CodexSetupState {
        onboarding_complete: provider_config::onboarding_complete(&home)?,
        full_access_enabled: provider_config::verify_full_access(&home).is_ok(),
    })
}

fn configure_codex_options(configure_access: bool, configure_chinese: bool) -> Result<(), String> {
    let _guard = PROVIDER_CONFIG_LOCK
        .lock()
        .map_err(|_| "供应商配置锁不可用")?;
    if configure_access && product_process_running("codex".into())? {
        return Err("请先完全退出 Codex，再配置完整访问权限".into());
    }
    let home = codex_home_dir()?;
    provider_config::commit(provider_config::option_files(
        &home,
        configure_access,
        configure_chinese,
    )?)?;
    if configure_access {
        provider_config::verify_full_access(&home)?;
    }
    Ok(())
}

#[cfg(test)]
fn set_toml_string(document: &mut String, section: Option<&str>, key: &str, value: &str) {
    set_toml_value(
        document,
        section,
        key,
        &serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into()),
    );
}

#[cfg(test)]
fn set_toml_value(document: &mut String, section: Option<&str>, key: &str, toml_value: &str) {
    let mut lines: Vec<String> = document.lines().map(str::to_owned).collect();
    let target_section = section.unwrap_or("");
    let section_header = if target_section.is_empty() {
        None
    } else {
        Some(format!("[{target_section}]"))
    };
    let mut current_section = String::new();
    let mut section_start = if section_header.is_none() {
        Some(0)
    } else {
        None
    };
    let mut section_end = lines.len();
    let mut key_line = None;
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if section_start.is_some() && current_section == target_section {
                section_end = index;
                break;
            }
            current_section = trimmed
                .trim_start_matches('[')
                .trim_end_matches(']')
                .trim()
                .to_owned();
            if current_section == target_section {
                section_start = Some(index + 1);
            }
            continue;
        }
        if section_start.is_some()
            && current_section == target_section
            && trimmed
                .split_once('=')
                .is_some_and(|(existing_key, _)| existing_key.trim() == key)
        {
            key_line = Some(index);
            break;
        }
    }
    let formatted = format!("{key} = {toml_value}");
    if let Some(index) = key_line {
        lines[index] = formatted;
    } else if let Some(start) = section_start {
        let insert_at = section_end.max(start).min(lines.len());
        lines.insert(insert_at, formatted);
    } else {
        if !lines.is_empty() && !lines.last().is_some_and(|line| line.trim().is_empty()) {
            lines.push(String::new());
        }
        if let Some(header) = section_header {
            lines.push(header);
        }
        lines.push(formatted);
    }
    *document = lines.join("\n");
    if !document.is_empty() {
        document.push('\n');
    }
}

#[cfg(test)]
mod codex_config_tests {
    use super::{
        codexpp_profile_models, enable_codexpp_force_chinese_locale, installer_api_base,
        parse_upstream_models, set_toml_string, store_codexpp_profile,
    };

    #[test]
    fn upstream_models_use_explicit_default_without_relying_on_sort_order() {
        let payload = serde_json::json!({
            "default_model": "gpt-6.1-sol",
            "data": [
                {"id":"claude-opus-5-5"},
                {"id":"gpt-6.1-sol"},
                {"id":"gpt-6.1-sol"}
            ]
        });
        let parsed = parse_upstream_models(&payload).unwrap();
        assert_eq!(parsed.models, ["claude-opus-5-5", "gpt-6.1-sol"]);
        assert_eq!(parsed.default_model.as_deref(), Some("gpt-6.1-sol"));

        let flagged = serde_json::json!({"data":[
            {"id":"claude-opus-5-5"},
            {"id":"gpt-6.1-sol","is_default":true}
        ]});
        assert_eq!(
            parse_upstream_models(&flagged)
                .unwrap()
                .default_model
                .as_deref(),
            Some("gpt-6.1-sol")
        );

        let no_default = serde_json::json!({"data":[{"id":"claude-opus-5-5"}]});
        assert_eq!(
            parse_upstream_models(&no_default).unwrap().default_model,
            None
        );
    }

    fn check_download_recovery(ignore_range: bool, always_stall: bool) {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::time::{Duration, Instant};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
            for attempt in 0..if always_stall { 4 } else { 2 } {
                let deadline = Instant::now() + Duration::from_secs(8);
                let mut connection = loop {
                    match listener.accept() {
                        Ok((connection, _)) => break connection,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "download failed to reconnect");
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(error) => panic!("{error}"),
                    }
                };
                connection.set_nonblocking(false).unwrap();
                connection
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    connection.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                if attempt == 0 || always_stall {
                    connection.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\nConnection: close\r\n\r\nhell").unwrap();
                    connection.flush().unwrap();
                    std::thread::sleep(Duration::from_millis(300));
                } else {
                    assert!(
                        String::from_utf8_lossy(&request)
                            .to_lowercase()
                            .contains("range: bytes=4-")
                    );
                    if ignore_range {
                        connection.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\nConnection: close\r\n\r\nhello world!").unwrap();
                    } else {
                        connection.write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Length: 8\r\nContent-Range: bytes 4-11/12\r\nConnection: close\r\n\r\no world!").unwrap();
                    }
                }
            }
        });
        let directory = std::env::temp_dir().join(format!(
            "codex-download-test-{}-{}",
            std::process::id(),
            address.port()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let partial = directory.join("installer.part");
        let destination = directory.join("installer.exe");
        let mut reporter = super::DownloadProgressReporter {
            emit: Box::new(|_| {}),
            product: "test".into(),
            phase: "download",
            total: None,
            resumable: true,
            status: String::new(),
            last_report: Instant::now(),
            last_bytes: 0,
        };
        let mut digest = sha2::Sha256::default();
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let client = reqwest::Client::builder()
                    .no_proxy()
                    .read_timeout(Duration::from_millis(150))
                    .build()
                    .unwrap();
                super::download_with_resume(
                    &client,
                    format!("http://{address}/installer").parse().unwrap(),
                    &partial,
                    &destination,
                    &super::DownloadControl::default(),
                    &mut reporter,
                    &mut digest,
                )
                .await
            });
        server.join().unwrap();
        if always_stall {
            assert!(result.unwrap_err().contains("自动重连 3 次"));
            assert!(partial.is_file());
            assert!(!destination.exists());
        } else {
            assert_eq!(result.unwrap(), 12);
            assert_eq!(std::fs::read(&destination).unwrap(), b"hello world!");
            use sha2::Digest;
            assert_eq!(digest.finalize(), sha2::Sha256::digest(b"hello world!"));
            assert!(!partial.exists());
            // The HTTP server has exited. Retrying installation must use the
            // completed file without opening another download connection.
            let mut retry_digest = sha2::Sha256::default();
            let retry = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    let client = reqwest::Client::builder()
                        .no_proxy()
                        .timeout(Duration::from_millis(150))
                        .build()
                        .unwrap();
                    super::download_with_resume(
                        &client,
                        format!("http://{address}/installer").parse().unwrap(),
                        &partial,
                        &destination,
                        &super::DownloadControl::default(),
                        &mut reporter,
                        &mut retry_digest,
                    )
                    .await
                });
            assert_eq!(retry.unwrap(), 12);
            assert_eq!(reporter.phase, "cached");
            assert_eq!(
                retry_digest.finalize(),
                sha2::Sha256::digest(b"hello world!")
            );
        }
        let _ = std::fs::remove_file(super::installer_cache::receipt_path(&destination));
        for path in [&partial, &destination] {
            if path.is_file() {
                std::fs::remove_file(path).unwrap();
            }
        }
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn stalled_download_reconnects_with_range() {
        check_download_recovery(false, false);
    }

    #[test]
    fn stalled_download_restarts_safely_when_range_is_ignored() {
        check_download_recovery(true, false);
    }

    #[test]
    fn stalled_download_stops_retrying_and_preserves_partial_file() {
        check_download_recovery(false, true);
    }

    #[test]
    fn installer_futures_fit_within_stack_budget() {
        fn unused<T>() -> T {
            panic!("type inference only; this factory must never run")
        }
        fn check<F: std::future::Future>(name: &str, _factory: impl FnOnce() -> F) {
            let bytes = std::mem::size_of::<F>();
            assert!(bytes < 32 * 1024, "{name} future occupies {bytes} bytes");
        }
        check("install_product", || {
            super::install_product(
                unused(),
                unused(),
                unused(),
                unused(),
                unused(),
                unused(),
                unused(),
                unused(),
                unused(),
                unused(),
            )
        });
        check("download_with_resume", || {
            super::download_with_resume(
                unused(),
                unused(),
                unused(),
                unused(),
                unused(),
                unused(),
                unused(),
            )
        });
        #[cfg(target_os = "windows")]
        check("install_codex_windows_fallback", || {
            super::install_codex_windows_fallback(unused(), unused(), unused(), unused())
        });
    }

    #[test]
    fn installer_download_proxy_is_limited_to_local_http_listener() {
        let timeout = std::time::Duration::from_secs(1);
        assert!(super::installer_download_client(Some("http://127.0.0.1:10809"), timeout).is_ok());
        assert!(super::installer_download_client(Some("http://localhost:10809"), timeout).is_ok());
        assert!(super::installer_download_client(None, timeout).is_ok());
        assert!(
            super::installer_download_client(Some("http://example.com:10809"), timeout).is_err()
        );
        assert!(
            super::installer_download_client(Some("https://127.0.0.1:10809"), timeout).is_err()
        );
    }

    #[test]
    fn provider_model_list_always_contains_selected_default() {
        for list in [
            serde_json::json!("other-model\n"),
            serde_json::json!(["other-model"]),
        ] {
            let profile = serde_json::json!({"modelList":list, "testModel":" selected-model "});
            assert_eq!(
                super::codexpp_profile_models(&profile),
                ["other-model", "selected-model"]
            );
        }
        let profile =
            serde_json::json!({"modelList":"selected-model", "testModel":"selected-model"});
        assert_eq!(super::codexpp_profile_models(&profile), ["selected-model"]);
    }

    #[test]
    fn confirmed_deletion_allows_active_provider_and_preserves_other_configuration() {
        let directory = std::env::temp_dir().join(format!(
            "provider-delete-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("settings.json");
        let profile = serde_json::json!({"id":"remove","name":"Test","upstreamBaseUrl":"https://example.test/v1",
            "testModel":"test-model","authContents":"{\"OPENAI_API_KEY\":\"test-secret\"}"});
        for active in [false, true] {
            let other = serde_json::json!({"id":"keep","name":"Other"});
            std::fs::write(&path, serde_json::to_vec(&serde_json::json!({
                "relayProfiles":[profile.clone(),other.clone()],
                "activeRelayId":if active {"remove"} else {"keep"},
                "relayBaseUrl":"https://example.test/v1","relayProfilesEnabled":true,"unrelated":"preserved"
            })).unwrap()).unwrap();
            let config = if active {
                "model_provider='custom'\nmodel='test-model'\nsandbox_mode='danger-full-access'\n[model_providers.custom]\nname='Test'\nbase_url='https://example.test/v1'\nexperimental_bearer_token='test-secret'\n[model_providers.other]\nname='Other'\n"
            } else {
                "model_provider='other'\nmodel='other-model'\n"
            };
            std::fs::write(directory.join("config.toml"), config).unwrap();
            std::fs::write(directory.join("auth.json"), br#"{"OPENAI_API_KEY":"test-secret","auth_mode":"apikey","tokens":{"access_token":"test-oauth"}}"#).unwrap();
            super::delete_codexpp_provider_at(&path, &directory, "remove").unwrap();
            let root: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(root["relayProfiles"], serde_json::json!([other]));
            assert_eq!(root["unrelated"], "preserved");
            let contents = std::fs::read_to_string(directory.join("config.toml")).unwrap();
            if active {
                assert!(root.get("activeRelayId").is_none());
                assert!(root.get("relayBaseUrl").is_none());
                assert_eq!(root["relayProfilesEnabled"], false);
                let doc = contents.parse::<toml_edit::DocumentMut>().unwrap();
                assert!(doc.get("model_provider").is_none());
                assert!(doc.get("model").is_none());
                assert!(doc["model_providers"].get("custom").is_none());
                assert_eq!(
                    doc["model_providers"]["other"]["name"].as_str(),
                    Some("Other")
                );
                assert_eq!(doc["sandbox_mode"].as_str(), Some("danger-full-access"));
                let auth: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(directory.join("auth.json")).unwrap())
                        .unwrap();
                assert!(auth.get("OPENAI_API_KEY").is_none());
                assert_eq!(auth["tokens"]["access_token"], "test-oauth");
            } else {
                assert_eq!(root["activeRelayId"], "keep");
                assert_eq!(root["relayProfilesEnabled"], true);
                assert_eq!(contents, config);
            }
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn deleting_missing_codexpp_settings_is_idempotent() {
        let directory = std::env::temp_dir().join(format!(
            "provider-delete-missing-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let result = super::delete_codexpp_provider_at(
            &directory.join("settings.json"),
            &directory.join("codex"),
            "stale-provider",
        );
        std::fs::remove_dir_all(directory).unwrap();
        assert!(result.is_ok());
    }

    #[test]
    fn account_api_rejects_non_primary_hosts() {
        assert_eq!(
            installer_api_base("https://api.jianguo.ltd/").unwrap(),
            "https://api.jianguo.ltd/api/v1"
        );
        assert_eq!(
            installer_api_base("https://api.jianguo.ltd/api/v1/").unwrap(),
            "https://api.jianguo.ltd/api/v1"
        );
        assert!(installer_api_base("https://backup.jianguo.ltd").is_err());
        assert!(installer_api_base("http://api.jianguo.ltd").is_err());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn auto_login_secret_round_trips_through_dpapi() {
        let secret = b"local-test-kami";
        let encrypted = super::protect_auto_login_kami(secret, true).unwrap();
        assert_ne!(encrypted, secret);
        assert_eq!(
            super::protect_auto_login_kami(&encrypted, false).unwrap(),
            secret
        );
    }

    #[test]
    fn codexpp_start_on_open_flag_can_be_enabled_and_disabled() {
        let directory = std::env::temp_dir().join(format!(
            "codexinstaller-start-on-open-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let flag = directory.join("watcher.start-codex-on-open");
        super::set_codexpp_start_codex_on_open(&directory, false).unwrap();
        super::set_codexpp_start_codex_on_open(&directory, true).unwrap();
        assert_eq!(std::fs::read(&flag).unwrap(), b"enabled");
        super::set_codexpp_start_codex_on_open(&directory, true).unwrap();
        let settings = directory.join("settings.json");
        std::fs::write(&settings, b"{}").unwrap();
        super::set_codexpp_start_codex_on_open(&directory, false).unwrap();
        assert!(!flag.exists());
        assert_eq!(std::fs::read(&settings).unwrap(), b"{}");
        std::fs::remove_file(settings).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn enables_codexpp_force_chinese_locale_without_changing_other_settings() {
        let mut settings = serde_json::json!({
            "relayProfilesEnabled": true,
            "windowSize": {"width": 1200}
        });

        enable_codexpp_force_chinese_locale(&mut settings);

        assert_eq!(settings["codexAppForceChineseLocale"], true);
        assert_eq!(settings["relayProfilesEnabled"], true);
        assert_eq!(settings["windowSize"]["width"], 1200);
    }

    #[test]
    fn reads_codexpp_models_from_string_or_array_and_falls_back_to_test_model() {
        assert_eq!(
            codexpp_profile_models(&serde_json::json!({
                "testModel": "primary",
                "modelList": ["primary", "secondary", "secondary"]
            })),
            vec!["primary", "secondary"]
        );
        assert_eq!(
            codexpp_profile_models(&serde_json::json!({
                "modelList": "first\nsecond\n"
            })),
            vec!["first", "second"]
        );
        assert_eq!(
            codexpp_profile_models(&serde_json::json!({"testModel": "fallback"})),
            vec!["fallback"]
        );
    }

    #[test]
    fn updates_root_key_without_touching_other_values() {
        let mut document = "model = \"gpt-test\"\nsandbox_mode = \"workspace-write\"\n[provider]\nname = \"kept\"\n".to_owned();
        set_toml_string(&mut document, None, "sandbox_mode", "danger-full-access");
        assert!(document.contains("model = \"gpt-test\""));
        assert!(document.contains("sandbox_mode = \"danger-full-access\""));
        assert!(document.contains("name = \"kept\""));
        assert_eq!(document.matches("sandbox_mode =").count(), 1);
    }

    #[test]
    fn inserts_desktop_locale_into_existing_section() {
        let mut document =
            "model = \"gpt-test\"\n[desktop]\ntheme = \"dark\"\n[provider]\nname = \"kept\"\n"
                .to_owned();
        set_toml_string(&mut document, Some("desktop"), "localeOverride", "zh-CN");
        assert!(document.contains("theme = \"dark\"\nlocaleOverride = \"zh-CN\"\n[provider]"));
        assert!(document.contains("name = \"kept\""));
    }

    #[test]
    fn appends_missing_desktop_section() {
        let mut document = "model = \"gpt-test\"\n".to_owned();
        set_toml_string(&mut document, Some("desktop"), "localeOverride", "zh-CN");
        assert!(document.ends_with("[desktop]\nlocaleOverride = \"zh-CN\"\n"));
    }

    #[test]
    fn editing_provider_replaces_in_place_and_removes_duplicate_names() {
        let mut profiles = vec![
            serde_json::json!({"id":"relay-1", "name":"Nut", "model":"old"}),
            serde_json::json!({"id":"relay-2", "name":"Other"}),
            serde_json::json!({"id":"relay-3", "name":"Nut", "model":"duplicate"}),
        ];
        store_codexpp_profile(
            &mut profiles,
            serde_json::json!({"id":"relay-1", "name":"Nut", "model":"new"}),
            Some(0),
            "relay-1",
            "Nut",
        )
        .unwrap();
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0]["model"], "new");
        assert_eq!(profiles[1]["id"], "relay-2");
    }

    #[test]
    fn provider_config_uses_stable_custom_id_for_chinese_names() {
        let mut document = "model = \"old-model\"\nmodel_provider = \"custom_\"\n[desktop]\nlocaleOverride = \"en-US\"\n".to_owned();
        set_toml_string(&mut document, None, "model_provider", "custom");
        set_toml_string(&mut document, None, "model", "gpt-test");
        set_toml_string(
            &mut document,
            Some("model_providers.custom"),
            "name",
            "坚果供应商",
        );
        set_toml_string(
            &mut document,
            Some("model_providers.custom"),
            "base_url",
            "https://api.example.com/v1",
        );
        set_toml_string(&mut document, Some("desktop"), "localeOverride", "zh-CN");
        assert!(document.contains("model_provider = \"custom\""));
        assert!(document.contains("[model_providers.custom]"));
        assert!(document.contains("name = \"坚果供应商\""));
        assert!(document.contains("base_url = \"https://api.example.com/v1\""));
        assert!(document.contains("localeOverride = \"zh-CN\""));
        assert_eq!(document.matches("model_provider =").count(), 1);
    }
}

fn codexpp_installer_args<'a>(product: &str, mode: &str, configured: &'a str) -> Vec<&'a str> {
    if product != "codexpp" || mode != "silent" {
        return Vec::new();
    }
    if configured.trim().is_empty() {
        // The upstream Windows setup uses NSIS; /S is case-sensitive.
        vec!["/S"]
    } else {
        configured.split_whitespace().collect()
    }
}

#[cfg(test)]
mod installer_args_tests {
    use super::codexpp_installer_args;

    #[test]
    fn paused_download_resumes_from_partial_response() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::time::{Duration, Instant};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut connection, _) = listener.accept().unwrap();
            connection
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                connection.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            connection
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\nConnection: close\r\n\r\n")
                .unwrap();
            connection.flush().unwrap();
            std::thread::sleep(Duration::from_millis(600));
            connection.write_all(b"hell").unwrap();
            connection.flush().unwrap();
            let mut discarded = [0; 64];
            let _ = connection.read(&mut discarded);
            drop(connection);

            let (mut resumed, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                resumed.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            assert!(
                String::from_utf8_lossy(&request)
                    .to_lowercase()
                    .contains("range: bytes=4-")
            );
            resumed.write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Length: 8\r\nContent-Range: bytes 4-11/12\r\nConnection: close\r\n\r\no world!").unwrap();
        });

        let directory =
            std::env::temp_dir().join(format!("codex-pause-resume-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let partial = directory.join("installer.part");
        let destination = directory.join("installer.exe");
        let control = std::sync::Arc::new(super::DownloadControl::default());
        let report_control = control.clone();
        let mut reporter = super::DownloadProgressReporter {
            emit: Box::new(move |progress| {
                if progress.downloaded >= 4 {
                    report_control
                        .paused
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                }
            }),
            product: "test".into(),
            phase: "download",
            total: None,
            resumable: true,
            status: String::new(),
            last_report: Instant::now(),
            last_bytes: 0,
        };
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let client = reqwest::Client::builder()
                    .no_proxy()
                    .read_timeout(Duration::from_secs(3))
                    .build()
                    .unwrap();
                let pause_control = control.clone();
                let pause_and_resume = tokio::spawn(async move {
                    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
                    while tokio::time::Instant::now() < deadline {
                        if pause_control
                            .paused
                            .load(std::sync::atomic::Ordering::SeqCst)
                        {
                            tokio::time::sleep(Duration::from_millis(150)).await;
                            pause_control
                                .paused
                                .store(false, std::sync::atomic::Ordering::SeqCst);
                            return;
                        }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    panic!("download did not enter the paused state");
                });
                let mut digest = sha2::Sha256::default();
                let download = super::download_with_resume(
                    &client,
                    format!("http://{address}/installer").parse().unwrap(),
                    &partial,
                    &destination,
                    &control,
                    &mut reporter,
                    &mut digest,
                );
                let result = tokio::time::timeout(Duration::from_secs(7), download)
                    .await
                    .expect("download should resume after pause");
                pause_and_resume.await.unwrap();
                result
            });
        server.join().unwrap();
        assert_eq!(result.unwrap(), 12);
        assert_eq!(std::fs::read(&destination).unwrap(), b"hello world!");
        std::fs::remove_file(super::installer_cache::receipt_path(&destination)).unwrap();
        std::fs::remove_file(&destination).unwrap();
        std::fs::remove_dir(&directory).unwrap();
    }

    #[test]
    fn paused_download_can_be_cancelled_without_resuming() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let directory =
                    std::env::temp_dir().join(format!("codex-cancel-test-{}", std::process::id()));
                std::fs::create_dir_all(&directory).unwrap();
                let partial = directory.join("installer.part");
                let destination = directory.join("installer.exe");
                std::fs::write(&partial, b"partial").unwrap();
                let control = super::DownloadControl::default();
                control
                    .paused
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                let mut reporter = super::DownloadProgressReporter {
                    emit: Box::new(|_| {}),
                    product: "test".into(),
                    phase: "download",
                    total: None,
                    resumable: true,
                    status: String::new(),
                    last_report: std::time::Instant::now(),
                    last_bytes: 0,
                };
                let client = reqwest::Client::builder().no_proxy().build().unwrap();
                let mut digest = sha2::Sha256::default();
                let download = super::download_with_resume(
                    &client,
                    "http://127.0.0.1:1/unused".parse().unwrap(),
                    &partial,
                    &destination,
                    &control,
                    &mut reporter,
                    &mut digest,
                );
                let cancel = async {
                    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                    control
                        .cancelled
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                };
                let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(2), async {
                    tokio::join!(download, cancel)
                })
                .await
                .expect("paused cancellation must not hang");
                assert_eq!(result.unwrap_err(), "DOWNLOAD_CANCELLED");
                assert!(!destination.exists());
                assert_eq!(std::fs::read(&partial).unwrap(), b"partial");
                std::fs::remove_file(&partial).unwrap();
                std::fs::remove_dir(&directory).unwrap();
            });
    }

    #[test]
    fn automatic_install_defaults_to_nsis_silent_flag() {
        assert_eq!(codexpp_installer_args("codexpp", "silent", ""), ["/S"]);
        assert_eq!(codexpp_installer_args("codexpp", "silent", " \t "), ["/S"]);
    }

    #[test]
    fn preserves_explicit_flags_and_interactive_mode() {
        assert_eq!(
            codexpp_installer_args("codexpp", "silent", "/S /NCRC"),
            ["/S", "/NCRC"]
        );
        assert!(codexpp_installer_args("codexpp", "interactive", "/S").is_empty());
        assert!(codexpp_installer_args("codex", "silent", "").is_empty());
    }
}

fn launch_installer(
    path: &PathBuf,
    product: &str,
    platform: &str,
    mode: &str,
    silent_args: &str,
) -> Result<String, String> {
    #[cfg(target_os = "windows")]
    {
        if platform != "windows" {
            return Err("安装包平台与当前设备不匹配".into());
        }
        if product == "v2rayn"
            && path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
        {
            let target = std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir)
                .join("JianGuo/v2rayN");
            fs::create_dir_all(&target).map_err(|error| error.to_string())?;
            extract_zip(&path, &target)?;
            let executable = target.join("v2rayN.exe");
            if !executable.is_file() {
                return Err("压缩包中未找到 v2rayN.exe".into());
            }
            std::process::Command::new(executable)
                .spawn()
                .map_err(|error| error.to_string())?;
            return Ok("launched".into());
        }
        let mut command = std::process::Command::new(path);
        #[cfg(target_os = "windows")]
        command.creation_flags(0x08000000);
        command.args(codexpp_installer_args(product, mode, silent_args));
        command
            .spawn()
            .map_err(|error| format!("无法启动安装程序：{error}"))?;
        return Ok("launched".into());
    }
    #[cfg(target_os = "macos")]
    {
        let _ = (product, mode, silent_args);
        if platform != "macos" {
            return Err("安装包平台与当前设备不匹配".into());
        }
        if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("dmg"))
        {
            std::process::Command::new("open")
                .arg(path)
                .spawn()
                .map_err(|error| format!("无法打开 DMG：{error}"))?;
            return Ok("manual".into());
        }
        return Err("macOS 当前仅支持 DMG 安装包".into());
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = (path, product, platform, mode, silent_args);
        Err("不支持的操作系统".into())
    }
}

fn extract_zip(archive: &PathBuf, destination: &PathBuf) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    #[cfg(target_os = "windows")]
    let extractor =
        PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into()))
            .join("System32")
            .join("tar.exe");
    #[cfg(not(target_os = "windows"))]
    let extractor = PathBuf::from("tar");
    let mut command = std::process::Command::new(extractor);
    command
        .args(["-xf"])
        .arg(archive)
        .arg("-C")
        .arg(destination);
    #[cfg(target_os = "windows")]
    command.creation_flags(0x08000000);
    let output = command
        .output()
        .map_err(|error| format!("无法启动系统压缩包解压工具：{error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "v2rayN 压缩包解压失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn restore_local_gateway() {
    let Ok(path) = codexpp_settings_path() else {
        return;
    };
    let Ok(bytes) = fs::read(path) else { return };
    let Ok(root) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return;
    };
    let Some(active_id) = root
        .get("activeRelayId")
        .and_then(serde_json::Value::as_str)
    else {
        return;
    };
    let Some(profile) = root
        .get("relayProfiles")
        .and_then(serde_json::Value::as_array)
        .and_then(|profiles| {
            profiles
                .iter()
                .find(|profile| profile["id"].as_str() == Some(active_id))
        })
    else {
        return;
    };
    let mut models = codexpp_profile_models(profile);
    if models.is_empty() {
        if let Some(model) = profile["testModel"]
            .as_str()
            .filter(|model| !model.trim().is_empty())
        {
            models.push(model.trim().to_owned());
        }
    }
    let Ok(home) = codex_home_dir() else { return };
    if !models.is_empty() {
        if let Ok((path, bytes)) = prepare_codex_model_catalog(&home, &models) {
            let _ = fs::write(path, bytes);
        }
    }
    if profile["relayMode"].as_str() != Some("gateway") {
        if !models.is_empty() {
            let _ = gateway::configure_catalog(models);
        }
        return;
    }
    let Some(upstream_base_url) = profile
        .get("upstreamBaseUrl")
        .and_then(serde_json::Value::as_str)
    else {
        return;
    };
    let Some(api_key) = profile
        .get("authContents")
        .and_then(serde_json::Value::as_str)
        .and_then(|contents| serde_json::from_str::<serde_json::Value>(contents).ok())
        .and_then(|auth| auth["OPENAI_API_KEY"].as_str().map(str::to_owned))
    else {
        return;
    };
    let Ok(config_bytes) = fs::read(home.join("config.toml")) else {
        return;
    };
    let Ok(config_text) = std::str::from_utf8(&config_bytes) else {
        return;
    };
    let Ok(doc) = config_text.parse::<toml_edit::DocumentMut>() else {
        return;
    };
    let Some(local_key) = doc
        .get("model_providers")
        .and_then(|providers| providers.get("custom"))
        .and_then(|provider| provider.get("experimental_bearer_token"))
        .and_then(|value| value.as_str())
        .map(str::to_owned)
    else {
        return;
    };
    let _ = gateway::configure_with_key(
        gateway::GatewayConfig {
            upstream_base_url: upstream_base_url.to_owned(),
            api_key,
        },
        local_key,
    );
    if !models.is_empty() {
        let _ = gateway::configure_catalog(models);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app);
        }))
        .manage(DownloadControl::default())
        .manage(account_portal::AccountPortal::default())
        .manage(usage::UsageState::default())
        .manage(wechat::WeChatState::default())
        .manage(V2raynService {
            current: Mutex::new(None),
            startup: AsyncMutex::new(()),
            session: AsyncMutex::new(None),
        })
        .setup(|app| {
            use tauri::{
                menu::{Menu, MenuItem},
                tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
            };

            let open = MenuItem::with_id(app, "open", "打开 Codex 使用助手", true, None::<&str>)?;
            let proxy_on = MenuItem::with_id(app, "proxy-on", "开启网络代理", true, None::<&str>)?;
            let proxy_off =
                MenuItem::with_id(app, "proxy-off", "关闭网络代理", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &proxy_on, &proxy_off, &quit])?;
            let mut tray_builder = TrayIconBuilder::with_id("codex-installer-tray")
                .tooltip("Codex 使用助手 - 网络代理已关闭")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open" => show_main_window(app),
                    "proxy-on" => {
                        let _ = app.emit("tray-proxy-action", true);
                    }
                    "proxy-off" => {
                        let _ = app.emit("tray-proxy-action", false);
                    }
                    "quit" => {
                        if let Some(tray) = app.tray_by_id("codex-installer-tray") {
                            let _ = tray.set_visible(false);
                        }
                        let app = app.clone();
                        std::thread::spawn(move || {
                            std::thread::sleep(Duration::from_millis(250));
                            app.exit(0);
                        });
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if matches!(
                        event,
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } | TrayIconEvent::DoubleClick {
                            button: MouseButton::Left,
                            ..
                        }
                    ) {
                        show_main_window(tray.app_handle());
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray_builder = tray_builder.icon(icon.clone());
            }
            let tray = tray_builder.build(app)?;
            app.manage(tray);
            restore_local_gateway();
            let proxy_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let _ = system_proxy_state(proxy_app).await;
            });

            if let Some(window) = app.get_webview_window("main") {
                let close_window = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = close_window.hide();
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            device_info,
            check_installations,
            create_codex_desktop_shortcut,
            product_process_running,
            stop_codex_processes,
            reset_codex_initial_state,
            launch_installed_product,
            launch_codex_via_codexpp,
            uninstall_product,
            fetch_site_config,
            open_external_url,
            save_auto_login_kami,
            load_auto_login_kami,
            clear_auto_login_kami,
            account_login,
            account_portal::account_portal,
            usage::codex_usage,
            usage::codex_usage_logs,
            wechat::wechat_action,
            account_balance,
            account_usage,
            account_redeem_kami,
            wallet::wallet_catalog,
            wallet::wallet_create_order,
            wallet::wallet_order_status,
            open_account_center,
            check_nut_subscription_access,
            installer_proxy_billing,
            set_download_paused,
            set_download_cancelled,
            install_product,
            codex_install_location::codex_install_options,
            codex_install_location::save_codex_install_location,
            install_codex_windows_fallback,
            fetch_codexpp_models,
            check_codexpp_provider,
            probe_provider_protocol,
            local_gateway_status,
            list_codexpp_providers,
            set_codexpp_active_provider,
            delete_codexpp_provider,
            configure_codexpp_provider,
            configure_codex_full_access,
            codex_full_access_enabled,
            codex_setup_state,
            apply_codex_full_access,
            localize_codex,
            list_codex_skills,
            list_installed_codex_skills,
            install_codex_skill,
            image_plugin::image_plugin_status,
            plugin_market::plugin_market,
            public_plugin_market::public_plugin_market,
            image_plugin::install_image_plugin,
            image_plugin::uninstall_builtin_plugin,
            image_plugin::sync_builtin_plugin_credentials,
            vector_plugin::vector_plugin_status,
            vector_plugin::install_vector_plugin,
            video_plugin::video_plugin_status,
            video_plugin::install_video_plugin,
            system_proxy_state,
            set_system_proxy,
            clear_system_proxy,
            v2rayn_api,
            select_proxy_profile,
            v2rayn_stop
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::ExitRequested { .. }) {
                if let Some(state) = app.try_state::<wechat::WeChatState>() {
                    wechat::shutdown(&state);
                }
                #[cfg(target_os = "windows")]
                if let Err(error) = restore_system_proxy(app) {
                    eprintln!("Failed to restore Windows proxy settings on exit: {error}");
                }
                #[cfg(target_os = "macos")]
                if let Err(error) = set_system_proxy_impl(app.clone(), false, 1) {
                    eprintln!("Failed to restore macOS proxy settings on exit: {error}");
                }
            }
            if matches!(event, tauri::RunEvent::Exit) {
                if let Some(service) = app.try_state::<V2raynService>() {
                    let managed = service
                        .current
                        .lock()
                        .ok()
                        .and_then(|mut guard| guard.take());
                    if let Some(mut current) = managed {
                        tauri::async_runtime::block_on(shutdown_v2rayn(&mut current));
                    }
                }
            }
        });
}
