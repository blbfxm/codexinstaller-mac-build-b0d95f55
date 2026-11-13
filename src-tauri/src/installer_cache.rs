use sha2::{Digest, Sha256};
use std::{fs, io::Read, path::{Path, PathBuf}};

#[derive(serde::Serialize, serde::Deserialize)]
struct Receipt {
    url: String,
    bytes: u64,
    sha256: String,
}

pub(crate) fn receipt_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".completed.json");
    name.into()
}

pub fn save(path: &Path, url: &str, bytes: u64, digest: &Sha256) -> Result<(), String> {
    let receipt = Receipt {
        url: url.into(), bytes,
        sha256: format!("{:x}", digest.clone().finalize()),
    };
    fs::write(receipt_path(path), serde_json::to_vec(&receipt).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

pub fn load(path: &Path, url: &str) -> Option<(u64, Sha256)> {
    let receipt: Receipt = serde_json::from_slice(&fs::read(receipt_path(path)).ok()?).ok()?;
    let mut input = fs::File::open(path).ok()?;
    if receipt.url != url || receipt.bytes == 0 || input.metadata().ok()?.len() != receipt.bytes {
        return None;
    }
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = input.read(&mut buffer).ok()?;
        if count == 0 { break; }
        digest.update(&buffer[..count]);
    }
    if format!("{:x}", digest.clone().finalize()) != receipt.sha256 { return None; }
    Some((receipt.bytes, digest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_download_survives_retries_but_not_source_changes_or_corruption() {
        let path = std::env::temp_dir().join(format!("codex-cache-test-{}.msix", std::process::id()));
        let url = "https://example.com/ChatGPT-x64.msix";
        fs::write(&path, b"complete download").unwrap();
        let mut digest = Sha256::new();
        digest.update(b"complete download");
        assert!(load(&path, url).is_none(), "unmarked file is not a completed download");
        save(&path, url, 17, &digest).unwrap();
        assert_eq!(load(&path, url).unwrap().0, 17);
        assert_eq!(load(&path, url).unwrap().0, 17, "retries reuse the same completed file");
        assert!(load(&path, "https://example.com/new-release.msix").is_none());
        fs::write(&path, b"corrupt! download").unwrap();
        assert!(load(&path, url).is_none(), "same-length corruption must be rejected");
        fs::write(&path, b"partial").unwrap();
        assert!(load(&path, url).is_none());
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(receipt_path(&path));
    }
}
