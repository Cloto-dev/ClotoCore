/**
 * The browser's session with the kernel.
 *
 * Every request the SPA makes with `fetch` can carry `X-API-Key`. The ones it
 * does *not* make itself cannot: an `EventSource`, an `<img src>`, an `<audio>`
 * element and the VRM model are started by the browser from a URL, and a URL
 * carries no headers. That is why the key ended up in `?token=` — and a URL is
 * also a history entry, a `Referer`, and a line in every proxy log between the
 * browser and the origin.
 *
 * A cookie is the other shape. The browser attaches it by itself to same-origin
 * requests, including the ones markup starts, so nothing has to put a credential
 * in a URL and nothing does. This module is the small amount of state that takes:
 * ask the kernel for one, and know whether it has arrived.
 *
 * # Why Tauri is exempt
 *
 * Under Tauri the SPA is served from a custom protocol, so its origin is not the
 * API's. A `SameSite=Strict` cookie the kernel sets is therefore never sent from
 * the desktop shell, and `?token=` remains the only channel there. The desktop
 * shell also *has* the key already (it injects it), so the risk the cookie
 * removes — a credential travelling through logs and history — is a browser
 * problem specifically. Every switch in this file is `isTauri`, and the reason is
 * always this one.
 */

import { isTauri } from '../lib/tauri';
import { API_BASE } from './api';

/**
 * The mint in flight (or settled) and the key that asked for it.
 *
 * Keyed by the key so that re-entering the same one does not mint again, and
 * changing it does.
 */
let session: { key: string; ready: Promise<boolean> } | null = null;

/**
 * Where this tab keeps the admin key.
 *
 * Named here rather than in the hook that reads it because the session has to
 * be restored *before* React exists (see `restoreBrowserSession`), and a second
 * spelling of a storage key is a drift whose only symptom is a session that
 * silently never starts.
 */
export const API_KEY_STORAGE_KEY = 'cloto-api-key';

/**
 * Ask the kernel for a session cookie, and resolve to whether it granted one.
 *
 * Idempotent per key. A failure clears the record rather than caching a "no"
 * forever: the kernel being briefly unreachable is not the same answer as the
 * key being wrong, and this layer cannot tell them apart.
 */
export function startBrowserSession(apiKey: string): Promise<boolean> {
  if (isTauri) return Promise.resolve(true);
  if (!apiKey) {
    session = null;
    return Promise.resolve(false);
  }
  if (session?.key === apiKey) return session.ready;

  const ready = fetch(`${API_BASE}/auth/session`, {
    method: 'POST',
    headers: { 'X-API-Key': apiKey },
    credentials: 'same-origin',
  })
    .then((res) => res.ok)
    .catch(() => false)
    .then((ok) => {
      if (!ok && session?.key === apiKey) session = null;
      return ok;
    });

  session = { key: apiKey, ready };
  return ready;
}

/**
 * Whether the credential a URL-borne request will rely on is in place.
 *
 * Callers that open a stream or render an asset URL should wait on this first.
 * `false` means there is nothing to wait for — no key has been offered yet — not
 * that the request would fail for some other reason.
 */
export function browserSessionReady(): Promise<boolean> {
  if (isTauri) return Promise.resolve(true);
  return session?.ready ?? Promise.resolve(false);
}

/**
 * End the session and clear the cookie.
 *
 * Paired with forgetting the key, and not optional there: a browser that has
 * dropped the key while still holding a live admin cookie has not signed out, it
 * has only lost the ability to say so.
 */
export async function endBrowserSession(): Promise<void> {
  session = null;
  if (isTauri) return;
  try {
    await fetch(`${API_BASE}/auth/session`, { method: 'DELETE', credentials: 'same-origin' });
  } catch {
    // Nothing to escalate to: the cookie expires on its own, and the caller is
    // already in the middle of forgetting the credential that would retry this.
  }
}

/**
 * Start a session from the key this tab already had, if any.
 *
 * Called before the first component mounts, and that timing is the point: a
 * child's effect runs *before* its provider's, so a mint started from a provider
 * effect happens after the children that open the event stream and render asset
 * URLs have already asked whether a credential exists — and their effects do not
 * re-run, because the key they depend on never changed. Restoring here removes
 * the ordering question instead of racing it.
 *
 * Not something to await: a slow or unreachable kernel must not hold up the
 * shell, and every consumer waits on the same promise through
 * [`browserSessionReady`].
 */
export function restoreBrowserSession(): Promise<boolean> {
  if (isTauri) return Promise.resolve(true);
  let stored = '';
  try {
    stored = sessionStorage.getItem(API_KEY_STORAGE_KEY) ?? '';
  } catch {
    // Storage throws in a locked-down browser; the wizard will ask for a key.
    return Promise.resolve(false);
  }
  return startBrowserSession(stored);
}

/** Test seam: forget any recorded session without asking the kernel. */
export function resetBrowserSessionForTests(): void {
  session = null;
}
