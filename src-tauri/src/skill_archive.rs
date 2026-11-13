use std::{
    fs,
    io::{Read, Seek},
    path::{Component, Path},
};

const MANIFEST: &str = ".codex-plugin/plugin.json";
const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;

fn safe_relative(value: &str) -> bool {
    !value.is_empty()
        && !value.contains(['\\', ':', '\0'])
        && value.split('/').all(|part| {
            !part.is_empty() && part != "." && part != ".." && !part.ends_with(['.', ' '])
        })
        && Path::new(value)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

pub fn validate_request(name: &str, tree: &str, path: &str) -> Result<(), String> {
    if !safe_relative(name) || name.contains('/') || name.trim() != name {
        return Err("技能名称无效".into());
    }
    if tree.len() != 40 || !tree.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("技能版本信息无效，请刷新技能目录后重试".into());
    }
    if path != format!("plugins/{name}") || !safe_relative(path) {
        return Err("技能目录路径无效，请刷新技能目录后重试".into());
    }
    Ok(())
}

pub fn validate_codex_skill_request(name: &str, tree: &str, path: &str) -> Result<(), String> {
    if !safe_relative(name) || name.contains('/') || name.trim() != name {
        return Err("技能名称无效".into());
    }
    if tree.len() != 40 || !tree.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("技能版本信息无效，请刷新技能目录后重试".into());
    }
    if path != name || !safe_relative(path) {
        return Err("技能目录路径无效，请刷新技能目录后重试".into());
    }
    Ok(())
}

// Catalog tree IDs can identify a plugin subtree, rather than a full repository.
pub fn extract<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    catalog_path: &str,
    expected_name: &str,
    staging: &Path,
) -> Result<(), String> {
    extract_plugin(archive, catalog_path, expected_name, staging, false)
}

pub fn extract_market_plugin<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    catalog_path: &str,
    expected_name: &str,
    staging: &Path,
) -> Result<(), String> {
    extract_plugin(archive, catalog_path, expected_name, staging, true)
}

fn extract_plugin<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    catalog_path: &str,
    expected_name: &str,
    staging: &Path,
    alternate_manifests: bool,
) -> Result<(), String> {
    if archive.len() > 20_000 {
        return Err("技能包文件数量超出限制".into());
    }
    let names = archive.file_names().collect::<Vec<_>>();
    let wrapper = names
        .first()
        .and_then(|name| name.split_once('/'))
        .map(|(root, _)| root);
    let wrapper = wrapper.filter(|root| {
        safe_relative(root)
            && names
                .iter()
                .all(|name| name.starts_with(&format!("{root}/")))
    });
    let mut prefixes = vec![format!("{catalog_path}/"), String::new()];
    if let Some(root) = wrapper {
        prefixes.insert(0, format!("{root}/{catalog_path}/"));
        prefixes.insert(1, format!("{root}/"));
    }
    let manifests = if alternate_manifests {
        vec![MANIFEST, ".claude-plugin/plugin.json", "plugin.json"]
    } else {
        vec![MANIFEST]
    };
    let (prefix, manifest) = prefixes
        .into_iter()
        .find_map(|prefix| {
            manifests
                .iter()
                .find(|manifest| {
                    archive
                        .index_for_name(&format!("{prefix}{manifest}"))
                        .is_some()
                })
                .map(|manifest| (prefix, *manifest))
        })
        .ok_or("技能包缺少有效的插件清单，请刷新目录后重试")?;
    let manifest_name = format!("{prefix}{manifest}");
    {
        let file = archive
            .by_name(&manifest_name)
            .map_err(|_| "无法读取技能清单")?;
        if file.size() > 1024 * 1024 {
            return Err("技能清单过大".into());
        }
        let value: serde_json::Value =
            serde_json::from_reader(file).map_err(|_| "技能清单格式无效，请刷新目录后重试")?;
        if value.get("name").and_then(serde_json::Value::as_str) != Some(expected_name) {
            return Err("技能清单与所选技能不一致，请刷新目录后重试".into());
        }
    }
    let mut total = 0_u64;
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(|_| "无法读取技能包文件")?;
        let Some(relative) = file.name().strip_prefix(&prefix) else {
            continue;
        };
        let relative = relative.trim_end_matches('/');
        if relative.is_empty() {
            continue;
        }
        if !safe_relative(relative)
            || file
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err("技能包包含不安全的文件路径或符号链接".into());
        }
        total = total.checked_add(file.size()).ok_or("技能包大小超出限制")?;
        if file.size() > MAX_FILE_BYTES || total > MAX_TOTAL_BYTES {
            return Err("技能包解压大小超出限制".into());
        }
        let target = staging.join(relative);
        if file.is_dir() {
            fs::create_dir_all(&target).map_err(|error| format!("创建技能目录失败：{error}"))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("创建技能目录失败：{error}"))?;
        }
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|error| format!("创建技能文件失败：{error}"))?;
        let copied = std::io::copy(&mut (&mut file).take(MAX_FILE_BYTES + 1), &mut output)
            .map_err(|error| format!("解压技能文件失败：{error}"))?;
        if copied != file.size() || copied > MAX_FILE_BYTES {
            return Err("技能包文件大小异常".into());
        }
        #[cfg(unix)]
        if alternate_manifests {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                &target,
                fs::Permissions::from_mode(if file.unix_mode().unwrap_or(0) & 0o111 != 0 {
                    0o755
                } else {
                    0o644
                }),
            )
            .map_err(|_| "无法设置插件文件权限")?;
        }
    }
    if manifest != MANIFEST {
        fs::create_dir_all(staging.join(".codex-plugin"))
            .map_err(|_| "无法创建 Codex 插件清单目录")?;
        fs::copy(staging.join(manifest), staging.join(MANIFEST))
            .map_err(|_| "无法生成 Codex 插件清单")?;
    }
    Ok(())
}

