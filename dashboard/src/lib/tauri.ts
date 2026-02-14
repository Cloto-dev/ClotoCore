/**
 * Tauri-specific utilities. These functions are no-ops in browser mode.
 */

export const isTauri = '__TAURI_INTERNALS__' in window;

/**
 * Capture the primary screen. Returns a base64-encoded PNG string.
 * Only available in Tauri mode; throws in browser mode.
 */
export async function captureScreen(): Promise<string> {
  if (!isTauri) {
    throw new Error('Screen capture is only available in Tauri desktop mode');
  }
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<string>('capture_screen');
}

/**
 * Get the kernel HTTP port from the Tauri backend.
 */
export async function getKernelPort(): Promise<number> {
  if (!isTauri) return 8081;
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<number>('get_kernel_port');
}

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
