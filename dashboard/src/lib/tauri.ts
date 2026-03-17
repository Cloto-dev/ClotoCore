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
 * Parse a semver string into [major, minor, patch] + optional pre-release tag.
 * Stable releases are considered NEWER than pre-releases of the same version.
 */
function parseSemver(v: string): { nums: number[]; pre: string | null } {
  const [core, ...preParts] = v.split('-');
  const nums = core.split('.').map((n) => Number.parseInt(n, 10) || 0);
  const pre = preParts.length > 0 ? preParts.join('-') : null;
  return { nums, pre };
}

function isNewerVersion(current: string, latest: string): boolean {
  const c = parseSemver(current);
  const l = parseSemver(latest);
  // Compare major.minor.patch
  for (let i = 0; i < 3; i++) {
    if ((l.nums[i] || 0) !== (c.nums[i] || 0)) return (l.nums[i] || 0) > (c.nums[i] || 0);
  }
  // Same major.minor.patch — compare pre-release
  // stable (null) > pre-release ("alpha.2")
  if (c.pre !== null && l.pre === null) return true; // current is pre, latest is stable → upgrade
  if (c.pre === null && l.pre !== null) return false; // current is stable, latest is pre → no downgrade
  if (c.pre === null && l.pre === null) return false; // both stable, same version
  // Both pre-release: compare segments (e.g. alpha.4 vs alpha.5)
  const cParts = c.pre!.split('.');
  const lParts = l.pre!.split('.');
  const len = Math.max(cParts.length, lParts.length);
  for (let i = 0; i < len; i++) {
    if (cParts[i] === undefined) return true; // latest has more segments → newer
    if (lParts[i] === undefined) return false;
    const cn = Number(cParts[i]);
    const ln = Number(lParts[i]);
    if (!Number.isNaN(cn) && !Number.isNaN(ln)) {
      if (ln !== cn) return ln > cn;
    } else if (lParts[i] !== cParts[i]) {
      return lParts[i] > cParts[i];
    }
  }
  return false;
}

export async function checkForUpdates(): Promise<UpdateInfo> {
  const current = __APP_VERSION__;
  const isCurrentPreRelease = parseSemver(current).pre !== null;

  const resp = await fetch('https://api.github.com/repos/Cloto-dev/ClotoCore/releases?per_page=30', {
    headers: { Accept: 'application/vnd.github.v3+json' },
  });
  if (!resp.ok) throw new Error(`GitHub API error: ${resp.status}`);
  const releases: Array<{ tag_name: string; prerelease: boolean; draft: boolean; published_at: string; body: string }> =
    await resp.json();

  // Pre-release users: newest overall. Stable users: newest stable only.
  const published = releases.filter((r) => !r.draft);
  const target = isCurrentPreRelease ? published[0] : published.find((r) => !r.prerelease);

  if (!target) {
    return { available: false, currentVersion: current, latestVersion: current };
  }

  const latest = (target.tag_name || '').replace(/^v/, '');
  return {
    available: isNewerVersion(current, latest),
    currentVersion: current,
    latestVersion: latest,
    releaseDate: target.published_at,
    releaseNotes: target.body,
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
