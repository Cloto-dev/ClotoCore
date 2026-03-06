# ClotoCore Installer & Distribution Strategy

**Version:** 1.0.0
**Status:** Approved Design
**Date:** 2026-03-04

---

## 1. Overview

A comprehensive design document for installer construction, distribution, and auto-update
to enable casual users to easily install ClotoCore.

### 1.1 Goals

- **Zero prerequisite knowledge**: No Rust/Node.js/Python development environment needed
- **Download → double-click → launch**: Complete in 3 steps
- **Install level selection**: Minimal / Normal / Custom
- **Auto-update**: Detect and apply new versions on launch

### 1.2 Target Platforms

| Platform | Format | Auto-update | Priority |
|----------|--------|-------------|----------|
| Windows x64 | NSIS (.exe) via Tauri — Desktop Installer | Ed25519 signing | Phase 1 |
| macOS x64 | DMG (.dmg) via Tauri — Desktop Installer | Ed25519 signing | Phase 1 |
| macOS arm64 | DMG (.dmg) via Tauri — Desktop Installer | Ed25519 signing | Phase 1 |
| Linux x64 | DEB (.deb) + AppImage via Tauri — Desktop Installer | Ed25519 signing | Phase 1 |
| Linux arm64 | CLI binary (.tar.gz) | — | Phase 1 |

---

## 2. Architecture

### 2.1 Distribution Channels

```
Developer (tag push: v0.5.3)
  │
  ├── GitHub Actions (CI/CD)
  │     ├── cargo tauri build (Windows, macOS, Linux)
  │     ├── cargo build (CLI: all platforms)
  │     ├── Ed25519 signing (all Tauri installers)
  │     ├── latest.json generation (all desktop platforms)
  │     └── Upload to GitHub Releases
  │
  └── GitHub Releases (distribution point)
        ├── cloto-system_0.5.3_x64-setup.exe       (Windows NSIS)
        ├── cloto-system_0.5.3_x64-setup.nsis.zip  (for Tauri updater)
        ├── cloto-system_0.5.3_x64-setup.nsis.zip.sig (Ed25519 signature)
        ├── cloto-system_0.5.3_amd64.deb            (Linux DEB)
        ├── cloto-system_0.5.3_amd64.AppImage        (Linux AppImage)
        ├── cloto-system_0.5.3_aarch64.dmg           (macOS arm64 DMG)
        ├── cloto-system_0.5.3_x64.dmg               (macOS x64 DMG)
        ├── cloto-0.5.3-linux-x64.tar.gz             (CLI: Linux x64)
        ├── cloto-0.5.3-macos-arm64.tar.gz           (CLI: macOS arm64)
        ├── latest.json                              (auto-update: all desktop platforms)
        ├── SHA256SUMS.txt                           (checksums)
        └── SHA256SUMS.txt.sig                       (cosign signature)
```

### 2.2 User Flow

```
Casual user:
  1. GitHub Releases → download cloto-setup-x.y.z.exe
  2. Double-click → NSIS installer launches
  3. Select install level (Full / Core / Custom)
  4. Choose install location and options
  5. Installation complete → desktop app launches
  6. First launch: setup wizard (Phase 2)
  7. Subsequent: auto-update check on launch (Phase 3)
```

---

## 3. Phase 1: CI/CD Release Build

### 3.1 Current State and Issues

**Existing `release.yml`**:
- Multi-platform build of CLI binary (`cloto_system`)
- Windows GUI installer via Inno Setup
- Checksum signing via cosign

**Issues**:
- Tauri desktop app (`app.exe`) is not being built
- Inno Setup installer only bundles CLI binary
- MCP servers (Python) are not bundled
- No Ed25519 signing for auto-update

### 3.2 Adding Tauri Build

Add a Tauri build job to `release.yml`.

