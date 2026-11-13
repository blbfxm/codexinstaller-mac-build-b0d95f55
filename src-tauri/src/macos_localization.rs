//! Persist the existing language hook in the app's renderer entry point so
//! Finder/Dock launches do not depend on a DevTools session in the assistant.
use std::{fs, io::{Read, Seek, SeekFrom, Write}, path::Path};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const SCRIPT: &[u8] = include_bytes!("codex_chinese.js");
const ENTRY: &str = "webview/index.html";
const ASSET: &str = "webview/assets/usage-assistant-chinese.js";
const TAG: &str = "<script src=\"./assets/usage-assistant-chinese.js\"></script>";
const BLOCK_SIZE: usize = 4 * 1024 * 1024;

fn read_header(archive: &mut fs::File) -> Result<(Value, u64), String> {
    let mut prefix = [0; 16];
    archive.read_exact(&mut prefix).map_err(|e| e.to_string())?;
    let number = |start| u32::from_le_bytes(prefix[start..start + 4].try_into().unwrap()) as usize;
    let length = number(12);
    let header_size = number(4);
    if number(0) != 4 || header_size < 8 || header_size > 32 * 1024 * 1024
        || length > header_size - 8 || number(8) != header_size - 4 {
        return Err("Codex 应用资源头格式不受支持".into());
    }
    let mut header = vec![0; length];
    archive.read_exact(&mut header).map_err(|e| e.to_string())?;
    let header = serde_json::from_slice(&header).map_err(|_| "Codex 应用资源目录格式无效")?;
    Ok((header, 8 + header_size as u64))
}

fn entry<'a>(header: &'a Value, name: &str) -> Result<&'a Value, String> {
    let mut node = header;
    for part in name.split('/') {
        node = node.get("files").and_then(|files| files.get(part))
            .ok_or_else(|| format!("Codex 应用资源缺少 {name}"))?;
    }
    Ok(node)
}

fn read_entry(archive: &mut fs::File, header: &Value, start: u64, name: &str) -> Result<Vec<u8>, String> {
    let node = entry(header, name)?;
    if node.get("unpacked") == Some(&Value::Bool(true)) || node.get("link").is_some() {
        return Err(format!("Codex 应用资源 {name} 格式不受支持"));
    }
    let offset = node["offset"].as_str().and_then(|s| s.parse::<u64>().ok()).ok_or("资源偏移无效")?;
    let size = node["size"].as_u64().filter(|size| *size <= 16 * 1024 * 1024).ok_or("资源大小无效")?;
    let position = start.checked_add(offset).ok_or("资源偏移超出范围")?;
    if position.checked_add(size).is_none_or(|end| end > archive.metadata().map(|m| m.len()).unwrap_or(0)) {
        return Err("Codex 应用资源不完整".into());
    }
    archive.seek(SeekFrom::Start(position)).map_err(|e| e.to_string())?;
    let mut bytes = vec![0; size as usize];
    archive.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}

fn file_entry(offset: u64, bytes: &[u8]) -> Value {
    json!({
        "size": bytes.len(), "offset": offset.to_string(),
        "integrity": {"algorithm":"SHA256", "hash":format!("{:x}", Sha256::digest(bytes)),
            "blockSize":BLOCK_SIZE, "blocks":bytes.chunks(BLOCK_SIZE)
                .map(|block| format!("{:x}", Sha256::digest(block))).collect::<Vec<_>>()}
    })
}

fn set_entry(header: &mut Value, name: &str, node: Value) -> Result<(), String> {
    let (parent, name) = name.rsplit_once('/').ok_or("资源路径无效")?;
    let mut target = header;
    for part in parent.split('/') {
        target = target.get_mut("files").and_then(|files| files.get_mut(part)).ok_or("资源目录缺失")?;
    }
    target.get_mut("files").and_then(Value::as_object_mut).ok_or("资源目录无效")?.insert(name.into(), node);
    Ok(())
}

