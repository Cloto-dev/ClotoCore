/**
 * Tauri-specific utilities. These functions are no-ops in browser mode.
 */

export const isTauri = '__TAURI_INTERNALS__' in window;

/**
 * Open a native file dialog to select a file.
 * Returns the selected file path or null if cancelled.
 */
export async function openFileDialog(options?: {
  title?: string;
  defaultPath?: string;
  filters?: Array<{ name: string; extensions: string[] }>;
}): Promise<string | null> {
  if (!isTauri) return null;
  const { open } = await import('@tauri-apps/plugin-dialog');
  const result = await open({
    title: options?.title,
    defaultPath: options?.defaultPath,
    filters: options?.filters,
    multiple: false,
    directory: false,
  });
  // open() returns string | string[] | null
  if (Array.isArray(result)) return result[0] ?? null;
  return result;
}

// ── Window Controls ──

export async function minimizeWindow() {
  if (!isTauri) return;
  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  await getCurrentWindow().minimize();
}

export async function toggleMaximizeWindow() {
  if (!isTauri) return;
  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  await getCurrentWindow().toggleMaximize();
}

export async function closeWindow() {
  if (!isTauri) return;
  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  await getCurrentWindow().close();
}

// ── VRM Window ──

// Track VRM window state at module level to avoid getByLabel race conditions
let vrmWindowOpen = false;
let vrmWindowRef: any = null;

/** Toggle VRM avatar window. Tauri: native window. Browser: popup. */
export async function openVrmWindow(agentId: string, apiKey?: string): Promise<void> {
  const keyParam = apiKey ? `?key=${encodeURIComponent(apiKey)}` : '';
  const path = `/vrm-viewer/${encodeURIComponent(agentId)}${keyParam}`;

  if (isTauri) {
    const { WebviewWindow } = await import('@tauri-apps/api/webviewWindow');
    const label = 'vrm-viewer';

    // Toggle off: close existing window
    if (vrmWindowOpen && vrmWindowRef) {
      try {
        await vrmWindowRef.destroy();
      } catch {
        /* already gone */
      }
      vrmWindowOpen = false;
      vrmWindowRef = null;
      return;
    }

    // Safety: also check via Tauri API in case state is stale
    try {
      const existing = await WebviewWindow.getByLabel(label);
      if (existing) {
        await existing.destroy();
        vrmWindowOpen = false;
        vrmWindowRef = null;
        return;
      }
    } catch {
      /* fine */
    }

    // Toggle on: create new window
    try {
      const win = new WebviewWindow(label, {
        url: path,
        title: 'VRM Avatar',
        width: 400,
        height: 600,
        minWidth: 200,
        minHeight: 300,
        resizable: true,
        decorations: false,
        transparent: true,
        shadow: false,
        alwaysOnTop: true,
      });

      vrmWindowRef = win;
      vrmWindowOpen = true;

      // Clean up tracking when window is destroyed (e.g. user closes via OS)
      win.once('tauri://destroyed', () => {
        vrmWindowOpen = false;
        vrmWindowRef = null;
      });
      win.once('tauri://error', () => {
        vrmWindowOpen = false;
        vrmWindowRef = null;
      });
    } catch (e) {
      if (import.meta.env.DEV) console.error('[VRM] Failed to create window:', e);
      vrmWindowOpen = false;
      vrmWindowRef = null;
    }
  } else {
    // Browser fallback: popup window
    const url = `${window.location.origin}${path}`;
    window.open(url, 'vrm-viewer', 'width=400,height=600,resizable=yes');
  }
}

// ── File Helpers ──

/** Read a text file via the Tauri backend. Returns null in browser mode. */
export async function readTextFile(path: string): Promise<string | null> {
  if (!isTauri) return null;
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    return await invoke<string>('read_text_file', { path });
  } catch {
    return null;
  }
}

/** Get the languages directory (Documents/ClotoCore/languages), creating it if needed. */
export async function getLanguagesDir(): Promise<string | null> {
  if (!isTauri) return null;
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    return await invoke<string>('get_languages_dir');
  } catch {
    return null;
  }
}

// ── Language Pack Management ──