```yaml
build-tauri:
  name: Build Tauri Installer (Windows)
  needs: build-dashboard
  runs-on: windows-latest
  steps:
    - uses: actions/checkout@v4
    - uses: actions/setup-node@v4
      with:
        node-version: "20"
    - uses: dtolnay/rust-toolchain@stable
      with:
        targets: x86_64-pc-windows-msvc
    - name: Install frontend dependencies
      run: npm ci
      working-directory: dashboard
    - name: Build Tauri
      uses: tauri-apps/tauri-action@v0
      env:
        GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
        TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}
      with:
        projectPath: dashboard
        tauriScript: npx tauri
```

> **Note:** Tauri desktop app is built for Windows (NSIS), macOS (DMG), and Linux (DEB + AppImage). Linux arm64 remains CLI-only.

**What `tauri-apps/tauri-action` does**:
- Executes `cargo tauri build`
- Generates platform-specific installers (NSIS / DMG / AppImage)
- Automatically generates Ed25519 signatures if `TAURI_SIGNING_PRIVATE_KEY` is set
- Outputs `.sig` files as artifacts

### 3.3 Ed25519 Key Generation and Management

```bash
# Generate key pair
npx @tauri-apps/cli signer generate -w ~/.tauri/cloto.key
```

- **Public key**: Commit to `tauri.conf.json` → `plugins.updater.pubkey`
- **Private key**: GitHub Secrets `TAURI_SIGNING_PRIVATE_KEY`
- **Password**: GitHub Secrets `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`

### 3.4 Auto-Update Manifest (`latest.json`)

Include a manifest for the Tauri updater endpoint in the GitHub Release.

```json
{
  "version": "0.5.3",
  "notes": "ClotoCore v0.5.3",
  "pub_date": "2026-03-04T00:00:00Z",
  "platforms": {
    "windows-x86_64": {
      "signature": "<Ed25519 signature>",
      "url": "https://github.com/Cloto-dev/ClotoCore/releases/download/v0.5.3/cloto-system_0.5.3_x64-setup.nsis.zip"
    }
  }
}
```

Auto-generated by an inline shell script within the `release` job in `release.yml`
(reads the signature from the `.sig` file and outputs a Windows-only `latest.json`).

### 3.5 Relationship with Existing Inno Setup Installer

Tauri's NSIS installer **replaces** Inno Setup.

| Item | Inno Setup (current) | Tauri NSIS (new) |
|------|---------------------|------------------|
| Target binary | CLI (`cloto_system`) | Desktop app (`app.exe`) |
| Dashboard | Via browser | Built-in Tauri WebView |
| Auto-update | None | Ed25519 native |
| Component selection | Defined in ISS | NSIS customization or in-app |
| Multilingual | English / Japanese | NSIS multilingual support |

**Migration plan**:
- Phase 1: Generate desktop app installers with Tauri (Windows NSIS, macOS DMG, Linux DEB + AppImage)
- Existing Inno Setup (`installer/`) is **deprecated** (last used in v0.5.11). Files retained for reference
- Linux arm64 distributed as CLI binary only (no Tauri build)

---

## 4. Phase 1: NSIS Installer Customization

### 4.1 Tauri NSIS Customization

Controlled via the `bundle.nsis` section in `tauri.conf.json`:

```json
{
  "bundle": {
    "targets": "all",
    "nsis": {
      "displayLanguageSelector": true,
      "languages": ["English", "Japanese"],
      "installMode": "both",
      "headerImage": "icons/nsis-header.bmp",
      "sidebarImage": "icons/nsis-sidebar.bmp"
    }
  }
}
```

### 4.2 Bundled Components

Tauri NSIS bundles the desktop app itself. Additional components
(MCP servers, Python runtime) are downloaded and set up on-demand
via the **Phase 2 in-app setup wizard**.

**Rationale**:
- Python runtime (embedded) is ~30MB, all MCP servers are ~50MB+
- Keep installer size minimal (core app only ~15-20MB)
- MCP server combinations vary by user
- "Download only what you need" in-app is the optimal UX

### 4.3 Install Options

