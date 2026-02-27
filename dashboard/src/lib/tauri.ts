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