// Write a new archive beside the original. Existing packed bytes and unpacked
// entries keep their offsets; only the entry page and our script are appended.
fn prepare_archive(source: &Path, destination: &Path) -> Result<Option<String>, String> {
    let mut archive = fs::File::open(source).map_err(|e| format!("读取 Codex 应用资源失败：{e}"))?;
    let (mut header, start) = read_header(&mut archive)?;
    let original = read_entry(&mut archive, &header, start, ENTRY)?;
    let html = std::str::from_utf8(&original).map_err(|_| "Codex 页面不是 UTF-8")?;
    if html.contains(TAG) && read_entry(&mut archive, &header, start, ASSET).ok().as_deref() == Some(SCRIPT) {
        return Ok(None);
    }
    let html = html.replace(TAG, "");
    let Some(head) = html.find("<head>") else { return Err("未找到 Codex 页面入口，无法持久化汉化".into()); };
    // Insert after CSP metadata (if present), before every executable script.
    let insert = html[head..].find("<script").map(|pos| head + pos)
        .or_else(|| html[head..].find("</head>").map(|pos| head + pos)).ok_or("Codex 页面入口不受支持")?;
    let html = format!("{}{}{}", &html[..insert], TAG, &html[insert..]);
    let payload_size = archive.metadata().map_err(|e| e.to_string())?.len().checked_sub(start).ok_or("资源长度无效")?;
    set_entry(&mut header, ENTRY, file_entry(payload_size, html.as_bytes()))?;
    set_entry(&mut header, ASSET, file_entry(payload_size + html.len() as u64, SCRIPT))?;
    let header = serde_json::to_vec(&header).map_err(|e| e.to_string())?;
    let hash = format!("{:x}", Sha256::digest(&header));
    let padded = (header.len() + 3) & !3;
    let mut output = fs::OpenOptions::new().write(true).create_new(true).open(destination)
        .map_err(|e| format!("无法写入 Codex 应用目录，请确认当前用户有修改此应用的权限：{e}"))?;
    for size in [4, padded + 8, padded + 4, header.len()] {
        output.write_all(&(size as u32).to_le_bytes()).map_err(|e| e.to_string())?;
    }
    output.write_all(&header).map_err(|e| e.to_string())?;
    output.write_all(&vec![0; padded - header.len()]).map_err(|e| e.to_string())?;
    archive.seek(SeekFrom::Start(start)).map_err(|e| e.to_string())?;
    std::io::copy(&mut archive, &mut output).map_err(|e| e.to_string())?;
    output.write_all(html.as_bytes()).and_then(|_| output.write_all(SCRIPT))
        .and_then(|_| output.sync_all()).map_err(|e| e.to_string())?;
    Ok(Some(hash))
}

fn updated_plist(path: &Path, hash: &str) -> Result<plist::Value, String> {
    let mut info = plist::Value::from_file(path).map_err(|e| format!("读取应用信息失败：{e}"))?;
    if let Some(integrity) = info.as_dictionary_mut().and_then(|info| info.get_mut("ElectronAsarIntegrity")) {
        let archive = integrity.as_dictionary_mut().and_then(|entries| entries.get_mut("Resources/app.asar"))
            .and_then(plist::Value::as_dictionary_mut).ok_or("应用资源完整性设置格式不受支持")?;
        archive.insert("algorithm".into(), plist::Value::String("SHA256".into()));
        archive.insert("hash".into(), plist::Value::String(hash.into()));
    }
    Ok(info)
}

#[cfg(target_os = "macos")]
fn sign(bundle: &Path) -> Result<(), String> {
    let status = std::process::Command::new("/usr/bin/codesign")
        .args(["--force", "--deep", "--sign", "-", "--preserve-metadata=identifier,entitlements,flags,runtime"])
        .arg(bundle).output().map_err(|e| format!("重新签名 Codex 失败：{e}"))?;
    if !status.status.success() {
        return Err(format!("重新签名 Codex 失败：{}", String::from_utf8_lossy(&status.stderr).trim()));
    }
    verify_signature(bundle)
}