Tauri NSIS defaults:
- Install location selection (`{autopf}\ClotoCore`)
- Desktop shortcut creation
- Start menu registration
- Language selection (English / Japanese)

---

## 5. Phase 2: In-App Setup Wizard (Future)

### 5.1 Overview

A wizard displayed on first launch. Sets up components according to
the selected install level.

### 5.2 Install Levels

| Level | Contents | Target Users |
|-------|----------|-------------|
| **Minimal** | Core app only. No MCP servers | Trial / evaluation purposes |
| **Normal** | Core app + recommended MCP servers (terminal, deepseek, embedding) + automatic Python venv setup | General users |
| **Custom** | Select MCP servers individually | Advanced users |

### 5.3 Wizard Flow

```
Step 1: Welcome
  │  "Welcome to ClotoCore"
  │  Select install level: [Minimal] [Normal (recommended)] [Custom]
  │
Step 2: Components (only for Custom selection)
  │  Checklist:
  │  ☑ terminal (command execution)
  │  ☑ deepseek (inference engine)
  │  ☑ embedding (vector search)
  │  ☐ cerebras (fast inference)
  │  ☐ tts (text-to-speech)
  │  ☐ stt (speech-to-text)
  │  ☐ ...
  │
Step 3: API Keys (if applicable servers are selected)
  │  DeepSeek API Key: [________________]
  │  Cerebras API Key: [________________]
  │  (Can be skipped → configure later from Settings)
  │
Step 4: Setup
  │  Progress bar:
  │  [===========          ] Building Python venv...
  │  [==================   ] Initializing MCP servers...
  │  [=====================] Complete!
  │
Step 5: Complete
     "Setup is complete"
     [Open Dashboard]
```

### 5.4 Technical Considerations

- **Python runtime**: Uses Embedded Python (pystand / python-build-standalone)
  - Does not depend on user's Python installation
  - Downloads ~30MB embedded Python during first-time setup
- **venv setup**: Execute `python -m venv` + `pip install` via Tauri's shell plugin
- **Progress notifications**: Send progress events between Tauri and frontend
- **Configuration saving**: Reflect setup results in `mcp.toml` / `.env`
- **Skippable**: Wizard can be re-run later from Settings → Setup

---

## 6. Phase 3: Auto-Update (Future)

Enable Tauri v2's native auto-update feature so the desktop app automatically
detects, downloads, and applies new versions.

### 6.1 Comparison with Option B (Current)

| Feature | Option B (current) | Tauri Native (Phase 3) |
|---------|-------------------|----------------------|
| Update check | Manual button | Automatic check on launch |
| Update apply | CLI binary replacement | Tauri native (platform-specific) |
| Signature verification | SHA256 checksum | Ed25519 cryptographic signature |
| User experience | Manual restart | Seamless restart prompt |
| Platform support | CLI needs to be in PATH | Works with entire Tauri build |

### 6.2 `tauri.conf.json` Updater Configuration

```json
{
  "plugins": {
    "updater": {
      "pubkey": "<Ed25519 public key>",
      "endpoints": [
        "https://github.com/Cloto-dev/ClotoCore/releases/latest/download/latest.json"
      ],
      "dialog": true
    }
  }
}
```

### 6.3 Dashboard UI Integration

**Auto-check on launch** (App.tsx or root layout):

```typescript
import { check } from '@tauri-apps/plugin-updater';

useEffect(() => {
  if (!isTauri) return;
  check().then(update => {
    if (update?.available) {
      // Show notification or modal
    }
  }).catch(console.error);
}, []);
```

**Manual check in Settings → About**:

```typescript
import { check } from '@tauri-apps/plugin-updater';

const update = await check();
if (update?.available) {
  await update.downloadAndInstall();
  // Prompt user to restart
}
```

### 6.4 Migration Steps from Option B

