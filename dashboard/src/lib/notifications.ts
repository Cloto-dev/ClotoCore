import { isTauri } from './tauri';

/** Send an OS notification in Tauri mode (no-op in browser). */
export async function sendNativeNotification(title: string, body: string) {
  if (!isTauri) return;
  try {
    const { isPermissionGranted, requestPermission, sendNotification } =
      await import('@tauri-apps/plugin-notification');
    let permitted = await isPermissionGranted();
    if (!permitted) {
      const result = await requestPermission();
      permitted = result === 'granted';
    }
    if (permitted) {
      sendNotification({ title, body });
    }
  } catch {
    // Notification plugin not available or permission denied - silently skip
  }
}