pub fn install(bytes: Vec<u8>, path: &str, name: &str, destination: &Path) -> Result<(), String> {
    install_archive(bytes, path, name, destination, false)
}

pub fn install_codex_skill(
    bytes: Vec<u8>,
    path: &str,
    name: &str,
    destination: &Path,
) -> Result<(), String> {
    install_archive(bytes, path, name, destination, true)
}

fn install_archive(
    bytes: Vec<u8>,
    path: &str,
    name: &str,
    destination: &Path,
    is_codex_skill: bool,
) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|_| "技能包格式无效，请刷新目录后重试")?;
    let parent = destination.parent().ok_or("技能安装目录无效")?;
    fs::create_dir_all(parent).map_err(|error| format!("无法创建技能安装目录：{error}"))?;
    let mut random = [0_u8; 8];
    getrandom::fill(&mut random).map_err(|_| "无法创建技能临时目录")?;
    let suffix = u64::from_ne_bytes(random);
    let staging = parent.join(format!(".skill-{suffix:016x}.download"));
    let backup = parent.join(format!(".skill-{suffix:016x}.backup"));
    fs::create_dir(&staging).map_err(|error| format!("无法创建技能临时目录：{error}"))?;
    let result = (|| {
        if is_codex_skill {
            extract_codex_skill(&mut archive, path, &staging)?;
        } else {
            extract(&mut archive, path, name, &staging)?;
        }
        let existed = destination.exists();
        if existed {
            fs::rename(destination, &backup)
                .map_err(|error| format!("无法备份已有技能：{error}"))?;
        }
        if let Err(error) = fs::rename(&staging, destination) {
            if existed {
                fs::rename(&backup, destination).map_err(|restore| {
                    format!(
                        "安装技能失败：{error}；原文件保留在 {}，恢复失败：{restore}",
                        backup.display()
                    )
                })?;
            }
            return Err(format!("安装技能失败：{error}"));
        }
        if existed {
            let _ = fs::remove_dir_all(&backup);
        }
        Ok(())
    })();
    let _ = fs::remove_dir_all(&staging);
    result
}

