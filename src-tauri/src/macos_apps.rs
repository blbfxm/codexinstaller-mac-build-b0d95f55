//! Resolve desktop bundles by their metadata, not a guessed executable name.
use std::{fs, path::{Component, Path, PathBuf}};

#[derive(Debug)]
pub struct DesktopApp {
    pub executable: PathBuf,
    pub version: String,
}

pub fn inspect(bundle: &Path, plus_plus: bool) -> Option<DesktopApp> {
    let contents = bundle.join("Contents");
    let info = plist::Value::from_file(contents.join("Info.plist")).ok()?;
    let info = info.as_dictionary()?;
    let id = info.get("CFBundleIdentifier")?.as_string()?.to_ascii_lowercase();
    let name = info.get("CFBundleExecutable")?.as_string()?;
    // Metadata must not redirect launching outside Contents/MacOS.
    let mut parts = Path::new(name).components();
    if !matches!(parts.next(), Some(Component::Normal(_))) || parts.next().is_some() { return None; }
    let executable = contents.join("MacOS").join(name);
    if !executable.is_file() { return None; }
    let known = if plus_plus {
        id.contains("codex-plus-plus") || id.contains("codexplusplus")
            || id.contains("codex-plusplus")
    } else {
        id == "com.openai.codex" || ((id == "com.openai.chat" || id == "com.openai.chatgpt")
            && bundled_cli_candidates(&executable).iter().any(|path| executable_file(path)))
    };
    if !known { return None; }
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        if fs::metadata(&executable).ok()?.permissions().mode() & 0o111 == 0 { return None; }
    }
    Some(DesktopApp {
        executable,
        version: info.get("CFBundleShortVersionString").or_else(|| info.get("CFBundleVersion"))
            .and_then(plist::Value::as_string).unwrap_or_default().into(),
    })
}

pub(crate) fn executable_file(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else { return false };
    if !metadata.is_file() { return false; }
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 { return false; }
    }
    true
}

/// Desktop releases have shipped both a flat CLI and a self-contained CLI package.
/// Keep the package in place: moving its executable can break resource lookup.
pub(crate) fn bundled_cli_candidates(executable: &Path) -> Vec<PathBuf> {
    let Some(contents) = executable.parent().and_then(Path::parent) else { return vec![] };
    let resources = contents.join("Resources");
    let package = resources.join("codex-cli");
    let mut candidates = vec![];
    if let Ok(bytes) = fs::read(package.join("codex-package.json")) {
        if let Ok(manifest) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            if let Some(entrypoint) = manifest["entrypoint"].as_str() {
                let relative = Path::new(entrypoint);
                // An application manifest cannot redirect discovery outside its CLI package.
                if !relative.as_os_str().is_empty() && relative.components().all(|part| matches!(part, Component::Normal(_))) {
                    if let (Ok(root), Ok(target)) = (package.canonicalize(), package.join(relative).canonicalize()) {
                        if target.starts_with(root) { candidates.push(target); }
                    }
                }
            }
        }
    }
    candidates.extend([
        package.join("bin/codex"),
        package.join("CodexCLI.app/Contents/MacOS/codex"),
        resources.join("codex"),
    ]);
    candidates
}

pub(crate) fn session_cli_candidates(
    desktop: Option<&Path>, home: Option<&Path>, search_path: Option<&std::ffi::OsStr>,
) -> Vec<PathBuf> {
    let mut candidates = desktop.map(bundled_cli_candidates).unwrap_or_default();
    let mut roots = vec![PathBuf::from("/Applications")];
    if let Some(home) = home { roots.insert(0, home.join("Applications")); }
    for root in roots {
        for name in ["ChatGPT.app", "Codex.app"] {
            candidates.extend(bundled_cli_candidates(&root.join(name).join("Contents/MacOS/ChatGPT")));
        }
    }
    if let Some(home) = home {
        for directory in [".local/bin", ".cargo/bin", ".volta/bin", ".npm-global/bin"] {
            candidates.push(home.join(directory).join("codex"));
        }
    }
    candidates.extend([PathBuf::from("/opt/homebrew/bin/codex"), PathBuf::from("/usr/local/bin/codex")]);
    if let Some(search_path) = search_path {
        // Do not execute from a relative GUI working directory inherited through PATH.
        candidates.extend(std::env::split_paths(search_path).filter(|path| path.is_absolute()).map(|path| path.join("codex")));
    }
    let mut seen = std::collections::HashSet::new();
    candidates.retain(|path| seen.insert(path.canonicalize().unwrap_or_else(|_| path.clone())));
    candidates
}

