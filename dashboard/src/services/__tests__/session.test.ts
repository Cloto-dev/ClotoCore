import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

/** Re-import the module graph with `isTauri` decided by `shell`. */
async function load(shell: 'browser' | 'tauri') {
  vi.resetModules();
  if (shell === 'tauri') {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  } else {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  }
  const session = await import('../session');
  const { API_BASE } = await import('../api');
  return { ...session, API_BASE };
}

let fetchMock: ReturnType<typeof vi.fn>;

beforeEach(() => {
  sessionStorage.clear();
  fetchMock = vi.fn().mockResolvedValue({ ok: true });
  vi.stubGlobal('fetch', fetchMock);
});

afterEach(() => {
  sessionStorage.clear();
  vi.unstubAllGlobals();
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  vi.resetModules();
});

describe('starting a browser session', () => {
  it('asks the kernel once and reports that it was granted', async () => {
    const { startBrowserSession, browserSessionReady, API_BASE } = await load('browser');

    await expect(startBrowserSession('k1')).resolves.toBe(true);
    await expect(browserSessionReady()).resolves.toBe(true);

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe(`${API_BASE}/auth/session`);
    expect(init).toMatchObject({ method: 'POST', credentials: 'same-origin' });
    // The key travels in the header on this one request, which is the whole
    // point: it is the last time it has to travel anywhere.
    expect(init.headers).toMatchObject({ 'X-API-Key': 'k1' });
    expect(url).not.toContain('k1');
  });

  it('does not mint again for the same key, and does for a new one', async () => {
    const { startBrowserSession } = await load('browser');
    await startBrowserSession('k1');
    await startBrowserSession('k1');
    expect(fetchMock).toHaveBeenCalledTimes(1);

    await startBrowserSession('k2');
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it('reports nothing to wait for before a key has been offered', async () => {
    const { browserSessionReady } = await load('browser');
    await expect(browserSessionReady()).resolves.toBe(false);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('treats an empty key as no session rather than as a request', async () => {
    const { startBrowserSession, browserSessionReady } = await load('browser');
    await expect(startBrowserSession('')).resolves.toBe(false);
    await expect(browserSessionReady()).resolves.toBe(false);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('retries after a refusal instead of caching the "no" forever', async () => {
    const { startBrowserSession, browserSessionReady } = await load('browser');
    fetchMock.mockResolvedValueOnce({ ok: false });

    await expect(startBrowserSession('k1')).resolves.toBe(false);
    // A kernel that was briefly unreachable and a key that is wrong look the
    // same from here, so the record is cleared rather than believed.
    await expect(browserSessionReady()).resolves.toBe(false);

    await expect(startBrowserSession('k1')).resolves.toBe(true);
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it('survives the kernel being unreachable', async () => {
    const { startBrowserSession } = await load('browser');
    fetchMock.mockRejectedValueOnce(new Error('connection refused'));
    await expect(startBrowserSession('k1')).resolves.toBe(false);
  });
});

describe('restoring a session before anything else asks', () => {
  /** Make the Access attempt — always the first request — be refused. */
  function noEdgeInFront() {
    fetchMock.mockResolvedValueOnce({ ok: false });
  }

  it('signs in with the assertion the edge added, and leaves the key alone', async () => {
    sessionStorage.setItem('cloto-api-key', 'k1');
    const { restoreBrowserSession, browserSessionReady, API_BASE } = await load('browser');

    await expect(restoreBrowserSession()).resolves.toBe(true);
    await expect(browserSessionReady()).resolves.toBe(true);

    // One request, and it is the Access one: a browser that came through the
    // edge has no reason to spend the key it happens to be holding.
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(fetchMock.mock.calls[0][0]).toBe(`${API_BASE}/auth/session/access`);
  });

  it('sends nothing of ours on the Access request', async () => {
    const { restoreBrowserSession } = await load('browser');
    await restoreBrowserSession();

    const [url, init] = fetchMock.mock.calls[0];
    expect(init).toMatchObject({ method: 'POST', credentials: 'same-origin' });
    // The assertion belongs to the edge and is added in front of the origin.
    // Anything this side attached would be a credential somewhere it is not
    // needed, which is the thing this whole module exists to stop.
    expect(init.headers).toBeUndefined();
    expect(init.body).toBeUndefined();
    expect(String(url)).not.toContain('token');
    expect(String(url)).not.toContain('key=');
  });

  it('falls back to the stored key when there is no edge in front', async () => {
    sessionStorage.setItem('cloto-api-key', 'k1');
    const { restoreBrowserSession, browserSessionReady, API_BASE } = await load('browser');
    noEdgeInFront();

    await expect(restoreBrowserSession()).resolves.toBe(true);
    await expect(browserSessionReady()).resolves.toBe(true);

    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(fetchMock.mock.calls[1][0]).toBe(`${API_BASE}/auth/session`);
    expect(fetchMock.mock.calls[1][1].headers).toMatchObject({ 'X-API-Key': 'k1' });
  });

  it('reports the answer to the whole sequence, not to the Access attempt alone', async () => {
    // The ordering this exists for: a child's effect runs before its provider's,
    // so a consumer asks whether a credential exists while the restore is still
    // in flight — and their effects do not re-run, so whatever they are told is
    // final for them. Being told "no" because the *first* of two attempts had
    // failed is the regression this pins.
    sessionStorage.setItem('cloto-api-key', 'k1');
    const { restoreBrowserSession, browserSessionReady } = await load('browser');
    noEdgeInFront();

    const restoring = restoreBrowserSession();
    const askedEarly = browserSessionReady();

    await expect(askedEarly).resolves.toBe(true);
    await expect(restoring).resolves.toBe(true);
  });

  it('asks the edge even when this tab has no key', async () => {
    const { restoreBrowserSession, API_BASE } = await load('browser');

    await expect(restoreBrowserSession()).resolves.toBe(true);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(fetchMock.mock.calls[0][0]).toBe(`${API_BASE}/auth/session/access`);
  });

  it('reports no session when neither the edge nor a key grants one', async () => {
    const { restoreBrowserSession, browserSessionReady } = await load('browser');
    noEdgeInFront();

    await expect(restoreBrowserSession()).resolves.toBe(false);
    await expect(browserSessionReady()).resolves.toBe(false);
    // The key was never tried, because there is none: one request, not two.
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it('still asks the edge when storage is locked down', async () => {
    const { restoreBrowserSession } = await load('browser');
    const getItem = vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('storage disabled');
    });
    try {
      // Storage rules out the key, not the edge, which wants nothing from it.
      await expect(restoreBrowserSession()).resolves.toBe(true);
      expect(fetchMock).toHaveBeenCalledTimes(1);
    } finally {
      getItem.mockRestore();
    }
  });

  it('uses the key name the hook stores under', async () => {
    const { API_KEY_STORAGE_KEY } = await load('browser');
    expect(API_KEY_STORAGE_KEY).toBe('cloto-api-key');
  });
});

describe('ending a browser session', () => {
  it('tells the kernel and forgets the record', async () => {
    const { startBrowserSession, endBrowserSession, browserSessionReady, API_BASE } = await load('browser');
    await startBrowserSession('k1');

    await endBrowserSession();

    const [url, init] = fetchMock.mock.calls[1];
    expect(url).toBe(`${API_BASE}/auth/session`);
    expect(init).toMatchObject({ method: 'DELETE', credentials: 'same-origin' });
    await expect(browserSessionReady()).resolves.toBe(false);
  });

  it('does not throw when the kernel is already gone', async () => {
    const { endBrowserSession } = await load('browser');
    fetchMock.mockRejectedValueOnce(new Error('connection refused'));
    await expect(endBrowserSession()).resolves.toBeUndefined();
  });
});

describe('under Tauri', () => {
  it('never asks for a cookie, and reports itself ready', async () => {
    // The desktop shell already holds the key, and its origin is not the API's,
    // so a cookie would be minted and then never sent.
    const { startBrowserSession, browserSessionReady, endBrowserSession, restoreBrowserSession } = await load('tauri');

    sessionStorage.setItem('cloto-api-key', 'k1');
    await expect(browserSessionReady()).resolves.toBe(true);
    await expect(startBrowserSession('k1')).resolves.toBe(true);
    await expect(restoreBrowserSession()).resolves.toBe(true);
    await endBrowserSession();

    expect(fetchMock).not.toHaveBeenCalled();
  });
});
