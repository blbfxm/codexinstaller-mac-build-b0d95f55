//! Owned native folder picker on its own STA; no helper process or WeChat operation lock.
use tauri::Manager;

#[cfg(target_os = "windows")]
fn dialog() -> Result<windows::Win32::UI::Shell::IFileOpenDialog, String> {
    use windows::{
        Win32::{
            System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
            UI::Shell::{
                FOS_DONTADDTORECENT, FOS_FORCEFILESYSTEM, FOS_NOCHANGEDIR, FOS_PICKFOLDERS,
                FileOpenDialog, IFileOpenDialog,
            },
        },
        core::w,
    };
    let dialog: IFileOpenDialog =
        unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) }
            .map_err(|_| "无法创建系统文件夹选择窗口")?;
    unsafe {
        let options = dialog.GetOptions().map_err(|_| "读取文件夹选择设置失败")?;
        dialog
            .SetOptions(
                options
                    | FOS_PICKFOLDERS
                    | FOS_FORCEFILESYSTEM
                    | FOS_NOCHANGEDIR
                    | FOS_DONTADDTORECENT,
            )
            .map_err(|_| "设置文件夹选择窗口失败")?;
        dialog
            .SetTitle(w!("选择 Codex 工作文件夹"))
            .map_err(|_| "设置文件夹选择标题失败")?;
    }
    Ok(dialog)
}

#[cfg(target_os = "windows")]
struct Apartment;
#[cfg(target_os = "windows")]
impl Apartment {
    fn open() -> Result<Self, String> {
        unsafe {
            windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
            )
            .ok()
        }
        .map_err(|_| "初始化系统文件夹选择窗口失败")?;
        Ok(Self)
    }
}
#[cfg(target_os = "windows")]
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { windows::Win32::System::Com::CoUninitialize() };
    }
}

pub async fn choose(app: &tauri::AppHandle) -> Result<String, String> {
    #[cfg(target_os = "windows")]
    let owner = app
        .get_webview_window("main")
        .ok_or("主窗口尚未准备好，请稍后重试")?
        .hwnd()
        .map_err(|_| "无法获取主窗口")?
        .0 as isize;
    let (send, receive) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("assistant-folder-picker".into())
        .spawn(move || {
            let result = (|| -> Result<String, String> {
                #[cfg(target_os = "windows")]
                {
                    use windows::{
                        Win32::{
                            Foundation::HWND, System::Com::CoTaskMemFree,
                            UI::Shell::SIGDN_FILESYSPATH,
                        },
                        core::HRESULT,
                    };
                    let _apartment = Apartment::open()?;
                    let dialog = dialog()?;
                    match unsafe { dialog.Show(Some(HWND(owner as *mut _))) } {
                        Ok(()) => {}
                        Err(e) if e.code() == HRESULT::from_win32(1223) => return Ok(String::new()),
                        Err(_) => {
                            return Err("文件夹选择窗口无法打开，请在输入框中填写完整路径".into());
                        }
                    }
                    let item = unsafe { dialog.GetResult() }.map_err(|_| "读取所选文件夹失败")?;
                    let text = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }
                        .map_err(|_| "读取所选文件夹路径失败")?;
                    let path = unsafe { text.to_string() }.map_err(|_| "所选文件夹路径无法识别");
                    unsafe { CoTaskMemFree(Some(text.0.cast())) };
                    return Ok(path?);
                }
                #[cfg(target_os = "macos")]
                {
                    let output = std::process::Command::new("/usr/bin/osascript")
                        .args([
                            "-e",
                            "POSIX path of (choose folder with prompt \"选择 Codex 工作文件夹\")",
                        ])
                        .output()
                        .map_err(|_| "无法打开文件夹选择窗口")?;
                    return if output.status.success() {
                        Ok(String::from_utf8_lossy(&output.stdout).trim().into())
                    } else if String::from_utf8_lossy(&output.stderr).contains("-128") {
                        Ok(String::new())
                    } else {
                        Err("文件夹选择失败，请直接填写完整路径".into())
                    };
                }
                #[allow(unreachable_code)]
                Err("请直接填写工作文件夹的完整路径".into())
            })();
            let _ = send.send(result);
        })
        .map_err(|_| "打开文件夹选择窗口失败")?;
    receive
        .await
        .map_err(|_| "文件夹选择已结束，请重试".to_string())?
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    #[test]
    fn native_picker_initializes_folder_options_without_helper_process() {
        std::thread::spawn(|| {
            use windows::Win32::UI::Shell::{
                FOS_FORCEFILESYSTEM, FOS_NOCHANGEDIR, FOS_PICKFOLDERS,
            };
            let _apartment = super::Apartment::open().unwrap();
            let dialog = super::dialog().unwrap();
            let options = unsafe { dialog.GetOptions() }.unwrap();
            for flag in [FOS_PICKFOLDERS, FOS_FORCEFILESYSTEM, FOS_NOCHANGEDIR] {
                assert_eq!((options & flag).0, flag.0);
            }
        })
        .join()
        .unwrap();
    }
}
