# Codex 使用助手

Cross-platform JianGuo Codex installation assistant built with Tauri 2, Rust, and TypeScript.

## Development

Requirements: Node.js 20+, Rust stable, and the platform's native Tauri build prerequisites.
Building the bundled v2rayN manager additionally requires the .NET 10 SDK, PowerShell 7, and network access to the pinned v2rayN source/Core archives. These are build-machine requirements only; they are not required on computers running the portable app.

```powershell
npm install
npm run dev
npm run tauri dev
npm run build
npm run tauri build
```

## Portable builds

The v2rayN WebAPI and architecture-matched Core files are published self-contained and staged under `src-tauri/bundled/v2rayn`. The build script pins the v2rayN source and Core repository commits and includes the v2rayN GPL-3.0 license and matching source archive in the portable distribution.

```powershell
npm run portable:win
```

On macOS, build the matching runtime first, then package the `.app` as a DMG and ZIP:

```sh
npm run v2rayn:runtime:mac:arm64
npm run portable:mac
```

The macOS script writes `Codex使用助手-macOS-osx-arm64.dmg` and the matching ZIP to
`dist-portable`. Use `v2rayn:runtime:mac:x64` for Intel Macs. The local API binds to
loopback only, and its random management key is kept in the user's app data directory.

### GitHub macOS builds

The private repository's Actions tab has a manually triggered
`macOS Test Packages` workflow. It builds separate Apple Silicon and Intel
packages on matching macOS runners. Select `Both`, `Apple-Silicon`, or `Intel`
when starting the workflow. Download each job's DMG/ZIP from Artifacts;
artifacts expire after seven days. Running the workflow uses the repository's
GitHub Actions allowance.

These packages use ad-hoc signing, not an Apple Developer ID, and are not
notarized. Gatekeeper may block them on another Mac. Test on a trusted Mac before
distribution; do not disable Gatekeeper globally. A production release needs an
Apple Developer ID and notarization, with credentials stored only in GitHub
Actions secrets, never in source code or chat.

The macOS system-proxy toggle is not implemented yet. For this test build,
uncheck the proxy option before one-click installation/configuration; bundling
the native proxy runtime does not enable macOS system-proxy integration.
Actual installation, login, configuration, and Codex launch still require
acceptance testing on a physical Mac.

The macOS localization action backs up `app.asar` and `Info.plist` under
`CODEX_HOME/backup/macos-localization-*`, installs the Chinese startup hook in
the application's renderer resources, updates ASAR integrity metadata, and
applies and verifies a local ad-hoc signature. This lets Finder/Dock launches
load Chinese without the assistant running. It replaces the application's
original signing identity; application updates can replace the hook, in which
case run localization again. Signing failures restore the backed-up resources
and report the backup location. Validate signing and direct launch on a Mac
before distributing this change.

Only source code belongs in Git. Local builds, downloaded installers, runtime
binaries, user databases, subscription settings, and credentials are excluded.
The modified v2rayN source is retained with its upstream license so macOS builds
can reproduce the integrated local service.

## Backend configuration contract

The installer login endpoint is `POST /api/v1/installer/login`. It accepts either a card key or credentials for an installer-only account created in the JianGuo admin panel. The client defaults to card-key login. Regular JianGuo account usernames/passwords are not used for installer login.

Platform package URLs, checksums, and Codex++ silent arguments are maintained under the admin panel's separate “Codex 安装配置” entry. The client reads public package values from `GET /api/v1/site-config`:

- `codex_windows_x64_url`, `codex_macos_x64_url`, `codex_macos_arm64_url`
- `codex_windows_x64_sha256`, `codex_macos_x64_sha256`, `codex_macos_arm64_sha256`
- Codex++ uses the latest stable assets from `BigPizzaV3/CodexPlusPlus` when no matching bundled installer is available: Windows x64 setup EXE and macOS universal DMG. Codex++ download URLs and SHA-256 values in site config are ignored.
- `v2rayn_windows_x64_url`, `v2rayn_macos_x64_url`, `v2rayn_macos_arm64_url`
- matching `v2rayn_*_sha256` values
- `codexplusplus_silent_args`

The v2rayN subscription URL is private and is returned only in the successful installer login response. It must not be added to anonymous site configuration. Installer-only accounts are stored separately from regular JianGuo accounts.

## Current implementation boundary

- Password login uses the existing `/api/v1/account/login-password` SHA-256 contract.
- Site config is fetched over HTTPS in the native application.
- Custom Codex++ provider configuration follows the `relayProfiles` shape observed in the JianGuo Windows project and backs up an existing settings file before writing.
- Providers can use pure API direct mode or the assistant's local gateway mode. The UI calls these connection modes “纯 API 直连” and “本地网关”; protocol adaptation is an internal gateway capability, not the user-facing mode name. The local gateway listens only on `127.0.0.1:47822`, adapts basic Codex Responses requests to upstream Chat Completions requests, and converts text/tool-call responses back to Responses-compatible JSON/SSE. It does not expose an external listening address or persist request bodies.
- Package lookup checks the per-platform bundled resource path first, then downloads from the configured HTTPS URL. A configured SHA-256 is verified before launch; SHA-256 is optional.
- Windows Codex++ automatic installation defaults to the upstream NSIS `/S` flag when `codexplusplus_silent_args` is empty. Explicit backend arguments override this default; manual installation passes no silent flags. macOS DMG installation remains interactive.
- The in-app v2rayN page uses the upstream WebAPI for subscription, profile, latency-test, and Core lifecycle management. Starting the Core exposes its local SOCKS/HTTP listeners; this version does not change Windows/macOS system proxy settings.
- The v2rayN manager runtime is staged from a pinned source revision. Its GPL-3.0 license and source archive are included with portable packages.
- The proxy compliance notice appears whenever the user enters the proxy page. The desktop client is not a trusted traffic meter; the backend must enforce any published free allowance, daily billing, and monthly traffic cap.
