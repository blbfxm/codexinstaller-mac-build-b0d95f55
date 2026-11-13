//! Native, transactional macOS system proxies. Save entire protocol dictionaries
//! so PAC, authentication, bypass rules, and unknown settings survive restoration.
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Clone, Serialize, Deserialize)]
struct Entry { id: String, before: Option<plist::Value>, after: Option<plist::Value> }
#[derive(Clone, Default, Serialize, Deserialize)]
struct Snapshot { entries: Vec<Entry> }

type Service = (String, bool, Option<plist::Value>);

fn plan(snapshot: &mut Snapshot, services: Vec<Service>, enable: Option<u16>, clear: bool) -> Vec<Entry> {
    let mut changes = vec![];
    for (id, active, config) in services {
        let previous = snapshot.entries.iter().find(|e| e.id == id && e.after == config).cloned();
        let after = if enable.is_some() || clear {
            if !active { continue; }
            Some(configured(&config, enable))
        } else {
            let Some(previous) = &previous else { continue };
            previous.before.clone()
        };
        if enable.is_some() || clear {
            snapshot.entries.retain(|e| e.id != id);
            snapshot.entries.push(Entry { id: id.clone(), before: previous.map_or_else(|| config.clone(), |e| e.before), after: after.clone() });
        }
        changes.push(Entry { id, before: config, after });
    }
    changes
}

fn configured(original: &Option<plist::Value>, port: Option<u16>) -> plist::Value {
    let mut dict = original.as_ref().and_then(plist::Value::as_dictionary).cloned().unwrap_or_default();
    for key in ["HTTPEnable", "HTTPSEnable", "SOCKSEnable", "ProxyAutoConfigEnable", "ProxyAutoDiscoveryEnable"] {
        dict.insert(key.into(), plist::Value::Integer(0.into()));
    }
    if let Some(port) = port {
        for prefix in ["HTTP", "HTTPS"] {
            dict.insert(format!("{prefix}Enable"), plist::Value::Integer(1.into()));
            dict.insert(format!("{prefix}Proxy"), "127.0.0.1".into());
            dict.insert(format!("{prefix}Port"), plist::Value::Integer(port.into()));
        }
        if !dict.contains_key("ExceptionsList") { dict.insert("ExceptionsList".into(), plist::Value::Array(vec![])); }
        if let Some(items) = dict.get_mut("ExceptionsList").and_then(plist::Value::as_array_mut) {
            for name in ["localhost", "127.0.0.1", "::1", "*.local"] {
                if !items.iter().any(|v| v.as_string() == Some(name)) { items.push(name.into()); }
            }
        }
    }
    plist::Value::Dictionary(dict)
}

fn enabled(config: &Option<plist::Value>) -> bool {
    config.as_ref().and_then(plist::Value::as_dictionary).is_some_and(|dict|
        ["HTTPEnable", "HTTPSEnable", "SOCKSEnable", "ProxyAutoConfigEnable", "ProxyAutoDiscoveryEnable"]
            .iter().any(|key| dict.get(*key).and_then(plist::Value::as_unsigned_integer) == Some(1)))
}

fn read_snapshot(path: &Path) -> Result<Snapshot, String> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| "原系统代理备份无效，未改动网络设置".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Snapshot::default()),
        Err(e) => Err(format!("读取系统代理备份失败：{e}")),
    }
}

