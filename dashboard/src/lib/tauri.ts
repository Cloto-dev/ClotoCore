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