pub fn discover_in(roots: &[PathBuf], plus_plus: bool) -> Option<DesktopApp> {
    for root in roots {
        for name in if plus_plus { &["Codex++.app", "CodexPlusPlus.app", "codex-plus-plus.app"][..] }
            else { &["Codex.app", "ChatGPT.app"][..] } {
            if let Some(app) = inspect(&root.join(name), plus_plus) { return Some(app); }
        }
        let Ok(entries) = fs::read_dir(root) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("app")) {
                if let Some(app) = inspect(&path, plus_plus) { return Some(app); }
            }
        }
    }
    None
}

#[cfg(target_os = "macos")]
pub fn discover(plus_plus: bool) -> Option<DesktopApp> {
    use std::{process::{Command, Stdio}, time::{Instant, Duration}};
    let mut roots = vec![PathBuf::from("/Applications")];
    if let Some(home) = std::env::var_os("HOME") { roots.insert(0, PathBuf::from(home).join("Applications")); }
    if let Ok(volumes) = fs::read_dir("/Volumes") {
        roots.extend(volumes.flatten().map(|volume| volume.path().join("Applications")));
    }
    if let Some(app) = discover_in(&roots, plus_plus) { return Some(app); }
    for bundle in registered_bundles(plus_plus) {
        let name = bundle.to_string_lossy();
        if name.contains("/.Trash/") || name.contains("/AppTranslocation/")
            || installer_volume(&bundle) { continue; }
        if let Some(app) = inspect(&bundle, plus_plus) { return Some(app); }
    }
    // Launch Services/Spotlight also finds renamed bundles and applications on
    // other disks. Never count a mounted installer or a trashed app as installed.
    let query = if plus_plus { "kMDItemCFBundleIdentifier == '*codex*plus*'cd" }
        else { "kMDItemCFBundleIdentifier == 'com.openai.codex' || kMDItemCFBundleIdentifier == 'com.openai.chat*'cd" };
    let mut child = Command::new("/usr/bin/mdfind").args(["-0", query])
        .stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if child.try_wait().ok()?.is_some() { break; }
        if Instant::now() >= deadline { let _ = child.kill(); let _ = child.wait(); return None; }
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = child.wait_with_output().ok()?;
    for name in output.stdout.split(|byte| *byte == 0) {
        let Ok(name) = std::str::from_utf8(name) else { continue };
        if name.is_empty() || installer_volume(Path::new(name)) || name.contains("/.Trash/")
            || name.contains("/AppTranslocation/") { continue; }
        if let Some(app) = inspect(Path::new(name), plus_plus) { return Some(app); }
    }
    None
}

#[cfg(target_os = "macos")]
fn installer_volume(bundle: &Path) -> bool {
    // A DMG normally contains a link to /Applications alongside the app. Real
    // installations on external disks can live in any folder and remain valid.
    bundle.ancestors().find(|p| p.parent() == Some(Path::new("/Volumes")))
        .is_some_and(|volume| fs::symlink_metadata(volume.join("Applications"))
            .is_ok_and(|metadata| metadata.file_type().is_symlink()))
}

