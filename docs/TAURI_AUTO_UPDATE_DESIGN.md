# Tauri Native Auto-Update — Design Document

**Version:** 0.1.0-draft
**Status:** Draft (for future implementation)
**Date:** 2026-03-03
**Target:** v0.6.x+

---

## 1. Overview

Tauri v2 のネイティブ自動更新機能を有効化し、デスクトップアプリが自動的に
新バージョンを検知・ダウンロード・適用するシステム。

現在の Option B (GitHub API + CLI shell 実行) を置き換える。

### 1.1 Advantages over Option B

| Feature | Option B (current) | Option A (this design) |
|---------|-------------------|----------------------|
| Update check | Manual button press | Automatic on startup |
| Update apply | CLI binary swap | Tauri native (platform-specific) |
| Signature verification | SHA256 checksum | Ed25519 cryptographic signature |
| User experience | Restart required manually | Seamless restart prompt |
| Platform support | CLI must be in PATH | Works for all Tauri builds |

---

## 2. Architecture

```
GitHub Release
  ├── latest.json          ← Tauri updater manifest
  ├── cloto-setup-0.5.0.exe.sig  ← Ed25519 signature
  ├── cloto-setup-0.5.0.exe      ← Windows installer
  ├── ClotoCore_0.5.0_amd64.AppImage.sig
  ├── ClotoCore_0.5.0_amd64.AppImage
  ├── ClotoCore_0.5.0.dmg.sig
  └── ClotoCore_0.5.0.dmg

Tauri App Startup
  │
  ├── Fetch latest.json from endpoint
  ├── Compare version vs current
  ├── If update available:
  │     ├── Download platform binary
  │     ├── Verify Ed25519 signature
  │     └── Prompt user to restart
  └── Apply update on restart
```

---

## 3. Key Generation

### 3.1 Generate Ed25519 Keypair

```bash
# Generate keypair using Tauri CLI
npx @tauri-apps/cli signer generate -w ~/.tauri/cloto.key
```

This produces:
- **Private key**: `~/.tauri/cloto.key` (NEVER commit to repository)
- **Public key**: Displayed in stdout (add to `tauri.conf.json`)

### 3.2 Store Keys

- **Private key**: GitHub repository secret `TAURI_SIGNING_PRIVATE_KEY`
- **Private key password**: GitHub repository secret `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
- **Public key**: Committed in `tauri.conf.json` → `plugins.updater.pubkey`

---

## 4. Configuration

### 4.1 `tauri.conf.json`

```json
{
  "plugins": {
    "updater": {
      "pubkey": "dW50cnVzdGVkIGNvbW1lbnQ6...",
      "endpoints": [
        "https://github.com/Cloto-dev/ClotoCore/releases/latest/download/latest.json"
      ],
      "dialog": true
    }
  }
}
```

### 4.2 `latest.json` Format

```json
{
  "version": "0.5.0",
  "notes": "## What's New\n- Feature 1\n- Bug fix 2",
  "pub_date": "2026-03-15T12:00:00Z",
  "platforms": {
    "windows-x86_64": {
      "signature": "dW50cnVzdGVkIGNvbW1lbnQ6...",
      "url": "https://github.com/Cloto-dev/ClotoCore/releases/download/v0.5.0/cloto-setup-0.5.0.exe.zip"
    },
    "linux-x86_64": {
      "signature": "dW50cnVzdGVkIGNvbW1lbnQ6...",
      "url": "https://github.com/Cloto-dev/ClotoCore/releases/download/v0.5.0/ClotoCore_0.5.0_amd64.AppImage.tar.gz"
    },
    "darwin-x86_64": {
      "signature": "dW50cnVzdGVkIGNvbW1lbnQ6...",
      "url": "https://github.com/Cloto-dev/ClotoCore/releases/download/v0.5.0/ClotoCore.app.tar.gz"
    },
    "darwin-aarch64": {
      "signature": "dW50cnVzdGVkIGNvbW1lbnQ6...",
      "url": "https://github.com/Cloto-dev/ClotoCore/releases/download/v0.5.0/ClotoCore.app.tar.gz"
    }
  }
}
```

---

## 5. Release Workflow Changes

### 5.1 `release.yml` Additions

```yaml
# After building Tauri bundles:
- name: Sign Tauri bundles
  env:
    TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}
  run: |
    npx @tauri-apps/cli signer sign \
      target/release/bundle/nsis/cloto-setup-*.exe \
      -k "$TAURI_SIGNING_PRIVATE_KEY" \
      -p "$TAURI_SIGNING_PRIVATE_KEY_PASSWORD"

- name: Generate latest.json
  run: |
    python scripts/generate_update_manifest.py \
      --version ${{ github.ref_name }} \
      --notes "${{ steps.release_notes.outputs.notes }}" \
      --artifacts target/release/bundle/
```

### 5.2 `scripts/generate_update_manifest.py`

Script to generate `latest.json` from signed artifacts:
- Reads `.sig` files alongside each platform binary
- Constructs the Tauri-compatible manifest format
- Outputs `latest.json` for upload to GitHub Release

---

## 6. Dashboard UI Integration

### 6.1 Auto-Check on Startup

```typescript
import { check } from '@tauri-apps/plugin-updater';

// In App.tsx or root layout
useEffect(() => {
  if (!isTauri) return;
  check().then(update => {
    if (update?.available) {
      // Show notification or modal
    }
  }).catch(console.error);
}, []);
```

### 6.2 Settings → About

Replace the current Option B `checkForUpdates()` call with the Tauri native updater:

```typescript
import { check } from '@tauri-apps/plugin-updater';

const update = await check();
if (update?.available) {
  await update.downloadAndInstall();
  // Prompt user to restart
}
```

---

## 7. Migration from Option B

1. Generate Ed25519 keypair
2. Add pubkey to `tauri.conf.json`
3. Add signing secrets to GitHub
4. Update `release.yml` with signing + manifest generation
5. Replace `checkForUpdates()` in `AboutSection.tsx` with Tauri native check
6. Remove CLI shell execution path (keep as fallback for non-Tauri)
7. Test full cycle: build → release → auto-update notification → apply

---

## 8. Security Considerations

- Ed25519 signatures prevent MITM attacks on update payloads
- Private key stored only in GitHub Secrets (never in repository)
- `latest.json` served over HTTPS from GitHub Releases
- Tauri verifies signature before applying any update
- Rollback: Previous version preserved by platform installer

---

*Document created: 2026-03-03*