fn save(path: &Path, snapshot: &Snapshot) -> Result<(), String> {
    use std::io::Write;
    let temp = path.with_extension("tmp");
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
    let mut file = options.open(&temp).map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec(snapshot).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    fs::rename(temp, path).map_err(|e| format!("保存系统代理备份失败：{e}"))
}

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use std::{ffi::{c_char, c_void}, ptr};
    type Ref = *const c_void;
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(value: Ref);
        fn CFStringCreateWithBytes(a: Ref, bytes: *const u8, len: isize, encoding: u32, external: u8) -> Ref;
        fn CFStringGetLength(s: Ref) -> isize;
        fn CFStringGetMaximumSizeForEncoding(len: isize, encoding: u32) -> isize;
        fn CFStringGetCString(s: Ref, bytes: *mut c_char, size: isize, encoding: u32) -> u8;
        fn CFArrayGetCount(a: Ref) -> isize;
        fn CFArrayGetValueAtIndex(a: Ref, index: isize) -> Ref;
        fn CFPropertyListCreateData(a: Ref, value: Ref, format: isize, options: usize, error: *mut Ref) -> Ref;
        fn CFDataGetLength(data: Ref) -> isize;
        fn CFDataGetBytePtr(data: Ref) -> *const u8;
        fn CFDataCreate(a: Ref, bytes: *const u8, len: isize) -> Ref;
        fn CFPropertyListCreateWithData(a: Ref, data: Ref, options: usize, format: *mut isize, error: *mut Ref) -> Ref;
    }
    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        fn AuthorizationCreate(rights: Ref, environment: Ref, flags: u32, auth: *mut Ref) -> i32;
        fn AuthorizationFree(auth: Ref, flags: u32) -> i32;
    }
    #[link(name = "SystemConfiguration", kind = "framework")]
    unsafe extern "C" {
        fn SCPreferencesCreate(a: Ref, name: Ref, id: Ref) -> Ref;
        fn SCPreferencesCreateWithAuthorization(a: Ref, name: Ref, id: Ref, auth: Ref) -> Ref;
        fn SCPreferencesLock(p: Ref, wait: u8) -> u8;
        fn SCPreferencesUnlock(p: Ref) -> u8;
        fn SCPreferencesCommitChanges(p: Ref) -> u8;
        fn SCPreferencesApplyChanges(p: Ref) -> u8;
        fn SCNetworkServiceCopyAll(p: Ref) -> Ref;
        fn SCNetworkServiceGetServiceID(s: Ref) -> Ref;
        fn SCNetworkServiceGetEnabled(s: Ref) -> u8;
        fn SCNetworkServiceCopyProtocol(s: Ref, kind: Ref) -> Ref;
        fn SCNetworkProtocolGetConfiguration(p: Ref) -> Ref;
        fn SCNetworkProtocolSetConfiguration(p: Ref, config: Ref) -> u8;
        fn SCError() -> i32;
    }
    struct Owned(Ref);
    impl Drop for Owned { fn drop(&mut self) { unsafe { if !self.0.is_null() { CFRelease(self.0); } } } }
    fn owned(value: Ref) -> Result<Owned, String> {
        if value.is_null() { Err("无法读取 macOS 网络服务配置".into()) } else { Ok(Owned(value)) }
    }
    fn string(text: &str) -> Result<Owned, String> {
        owned(unsafe { CFStringCreateWithBytes(ptr::null(), text.as_ptr(), text.len() as isize, 0x08000100, 0) })
    }
    fn text(value: Ref) -> Result<String, String> {
        if value.is_null() { return Err("macOS 网络服务缺少标识".into()); }
        unsafe {
            let len = CFStringGetMaximumSizeForEncoding(CFStringGetLength(value), 0x08000100) + 1;
            let mut bytes = vec![0u8; len as usize];
            if CFStringGetCString(value, bytes.as_mut_ptr().cast(), len, 0x08000100) == 0 { return Err("读取网络服务标识失败".into()); }
            Ok(String::from_utf8_lossy(bytes.split(|b| *b == 0).next().unwrap_or_default()).into())
        }
    }
    fn decode(value: Ref) -> Result<Option<plist::Value>, String> {
        if value.is_null() { return Ok(None); }
        let data = owned(unsafe { CFPropertyListCreateData(ptr::null(), value, 100, 0, ptr::null_mut()) })?;
        let bytes = unsafe { std::slice::from_raw_parts(CFDataGetBytePtr(data.0), CFDataGetLength(data.0) as usize) };
        plist::Value::from_reader(std::io::Cursor::new(bytes)).map(Some).map_err(|e| e.to_string())
    }
    fn encode(value: &plist::Value) -> Result<Owned, String> {
        let mut bytes = vec![];
        value.to_writer_binary(&mut bytes).map_err(|e| e.to_string())?;
        let data = owned(unsafe { CFDataCreate(ptr::null(), bytes.as_ptr(), bytes.len() as isize) })?;
        owned(unsafe { CFPropertyListCreateWithData(ptr::null(), data.0, 0, ptr::null_mut(), ptr::null_mut()) })
    }
    pub struct Session { prefs: Owned, auth: Ref, locked: bool }
    impl Drop for Session {
        fn drop(&mut self) { unsafe {
            if self.locked { SCPreferencesUnlock(self.prefs.0); }
            if !self.auth.is_null() { AuthorizationFree(self.auth, 0); }
        } }
    }
    impl Session {
        pub fn new(write: bool) -> Result<Self, String> {
            let name = string("Codex 使用助手")?;
            let mut auth = ptr::null();
            let prefs = unsafe {
                if write {
                    if AuthorizationCreate(ptr::null(), ptr::null(), 0, &mut auth) != 0 { return Err("无法申请系统网络设置授权".into()); }
                    SCPreferencesCreateWithAuthorization(ptr::null(), name.0, ptr::null(), auth)
                } else { SCPreferencesCreate(ptr::null(), name.0, ptr::null()) }
            };
            if prefs.is_null() { unsafe { if !auth.is_null() { AuthorizationFree(auth, 0); } } return Err("无法打开 macOS 网络配置".into()); }
            let mut session = Self { prefs: Owned(prefs), auth, locked: false };
            if write {
                if unsafe { SCPreferencesLock(prefs, 0) } == 0 { return Err("无法锁定系统网络配置，请完成 macOS 授权后重试".into()); }
                session.locked = true;
            }
            Ok(session)
        }
        pub fn services(&self) -> Result<Vec<Service>, String> {
            let all = owned(unsafe { SCNetworkServiceCopyAll(self.prefs.0) })?;
            let kind = string("Proxies")?;
            let mut result = vec![];
            for i in 0..unsafe { CFArrayGetCount(all.0) } {
                let service = unsafe { CFArrayGetValueAtIndex(all.0, i) };
                let protocol = unsafe { SCNetworkServiceCopyProtocol(service, kind.0) };
                if protocol.is_null() { continue; }
                let protocol = Owned(protocol);
                result.push((text(unsafe { SCNetworkServiceGetServiceID(service) })?,
                    unsafe { SCNetworkServiceGetEnabled(service) } != 0,
                    decode(unsafe { SCNetworkProtocolGetConfiguration(protocol.0) })?));
            }
            Ok(result)
        }
        pub fn write(&self, entries: &[Entry], after: bool) -> Result<(), String> {
            let all = owned(unsafe { SCNetworkServiceCopyAll(self.prefs.0) })?;
            let kind = string("Proxies")?;
            for i in 0..unsafe { CFArrayGetCount(all.0) } {
                let service = unsafe { CFArrayGetValueAtIndex(all.0, i) };
                let id = text(unsafe { SCNetworkServiceGetServiceID(service) })?;
                let Some(entry) = entries.iter().find(|e| e.id == id) else { continue };
                let protocol = owned(unsafe { SCNetworkServiceCopyProtocol(service, kind.0) })?;
                let value = if after { &entry.after } else { &entry.before };
                let config = value.as_ref().map(encode).transpose()?;
                if unsafe { SCNetworkProtocolSetConfiguration(protocol.0, config.as_ref().map_or(ptr::null(), |v| v.0)) } == 0 {
                    return Err("保存 macOS 代理配置失败".into());
                }
            }
            if unsafe { SCPreferencesCommitChanges(self.prefs.0) } == 0 || unsafe { SCPreferencesApplyChanges(self.prefs.0) } == 0 {
                return Err(format!("应用系统代理失败，请允许 macOS 修改网络设置（错误 {}）", unsafe { SCError() }));
            }
            Ok(())
        }
    }
}