#[cfg(target_os = "macos")]
fn registered_bundles(plus_plus: bool) -> Vec<PathBuf> {
    use std::{ffi::c_void, ptr};
    type Ref = *const c_void;
    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" { fn LSCopyApplicationURLsForBundleIdentifier(id: Ref, error: *mut Ref) -> Ref; }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithBytes(a: Ref, bytes: *const u8, len: isize, encoding: u32, external: u8) -> Ref;
        fn CFArrayGetCount(a: Ref) -> isize;
        fn CFArrayGetValueAtIndex(a: Ref, index: isize) -> Ref;
        fn CFURLGetFileSystemRepresentation(url: Ref, resolve: u8, buffer: *mut u8, size: isize) -> u8;
        fn CFRelease(value: Ref);
    }
    let ids: &[&str] = if plus_plus { &["com.codexplusplus.app", "com.codex-plus-plus", "com.codex-plus-plus-manager"] }
        else { &["com.openai.codex", "com.openai.chat", "com.openai.chatgpt"] };
    let mut result = vec![];
    for id in ids {
        unsafe {
            let name = CFStringCreateWithBytes(ptr::null(), id.as_ptr(), id.len() as isize, 0x08000100, 0);
            if name.is_null() { continue; }
            let urls = LSCopyApplicationURLsForBundleIdentifier(name, ptr::null_mut());
            CFRelease(name);
            if urls.is_null() { continue; }
            for i in 0..CFArrayGetCount(urls) {
                let mut bytes = [0u8; 4096];
                if CFURLGetFileSystemRepresentation(CFArrayGetValueAtIndex(urls, i), 1, bytes.as_mut_ptr(), bytes.len() as isize) != 0 {
                    if let Ok(path) = std::str::from_utf8(bytes.split(|b| *b == 0).next().unwrap_or_default()) {
                        result.push(PathBuf::from(path));
                    }
                }
            }
            CFRelease(urls);
        }
    }
    result
}

#[cfg(target_os = "macos")]
pub fn process_ids(executable: &Path) -> Result<Vec<u32>, String> {
    let output = std::process::Command::new("/bin/ps").args(["-axo", "pid=,comm="]).output()
        .map_err(|e| format!("读取应用运行状态失败：{e}"))?;
    if !output.status.success() { return Err("读取应用运行状态失败".into()); }
    let bundle = executable.ancestors().find(|p| p.extension().is_some_and(|e| e == "app"))
        .ok_or("未找到应用程序包")?;
    Ok(String::from_utf8_lossy(&output.stdout).lines().filter_map(|line| {
        let (pid, command) = line.trim().split_once(char::is_whitespace)?;
        Path::new(command.trim()).starts_with(bundle).then(|| pid.parse().ok()).flatten()
    }).collect())
}