/** Scan Documents/ClotoCore/languages/ for all .json packs. Returns (filename, content) pairs. */
export async function scanLanguagesDir(): Promise<Array<[string, string]>> {
  if (!isTauri) return [];
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    return await invoke<Array<[string, string]>>('scan_languages_dir');
  } catch {
    return [];
  }
}

/** Save a language pack JSON file to the languages directory. */
export async function saveLanguagePack(filename: string, content: string): Promise<boolean> {
  if (!isTauri) return false;
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('save_language_pack', { filename, content });
    return true;
  } catch {
    return false;
  }
}

/** Remove a language pack file from the languages directory. */
export async function removeLanguagePack(filename: string): Promise<boolean> {
  if (!isTauri) return false;
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('remove_language_pack', { filename });
    return true;
  } catch {
    return false;
  }
}

/** Install bundled default language packs if they don't exist yet. */
export async function installDefaultPacks(): Promise<number> {
  if (!isTauri) return 0;
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    return await invoke<number>('install_default_packs');
  } catch {
    return 0;
  }
}

// ── Auto API Key ──

/** Fetch the auto-generated API key from the Tauri backend. Returns null in browser mode. */
export async function getAutoApiKey(): Promise<string | null> {
  if (!isTauri) return null;
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    return (await invoke<string | null>('get_auto_api_key')) ?? null;
  } catch {
    return null;
  }
}

// ── Update Checker ──

export interface UpdateInfo {
  available: boolean;
  currentVersion: string;
  latestVersion: string;
  releaseDate?: string;
  releaseNotes?: string;
}

/**
 * Update channels served by the signed `updater-feed` rolling release
 * (docs/RELEASE_PIPELINE_DESIGN.md §5.1). Channel names come from the
 * Release Lifecycle Standard tiers verbatim.
 */
export type UpdateChannel = 'stable' | 'current' | 'experimental';
export const UPDATE_CHANNELS: readonly UpdateChannel[] = ['stable', 'current', 'experimental'];
export const UPDATE_CHANNEL_STORAGE_KEY = 'cloto-update-channel';

/** Resolve the persisted update channel, defaulting to `stable` on any unknown value. */
export function getUpdateChannel(): UpdateChannel {
  const stored = localStorage.getItem(UPDATE_CHANNEL_STORAGE_KEY);
  return UPDATE_CHANNELS.includes(stored as UpdateChannel) ? (stored as UpdateChannel) : 'stable';
}

/**
 * True when this build is a pre-release (semver pre-release suffix on the app
 * version). Locally derived — no network, no manifest (design §6).
 */
export const isExperimentalBuild = __APP_VERSION__.includes('-');

/**
 * Check for updates on the persisted channel. The Rust side builds the updater
 * against `…/updater-feed/{channel}.json` at check time; artifact signatures
 * are still verified against the pubkey in `tauri.conf.json`. Browser mode is
 * a no-op (returns `available: false`).
 */
export async function checkForUpdates(): Promise<UpdateInfo> {
  const current = __APP_VERSION__;
  if (!isTauri) {
    return { available: false, currentVersion: current, latestVersion: current };
  }
  const { invoke } = await import('@tauri-apps/api/core');
  return await invoke<UpdateInfo>('updater_check', { channel: getUpdateChannel() });
}

/**
 * Download and install the update available on the persisted channel, then
 * relaunch the app. Throws when not in desktop mode or no update is available.
 *
 * On Windows (NSIS), the installer terminates the running process and starts
 * the new exe itself, so the `relaunch()` call may reject — that is expected
 * and is swallowed. On macOS / Linux (.AppImage), `relaunch()` performs the
 * restart explicitly.
 */
export async function applyUpdate(): Promise<string> {
  if (!isTauri) throw new Error('Update can only be applied in desktop mode');
  const { invoke } = await import('@tauri-apps/api/core');
  const { relaunch } = await import('@tauri-apps/plugin-process');
  const message = await invoke<string>('updater_download_and_install', {
    channel: getUpdateChannel(),
  });
  try {
    await relaunch();
  } catch {
    // NSIS installer may have already killed the process
  }
  return message;
}