#[cfg(target_os = "macos")]
static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(target_os = "macos")]
pub fn state(path: &Path) -> Result<serde_json::Value, String> {
    let _guard = LOCK.lock().map_err(|_| "系统代理正在切换")?;
    state_unlocked(path)
}
#[cfg(target_os = "macos")]
fn state_unlocked(path: &Path) -> Result<serde_json::Value, String> {
    let services = native::Session::new(false)?.services()?;
    let snapshot = read_snapshot(path)?;
    let active = services.iter().filter(|(_, on, _)| *on).collect::<Vec<_>>();
    let owned = active.iter().any(|(id, _, config)| snapshot.entries.iter().any(|e| &e.id == id && &e.after == config));
    let enabled = active.iter().any(|(id, _, config)| super::macos_proxy::enabled(config)
        && snapshot.entries.iter().any(|e| &e.id == id && &e.after == config));
    let server = active.iter().find_map(|(_, _, config)| {
        let dict = config.as_ref()?.as_dictionary()?;
        (dict.get("HTTPEnable")?.as_unsigned_integer()? == 1).then(||
            format!("{}:{}", dict.get("HTTPProxy").and_then(plist::Value::as_string).unwrap_or_default(),
                dict.get("HTTPPort").and_then(plist::Value::as_unsigned_integer).unwrap_or(0)))
    }).unwrap_or_default();
    Ok(serde_json::json!({"supported":true,"enabled":enabled,"assistantManaged":owned,
        "systemEnabled":active.iter().any(|(_,_,c)| super::macos_proxy::enabled(c)),"server":server}))
}