1. Generate Ed25519 key pair (see Section 3.3)
2. Add public key to `tauri.conf.json`
3. Register private key in GitHub Secrets
4. Add signing + manifest generation to `release.yml`
5. Replace `checkForUpdates()` in `AboutSection.tsx` with Tauri native check
6. Remove CLI shell execution path (maintain fallback for non-Tauri environments)
7. Full cycle test: build → release → auto-update notification → apply

### 6.5 Prerequisites

- Phase 1 CI/CD generates Ed25519 signatures + `latest.json`
- Public key is configured in `tauri.conf.json`
- Private key is configured in GitHub Secrets

---

## 7. Versioning Strategy

> For versioning details, see `docs/DEVELOPMENT.md` Section 3.

**Release-specific notes**:
- `release.yml` automatically publishes versions containing `-alpha` / `-beta` / `-rc`
  with the prerelease flag (already implemented)
- Three locations (`Cargo.toml`, `dashboard/package.json`, `tauri.conf.json`) must
  be updated simultaneously. `release.yml` validates consistency with the tag

---

## 8. Security

### 8.1 Signing Chain

```
Developer
  │  git tag v0.5.3 && git push --tags
  │
GitHub Actions
  │  ├── Binary build
  │  ├── Ed25519 signing (for Tauri updater)
  │  ├── cosign signing (for checksums, keyless)
  │  └── SHA256 checksum generation
  │
GitHub Releases
  │  ├── .exe / .dmg / .AppImage (installers)
  │  ├── .sig (Ed25519 signatures)
  │  ├── SHA256SUMS.txt (checksums)
  │  └── SHA256SUMS.txt.sig (cosign signature)
  │
End User
     ├── Installer: OS verifies signature
     └── Auto-update: Tauri verifies Ed25519
```

### 8.2 Key Management

| Key | Storage Location | Purpose |
|-----|-----------------|---------|
| Ed25519 private key | GitHub Secrets | Signing for Tauri auto-update |
| Ed25519 public key | `tauri.conf.json` (repository) | Client-side signature verification |
| cosign | keyless (Sigstore) | Checksum signing |

---

## 9. Implementation Roadmap

### v0.5.x: UI/UX Optimization Phase (Current)

Improve dashboard usability and visibility, stabilizing the UI
before installer distribution.

- Persistent sidebar layout (AppLayout + AppSidebar)
- Unified modal system (Modal component)
- Card grid for MCP page
- Sidebar collapse feature
- Browser history navigation (back/forward)
- Dark theme adjustments
- Other UI improvements and bug fixes

### v0.6.0: Phase 1 — CI/CD + Tauri Installer Distribution

1. Generate Ed25519 key pair, register in GitHub Secrets
2. Add public key to `tauri.conf.json`, NSIS configuration (Windows-only targets)
3. `release.yml`: Replace `build-installer` (Inno Setup) with `build-tauri` (NSIS)
4. Generate `latest.json` inline within the `release` job (Windows-only)
5. Deprecate Inno Setup files (retain for reference)
6. Verify with test release (`v0.6.0-alpha.1`)

### v0.7.0: Phase 2 — In-App Setup Wizard

1. Design wizard UI components
2. Embedded Python download and extraction logic
3. MCP server selection and installation flow
4. API key configuration flow
5. Re-run capability from Settings

### v0.7.x: Phase 3 — Enable Auto-Update

1. Confirm `latest.json` auto-generation
2. Add update check UI to Dashboard
3. Test update download and apply flow
4. Migration from Option B (CLI update)

---

## 10. Related Files

| File | Description |
|------|-------------|
| `.github/workflows/release.yml` | Release build CI/CD (build-tauri + latest.json generation) |
| `dashboard/src-tauri/tauri.conf.json` | Tauri configuration (NSIS, updater, Ed25519 public key) |
| `installer/cloto-setup.iss` | Inno Setup configuration (**deprecated**: retained for reference, last used v0.5.11) |
| `installer/build-installer.ps1` | Inno Setup build script (**deprecated**: same as above) |

---

*Document created: 2026-03-04*