fn extract_codex_skill<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    catalog_path: &str,
    staging: &Path,
) -> Result<(), String> {
    if archive.len() > 20_000 {
        return Err("技能包文件数量超出限制".into());
    }
    let names = archive.file_names().collect::<Vec<_>>();
    let wrapper = names
        .first()
        .and_then(|name| name.split_once('/'))
        .map(|(root, _)| root)
        .filter(|root| {
            safe_relative(root)
                && names
                    .iter()
                    .all(|name| name.starts_with(&format!("{root}/")))
        });
    let mut prefixes = vec![format!("{catalog_path}/"), String::new()];
    if let Some(root) = wrapper {
        prefixes.insert(0, format!("{root}/{catalog_path}/"));
        prefixes.insert(1, format!("{root}/"));
    }
    let prefix = prefixes
        .into_iter()
        .find(|prefix| {
            archive
                .index_for_name(&format!("{prefix}SKILL.md"))
                .is_some()
        })
        .ok_or("技能包缺少 SKILL.md，请刷新目录后重试")?;

    let mut total = 0_u64;
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(|_| "无法读取技能包文件")?;
        let Some(relative) = file.name().strip_prefix(&prefix) else {
            continue;
        };
        let relative = relative.trim_end_matches('/');
        if relative.is_empty() {
            continue;
        }
        if !safe_relative(relative)
            || file
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err("技能包包含不安全的文件路径或符号链接".into());
        }
        total = total.checked_add(file.size()).ok_or("技能包大小超出限制")?;
        if file.size() > MAX_FILE_BYTES || total > MAX_TOTAL_BYTES {
            return Err("技能包解压大小超出限制".into());
        }
        let target = staging.join(relative);
        if file.is_dir() {
            fs::create_dir_all(&target).map_err(|error| format!("创建技能目录失败：{error}"))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("创建技能目录失败：{error}"))?;
        }
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|error| format!("创建技能文件失败：{error}"))?;
        let copied = std::io::copy(&mut (&mut file).take(MAX_FILE_BYTES + 1), &mut output)
            .map_err(|error| format!("解压技能文件失败：{error}"))?;
        if copied != file.size() || copied > MAX_FILE_BYTES {
            return Err("技能包文件大小异常".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    #[test]
    fn public_market_normalizes_alternate_manifests_and_rejects_unsafe_archives() {
        for manifest in ["plugin.json", ".claude-plugin/plugin.json", MANIFEST] {
            for unsafe_path in [false, true] {
                let temp = std::env::temp_dir().join(format!(
                    "market-manifest-{}-{}-{unsafe_path}",
                    std::process::id(),
                    manifest.replace('/', "-")
                ));
                fs::create_dir_all(&temp).unwrap();
                let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
                let options = zip::write::SimpleFileOptions::default();
                zip.start_file(format!("repo/{manifest}"), options).unwrap();
                zip.write_all(br#"{"name":"demo","version":"1.0.0"}"#)
                    .unwrap();
                zip.start_file(
                    if unsafe_path {
                        "repo/../escape"
                    } else {
                        "repo/skills/demo/SKILL.md"
                    },
                    options,
                )
                .unwrap();
                zip.write_all(b"demo").unwrap();
                let bytes = zip.finish().unwrap().into_inner();
                let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
                let result = extract_market_plugin(&mut archive, "plugins/demo", "demo", &temp);
                if unsafe_path {
                    assert!(result.is_err());
                } else {
                    assert!(result.is_ok());
                    assert!(temp.join(MANIFEST).is_file());
                    assert!(temp.join("skills/demo/SKILL.md").is_file());
                }
                fs::remove_dir_all(temp).unwrap();
            }
        }
    }

    fn package(prefix: &str, manifest: &str, extra: Option<&str>) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        for (path, body) in [
            (format!("{prefix}{MANIFEST}"), manifest),
            (format!("{prefix}skills/demo/SKILL.md"), "# Demo"),
        ] {
            zip.start_file(path, options).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        if let Some(path) = extra {
            zip.start_file(format!("{prefix}{path}"), options).unwrap();
            zip.write_all(b"bad").unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn installs_standard_codex_skill_from_repository_archive() {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("repo/meeting-notes-and-actions/SKILL.md", options)
            .unwrap();
        zip.write_all(b"---\nname: meeting-notes-and-actions\n---\n")
            .unwrap();
        zip.start_file(
            "repo/meeting-notes-and-actions/references/template.md",
            options,
        )
        .unwrap();
        zip.write_all(b"# Meeting notes").unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        let temp = std::env::temp_dir().join(format!("codex-skill-test-{}", std::process::id()));
        let destination = temp.join("meeting-notes-and-actions");
        install_codex_skill(
            bytes,
            "meeting-notes-and-actions",
            "meeting-notes-and-actions",
            &destination,
        )
        .unwrap();
        assert!(destination.join("SKILL.md").is_file());
        assert!(destination.join("references/template.md").is_file());
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn installs_subtree_and_repository_archives_with_or_without_wrapper() {
        for prefix in ["repo-tree/", "repo-tree/plugins/demo/", "", "plugins/demo/"] {
            let temp = std::env::temp_dir().join(format!(
                "skill-test-{}-{}",
                std::process::id(),
                prefix.replace('/', "-")
            ));
            fs::create_dir_all(&temp).unwrap();
            let destination = temp.join("demo");
            install(
                package(prefix, r#"{"name":"demo"}"#, None),
                "plugins/demo",
                "demo",
                &destination,
            )
            .unwrap();
            assert!(destination.join(MANIFEST).is_file());
            assert_eq!(
                fs::read_to_string(destination.join("skills/demo/SKILL.md")).unwrap(),
                "# Demo"
            );
            // A repeat install replaces cleanly rather than leaving stale files.
            fs::write(destination.join("old.txt"), "old").unwrap();
            install(
                package(prefix, r#"{"name":"demo"}"#, None),
                "plugins/demo",
                "demo",
                &destination,
            )
            .unwrap();
            assert!(!destination.join("old.txt").exists());
            fs::remove_dir_all(temp).unwrap();
        }
    }

    #[test]
    fn invalid_packages_preserve_existing_installation() {
        let temp = std::env::temp_dir().join(format!("skill-invalid-{}", std::process::id()));
        let destination = temp.join("demo");
        fs::create_dir_all(&destination).unwrap();
        fs::write(destination.join("keep.txt"), "keep").unwrap();
        for bytes in [
            package("repo/", r#"{"name":"wrong"}"#, None),
            package("repo/", "not json", None),
            package("repo/", r#"{"name":"demo"}"#, Some("../escape.txt")),
            package("repo/", r#"{"name":"demo"}"#, Some("C:/escape.txt")),
            package("repo/", r#"{"name":"demo"}"#, Some("nested\\escape.txt")),
            package("repo/other/", r#"{"name":"demo"}"#, None),
        ] {
            assert!(install(bytes, "plugins/demo", "demo", &destination).is_err());
            assert_eq!(
                fs::read_to_string(destination.join("keep.txt")).unwrap(),
                "keep"
            );
        }
        assert_eq!(fs::read_dir(&temp).unwrap().count(), 1);
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn rejects_invalid_catalog_parameters() {
        let tree = "876838d9751a6131bc83b926c3a880f2264bfa60";
        assert!(validate_request("demo", tree, "plugins/demo").is_ok());
        for (name, revision, path) in [
            ("../demo", tree, "plugins/../demo"),
            ("demo", "main", "plugins/demo"),
            ("demo", tree, "plugins/other"),
        ] {
            assert!(validate_request(name, revision, path).is_err());
        }
    }

    #[tokio::test]
    #[ignore = "downloads an actual upstream tree archive"]
    async fn upstream_tree_archive_installs() {
        let tree = "876838d9751a6131bc83b926c3a880f2264bfa60";
        let bytes = reqwest::Client::new()
            .get(format!(
                "https://codeload.github.com/BigPizzaV3/CodexPlusPlusPluginCache/zip/{tree}"
            ))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .bytes()
            .await
            .unwrap();
        let temp = std::env::temp_dir().join(format!("skill-upstream-{}", std::process::id()));
        let destination = temp.join("superdesign");
        install(
            bytes.to_vec(),
            "plugins/superdesign",
            "superdesign",
            &destination,
        )
        .unwrap();
        assert!(destination.join("skills/superdesign/SKILL.md").is_file());
        fs::remove_dir_all(temp).unwrap();
    }
}