#[cfg(target_os = "macos")]
fn verify_signature(bundle: &Path) -> Result<(), String> {
    let status = std::process::Command::new("/usr/bin/codesign").args(["--verify", "--deep", "--strict"])
        .arg(bundle).output().map_err(|e| format!("校验 Codex 签名失败：{e}"))?;
    if !status.status.success() { return Err(format!("校验 Codex 签名失败：{}", String::from_utf8_lossy(&status.stderr).trim())); }
    Ok(())
}

#[cfg(target_os = "macos")]
pub(crate) fn install(executable: &Path, home: &Path) -> Result<(), String> {
    install_with_signer(executable, home, sign, verify_signature)
}

fn install_with_signer(
    executable: &Path,
    home: &Path,
    sign: impl Fn(&Path) -> Result<(), String>,
    verify: impl Fn(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let bundle = executable.ancestors().find(|path| path.extension().is_some_and(|ext| ext == "app"))
        .ok_or("未找到 Codex 应用程序包")?;
    let archive = bundle.join("Contents/Resources/app.asar");
    let info = bundle.join("Contents/Info.plist");
    let temporary = archive.with_extension("usage-assistant.tmp");
    // Never remove a pre-existing staging file owned by an earlier failed run.
    if temporary.exists() { return Err(format!("存在未完成的汉化临时文件，请先检查：{}", temporary.display())); }
    let prepared = prepare_archive(&archive, &temporary);
    let hash = match prepared {
        Ok(Some(hash)) => hash,
        Ok(None) => return verify(bundle),
        Err(error) => { let _ = fs::remove_file(&temporary); return Err(error); }
    };
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
    let backup = home.join("backup").join(format!("macos-localization-{stamp}"));
    let result = (|| {
        let updated = updated_plist(&info, &hash)?;
        fs::create_dir_all(&backup).map_err(|e| e.to_string())?;
        fs::copy(&archive, backup.join("app.asar")).map_err(|e| format!("备份应用资源失败：{e}"))?;
        fs::copy(&info, backup.join("Info.plist")).map_err(|e| format!("备份应用信息失败：{e}"))?;
        fs::rename(&temporary, &archive).map_err(|e| format!("替换应用资源失败：{e}"))?;
        if let Err(error) = updated.to_file_xml(&info).map_err(|e| e.to_string()).and_then(|_| sign(bundle)) {
            let restored = fs::copy(backup.join("app.asar"), &archive)
                .and_then(|_| fs::copy(backup.join("Info.plist"), &info));
            let recovery = restored.map_err(|e| e.to_string()).and_then(|_| sign(bundle));
            return Err(format!("{error}。恢复结果：{}。备份位置：{}", recovery.err().unwrap_or_else(|| "已恢复".into()), backup.display()));
        }
        Ok(())
    })();
    let _ = fs::remove_file(&temporary);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patches_startup_before_modules_preserving_other_files_and_integrity() {
        let root = std::env::temp_dir().join(format!("macos-localization-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let original = root.join("app.asar");
        let output = root.join("patched.asar");
        let html = b"<html><head><meta http-equiv=\"Content-Security-Policy\" content=\"script-src 'self'\"><script type=\"module\" src=\"./assets/main.js\"></script></head><body>Codex</body></html>";
        let existing = b"original application module";
        let header = json!({"files":{"webview":{"files":{
            "index.html":file_entry(0, html),
            "assets":{"files":{"main.js":file_entry(html.len() as u64, existing)}}
        }}, "native":{"unpacked":true, "size":12}}});
        let header = serde_json::to_vec(&header).unwrap();
        let padded = (header.len() + 3) & !3;
        let mut bytes = vec![];
        for size in [4, padded + 8, padded + 4, header.len()] { bytes.extend_from_slice(&(size as u32).to_le_bytes()); }
        bytes.extend_from_slice(&header); bytes.resize(16 + padded, 0);
        bytes.extend_from_slice(html); bytes.extend_from_slice(existing);
        fs::write(&original, &bytes).unwrap();
        let hash = prepare_archive(&original, &output).unwrap().unwrap();
        assert_eq!(fs::read(&original).unwrap(), bytes);
        let mut archive = fs::File::open(&output).unwrap();
        let (header, start) = read_header(&mut archive).unwrap();
        let html = read_entry(&mut archive, &header, start, ENTRY).unwrap();
        let html = String::from_utf8(html).unwrap();
        assert!(html.find("Content-Security-Policy").unwrap() < html.find(TAG).unwrap());
        assert!(html.find(TAG).unwrap() < html.find("type=\"module\"").unwrap());
        assert_eq!(html.matches(TAG).count(), 1);
        assert_eq!(read_entry(&mut archive, &header, start, ASSET).unwrap(), SCRIPT);
        assert_eq!(read_entry(&mut archive, &header, start, "webview/assets/main.js").unwrap(), existing);
        assert_eq!(header["files"]["native"]["unpacked"], true);
        assert_eq!(entry(&header, ENTRY).unwrap()["integrity"]["hash"], format!("{:x}", Sha256::digest(html.as_bytes())));
        let info = root.join("Info.plist");
        let mut value = plist::Value::Dictionary(plist::Dictionary::new());
        let mut integrity = plist::Dictionary::new();
        let mut entry = plist::Dictionary::new();
        entry.insert("hash".into(), plist::Value::String("original".into()));
        integrity.insert("Resources/app.asar".into(), plist::Value::Dictionary(entry));
        value.as_dictionary_mut().unwrap().insert("ElectronAsarIntegrity".into(), plist::Value::Dictionary(integrity));
        value.to_file_xml(&info).unwrap();
        let updated = updated_plist(&info, &hash).unwrap();
        assert_eq!(updated.as_dictionary().unwrap()["ElectronAsarIntegrity"].as_dictionary().unwrap()["Resources/app.asar"].as_dictionary().unwrap()["hash"].as_string(), Some(hash.as_str()));
        let mut raw = fs::File::open(&output).unwrap();
        let mut prefix = [0;16]; raw.read_exact(&mut prefix).unwrap();
        let mut json = vec![0;u32::from_le_bytes(prefix[12..16].try_into().unwrap()) as usize];
        raw.read_exact(&mut json).unwrap();
        assert_eq!(hash, format!("{:x}", Sha256::digest(&json)));
        assert_eq!(prepare_archive(&output, &root.join("unchanged.asar")).unwrap(), None);
        assert!(!root.join("unchanged.asar").exists());
        for fail in [false, true] {
            let bundle = root.join(if fail { "Rollback.app" } else { "Success.app" });
            let contents = bundle.join("Contents");
            fs::create_dir_all(contents.join("Resources")).unwrap();
            let app_archive = contents.join("Resources/app.asar");
            let app_info = contents.join("Info.plist");
            fs::write(&app_archive, &bytes).unwrap();
            fs::copy(&info, &app_info).unwrap();
            let original_info = fs::read(&app_info).unwrap();
            let attempts = std::cell::Cell::new(0);
            let signer = |_: &Path| {
                attempts.set(attempts.get() + 1);
                if fail && attempts.get() == 1 { Err("fixture signing failure".into()) } else { Ok(()) }
            };
            let executable = contents.join("MacOS/ChatGPT");
            let home = root.join(if fail { "rollback-home" } else { "success-home" });
            let result = install_with_signer(&executable, &home, &signer, |_| Ok(()));
            assert!(home.join("backup").is_dir());
            assert!(!contents.join("Resources/app.usage-assistant.tmp").exists());
            if fail {
                assert!(result.unwrap_err().contains("fixture signing failure"));
                assert_eq!(attempts.get(), 2, "must restore the original resources and repair the signature");
                assert_eq!(fs::read(&app_archive).unwrap(), bytes);
                assert_eq!(fs::read(&app_info).unwrap(), original_info);
            } else {
                result.unwrap();
                assert_eq!(attempts.get(), 1);
                let verified = std::cell::Cell::new(false);
                install_with_signer(&executable, &home, |_| panic!("must not sign an unchanged app"), |_| { verified.set(true); Ok(()) }).unwrap();
                assert!(verified.get(), "even an unchanged app must have a valid signature");
            }
        }
        fs::remove_dir_all(root).unwrap();
    }
}