#[cfg(target_os = "macos")]
pub fn stop(executable: &Path) -> Result<(), String> {
    for pid in process_ids(executable)? {
        let status = std::process::Command::new("/bin/kill").args(["-KILL", &pid.to_string()]).status()
            .map_err(|e| format!("关闭应用失败：{e}"))?;
        if !status.success() && process_ids(executable)?.contains(&pid) { return Err("无法关闭应用，请检查系统权限".into()); }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(root: &Path, name: &str, id: &str, executable: &str) -> PathBuf {
        let app = root.join(name);
        fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        let mut info = plist::Dictionary::new();
        for (key, value) in [("CFBundleIdentifier", id), ("CFBundleExecutable", executable), ("CFBundleShortVersionString", "26.11")] {
            info.insert(key.into(), value.into());
        }
        plist::Value::Dictionary(info).to_file_binary(app.join("Contents/Info.plist")).unwrap();
        let bin = app.join("Contents/MacOS").join(executable);
        fs::write(&bin, b"fixture").unwrap();
        #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; fs::set_permissions(bin, fs::Permissions::from_mode(0o755)).unwrap(); }
        app
    }
    #[test]
    fn desktop_metadata_handles_chatgpt_executable_and_user_applications() {
        let root = std::env::temp_dir().join(format!("mac-app-detection-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let global = root.join("Applications");
        let user = root.join("user/Applications");
        fixture(&global, "ChatGPT.app", "com.openai.chat", "ChatGPT");
        // The standalone ChatGPT app is not the Codex desktop application.
        assert!(discover_in(&[global.clone()], false).is_none());
        let app = fixture(&user, "Codex.app", "com.openai.codex", "ChatGPT");
        let found = discover_in(&[global, user.clone()], false).unwrap();
        assert_eq!(found.executable, app.join("Contents/MacOS/ChatGPT"));
        assert_eq!(found.version, "26.11");
        fs::rename(&app, user.join("我的 Codex.app")).unwrap();
        assert!(discover_in(&[user.clone()], false).is_some());
        fs::remove_file(user.join("我的 Codex.app/Contents/MacOS/ChatGPT")).unwrap();
        assert!(discover_in(&[user], false).is_none());
        fs::remove_dir_all(root).unwrap();
    }

    fn cli_fixture(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[test]
    fn session_engine_finds_new_and_legacy_layouts_in_renamed_external_apps() {
        let root = std::env::temp_dir().join(format!("mac-cli-layouts-{}", std::process::id()));
        let app = fixture(&root.join("External/Applications"), "自定义名字.app", "com.openai.codex", "ChatGPT");
        let desktop = app.join("Contents/MacOS/ChatGPT");
        let resources = app.join("Contents/Resources");
        for relative in ["codex", "codex-cli/bin/codex", "codex-cli/CodexCLI.app/Contents/MacOS/codex"] {
            let engine = resources.join(relative);
            cli_fixture(&engine);
            let found = session_cli_candidates(Some(&desktop), Some(&root.join("home")), None)
                .into_iter().find(|path| executable_file(path)).unwrap();
            assert_eq!(found, engine);
            fs::remove_file(engine).unwrap();
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn manifest_entrypoint_stays_inside_package_and_cli_falls_back_to_user_installation() {
        let root = std::env::temp_dir().join(format!("mac-cli-manifest-{}", std::process::id()));
        let app = fixture(&root, "ChatGPT.app", "com.openai.chatgpt", "ChatGPT");
        let desktop = app.join("Contents/MacOS/ChatGPT");
        let package = app.join("Contents/Resources/codex-cli");
        let engine = package.join("engine/codex");
        cli_fixture(&engine);
        fs::write(package.join("codex-package.json"), br#"{"entrypoint":"engine/codex"}"#).unwrap();
        assert_eq!(bundled_cli_candidates(&desktop).first().unwrap(), &engine.canonicalize().unwrap());
        assert!(inspect(&app, false).is_some(), "New ChatGPT-named Codex bundle is recognized by its CLI package");
        let outside = app.join("Contents/Resources/outside");
        cli_fixture(&outside);
        fs::write(package.join("codex-package.json"), br#"{"entrypoint":"../outside"}"#).unwrap();
        assert!(!bundled_cli_candidates(&desktop).contains(&outside));
        let home = root.join("user");
        let user_cli = home.join(".local/bin/codex");
        cli_fixture(&user_cli);
        assert!(session_cli_candidates(None, Some(&home), None).contains(&user_cli));
        let custom = root.join("custom/bin");
        let path = std::env::join_paths([custom.clone(), PathBuf::from(".")]).unwrap();
        let paths = session_cli_candidates(None, Some(&home), Some(&path));
        assert!(paths.contains(&custom.join("codex")));
        assert!(!paths.contains(&PathBuf::from("./codex")));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn broken_legacy_symlink_does_not_hide_new_cli_and_unexecutable_files_are_rejected() {
        use std::os::unix::{fs::symlink, fs::PermissionsExt};
        let root = std::env::temp_dir().join(format!("mac-cli-symlink-{}", std::process::id()));
        let app = fixture(&root, "Codex.app", "com.openai.codex", "ChatGPT");
        let desktop = app.join("Contents/MacOS/ChatGPT");
        let resources = app.join("Contents/Resources");
        let engine = resources.join("codex-cli/CodexCLI.app/Contents/MacOS/codex");
        cli_fixture(&engine);
        symlink("missing-codex", resources.join("codex")).unwrap();
        let found = bundled_cli_candidates(&desktop).into_iter().find(|path| executable_file(path)).unwrap();
        assert_eq!(found, engine);
        fs::set_permissions(&engine, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!executable_file(&engine));
        symlink(root.join("not-in-package"), resources.join("codex-cli/escape")).unwrap();
        cli_fixture(&root.join("not-in-package"));
        fs::write(resources.join("codex-cli/codex-package.json"), br#"{"entrypoint":"escape"}"#).unwrap();
        assert!(!bundled_cli_candidates(&desktop).contains(&root.join("not-in-package").canonicalize().unwrap()));
        fs::remove_dir_all(root).unwrap();
    }
}