#[cfg(target_os = "macos")]
pub fn change(path: &Path, enable: Option<u16>, clear: bool) -> Result<serde_json::Value, String> {
    let _guard = LOCK.lock().map_err(|_| "系统代理正在切换")?;
    let mut snapshot = read_snapshot(path)?;
    let previous_snapshot = snapshot.clone();
    if enable.is_none() && !clear && snapshot.entries.is_empty() { return state_unlocked(path); }
    let session = native::Session::new(true)?;
    let changes = plan(&mut snapshot, session.services()?, enable, clear);
    if enable.is_some() && changes.is_empty() { return Err("未找到可设置代理的已启用网络服务，请先连接网络".into()); }
    if enable.is_some() || clear { save(path, &snapshot)?; }
    if !changes.is_empty() {
        if let Err(error) = session.write(&changes, true) {
            let rollback = session.write(&changes, false);
            return Err(if rollback.is_ok() {
                save(path, &previous_snapshot)?;
                error
            } else { format!("{error}；恢复未完成，请使用清理系统代理") });
        }
    }
    drop(session);
    if enable.is_none() && !clear && path.exists() { fs::remove_file(path).map_err(|e| e.to_string())?; }
    state_unlocked(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_auth_bypass_and_pac_for_exact_restoration() {
        let mut dict = plist::Dictionary::new();
        dict.insert("HTTPEnable".into(), plist::Value::Integer(1.into()));
        dict.insert("HTTPProxy".into(), "previous.example".into());
        dict.insert("HTTPUser".into(), "test-user".into());
        dict.insert("ProxyAutoConfigEnable".into(), plist::Value::Integer(1.into()));
        dict.insert("ProxyAutoConfigURLString".into(), "https://example.org/proxy.pac".into());
        dict.insert("ExceptionsList".into(), plist::Value::Array(vec!["*.example.org".into()]));
        let before = Some(plist::Value::Dictionary(dict));
        let after = configured(&before, Some(10809));
        let settings = after.as_dictionary().unwrap();
        assert_eq!(settings["HTTPProxy"].as_string(), Some("127.0.0.1"));
        assert_eq!(settings["HTTPSPort"].as_unsigned_integer(), Some(10809));
        assert_eq!(settings["HTTPUser"].as_string(), Some("test-user"));
        assert_eq!(settings["ProxyAutoConfigEnable"].as_unsigned_integer(), Some(0));
        assert_eq!(settings["ExceptionsList"].as_array().unwrap().len(), 5);
        let snapshot = Snapshot { entries: vec![Entry { id: "Wi-Fi 中文".into(), before: before.clone(), after: Some(after) }] };
        let copy: Snapshot = serde_json::from_slice(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
        assert_eq!(copy.entries[0].before, before);
        assert!(!enabled(&Some(configured(&before, None))));
    }
    #[test]
    fn repeated_enable_and_clear_restore_the_original_and_respect_other_apps() {
        let mut snapshot = Snapshot::default();
        let initial = vec![("wifi".into(), true, None), ("disabled".into(), false, None)];
        let first = plan(&mut snapshot, initial, Some(10809), false);
        assert_eq!(first.len(), 1);
        let second = plan(&mut snapshot, vec![("wifi".into(), true, first[0].after.clone())], Some(10810), false);
        assert!(snapshot.entries[0].before.is_none());
        let clear = plan(&mut snapshot, vec![("wifi".into(), true, second[0].after.clone())], None, true);
        assert!(!enabled(&clear[0].after));
        let restore = plan(&mut snapshot, vec![("wifi".into(), false, clear[0].after.clone())], None, false);
        assert!(restore[0].after.is_none(), "restore even a now-disabled network service");
        let external = Some(configured(&None, Some(7890)));
        assert!(plan(&mut snapshot, vec![("wifi".into(), true, external)], None, false).is_empty());
    }
    #[test]
    fn snapshot_is_durable_and_corruption_does_not_trigger_network_changes() {
        let dir = std::env::temp_dir().join(format!("mac-proxy-snapshot-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("proxy.json");
        assert!(read_snapshot(&path).unwrap().entries.is_empty());
        let snapshot = Snapshot { entries: vec![Entry { id: "Wi-Fi 中文".into(), before: None, after: Some(configured(&None, Some(10809))) }] };
        save(&path, &snapshot).unwrap();
        assert_eq!(read_snapshot(&path).unwrap().entries[0].after, snapshot.entries[0].after);
        fs::write(&path, b"broken").unwrap();
        assert!(read_snapshot(&path).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn reads_native_network_services_without_modifying_them() {
        let session = native::Session::new(false).unwrap();
        let services = session.services().unwrap();
        assert!(!services.is_empty(), "macOS runner must expose at least one network service");
        assert!(services.iter().all(|(id, _, _)| !id.is_empty()));
    }
}
