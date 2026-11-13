#!/bin/sh
set -eu

root="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
arch="$(uname -m)"
case "$arch" in
  arm64) runtime="osx-arm64" ;;
  x86_64) runtime="osx-x64" ;;
  *) echo "Unsupported macOS architecture: $arch" >&2; exit 2 ;;
esac

runtime_dir="$root/src-tauri/bundled/v2rayn/$runtime"
test -x "$runtime_dir/api/v2rayN.WebAPI" || {
  echo "Build the v2rayN runtime first: $runtime" >&2
  exit 1
}
test -x "$runtime_dir/bin/xray/xray" || {
  echo "The packaged v2rayN Core executable is missing or not executable." >&2
  exit 1
}

export APPLE_SIGNING_IDENTITY="-"
npm run tauri -- build --bundles app
app_bundle="$root/src-tauri/target/release/bundle/macos/Codex 使用助手.app"
test -d "$app_bundle" || {
  echo "Tauri did not produce the expected .app bundle: $app_bundle" >&2
  exit 1
}

output="$root/dist-portable"
mkdir -p "$output"
zip_path="$output/Codex使用助手-macOS-$runtime.zip"
dmg_path="$output/Codex使用助手-macOS-$runtime.dmg"

ditto -c -k --sequesterRsrc --keepParent "$app_bundle" "$zip_path"
staging="$(mktemp -d "${TMPDIR:-/tmp}/codexinstaller-dmg.XXXXXX")"
trap 'rm -rf "$staging"' EXIT
ditto "$app_bundle" "$staging/$(basename "$app_bundle")"
ln -s /Applications "$staging/Applications"

# Disk image creation can temporarily report EBUSY on hosted macOS runners.
attempt=1
until hdiutil create -volname "Codex使用助手" -srcfolder "$staging" -fs HFS+ -ov -format UDZO "$dmg_path"; do
  if [ "$attempt" -ge 5 ]; then
    echo "Disk image creation failed after $attempt attempts." >&2
    exit 1
  fi
  sleep "$((attempt * 5))"
  attempt="$((attempt + 1))"
done

echo "Portable ZIP: $zip_path"
echo "Disk image: $dmg_path"
