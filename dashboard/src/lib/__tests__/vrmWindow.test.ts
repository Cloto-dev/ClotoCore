import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

/**
 * The avatar window is the third place the key used to reach a URL, and the
 * easiest one to miss: closing `?token=` on the API leaves `?key=` sitting in a
 * second window's address bar and history.
 */
async function loadBrowserShell() {
  vi.resetModules();
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  return await import('../tauri');
}

let openMock: ReturnType<typeof vi.fn>;

beforeEach(() => {
  openMock = vi.fn();
  vi.stubGlobal('open', openMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.resetModules();
});

describe('the VRM avatar window in a browser', () => {
  it('opens a popup with no credential in its URL', async () => {
    const { openVrmWindow } = await loadBrowserShell();

    await openVrmWindow('agent-1', 'a-secret-admin-key');

    expect(openMock).toHaveBeenCalledTimes(1);
    const url = openMock.mock.calls[0][0] as string;
    // Same origin as its opener, so it shares the cookie jar and is already
    // authenticated; the key would only be adding itself to a second history.
    expect(url).toContain('/vrm-viewer/agent-1');
    expect(url).not.toContain('key=');
    expect(url).not.toContain('a-secret-admin-key');
  });

  it('opens the same URL when no key was passed at all', async () => {
    const { openVrmWindow } = await loadBrowserShell();
    await openVrmWindow('agent-1');
    expect(openMock.mock.calls[0][0]).toContain('/vrm-viewer/agent-1');
    expect(openMock.mock.calls[0][0]).not.toContain('?');
  });
});
