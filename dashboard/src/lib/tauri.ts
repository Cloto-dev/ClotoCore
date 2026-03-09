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
      try { await vrmWindowRef.destroy(); } catch { /* already gone */ }
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
    } catch { /* fine */ }

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
      console.error('[VRM] Failed to create window:', e);
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
  } catch { return []; }
}

/** Save a language pack JSON file to the languages directory. */
export async function saveLanguagePack(filename: string, content: string): Promise<boolean> {
  if (!isTauri) return false;
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('save_language_pack', { filename, content });
    return true;
  } catch { return false; }
}

/** Remove a language pack file from the languages directory. */
export async function removeLanguagePack(filename: string): Promise<boolean> {
  if (!isTauri) return false;
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('remove_language_pack', { filename });
    return true;
  } catch { return false; }
}

/** Install bundled default language packs if they don't exist yet. */
export async function installDefaultPacks(): Promise<number> {
  if (!isTauri) return 0;
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    return await invoke<number>('install_default_packs');
  } catch { return 0; }
}

// ── Auto API Key ──

/** Fetch the auto-generated API key from the Tauri backend. Returns null in browser mode. */
export async function getAutoApiKey(): Promise<string | null> {
  if (!isTauri) return null;
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    return await invoke<string | null>('get_auto_api_key') ?? null;
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

function isNewerVersion(current: string, latest: string): boolean {
  const c = current.split('.').map(Number);
  const l = latest.split('.').map(Number);
  for (let i = 0; i < 3; i++) {
    if ((l[i] || 0) !== (c[i] || 0)) return (l[i] || 0) > (c[i] || 0);
  }
  return false;
}

export async function checkForUpdates(): Promise<UpdateInfo> {
  const current = __APP_VERSION__;
  const resp = await fetch(
    'https://api.github.com/repos/Cloto-dev/ClotoCore/releases/latest',
    { headers: { Accept: 'application/vnd.github.v3+json' } }
  );
  if (!resp.ok) throw new Error(`GitHub API error: ${resp.status}`);
  const data = await resp.json();
  const latest = (data.tag_name || '').replace(/^v/, '');
  return {
    available: isNewerVersion(current, latest),
    currentVersion: current,
    latestVersion: latest,
    releaseDate: data.published_at,
    releaseNotes: data.body,
  };
}

export async function applyUpdate(): Promise<string> {
  if (!isTauri) throw new Error('Update can only be applied in desktop mode');
  const { Command } = await import('@tauri-apps/plugin-shell');
  const cmd = Command.create('cloto_system', ['update', '--yes']);
  const output = await cmd.execute();
  if (output.code !== 0) throw new Error(output.stderr || 'Update failed');
  return output.stdout;
}
