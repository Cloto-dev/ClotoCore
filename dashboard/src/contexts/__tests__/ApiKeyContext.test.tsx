import { act, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

/**
 * The credential wiring, as opposed to the credential logic: minting when a key
 * arrives, and ending the session when one is forgotten. Both are properties of
 * the provider rather than of any screen, because a screen is exactly the place
 * where one of them would eventually be left out.
 */
async function load() {
  vi.resetModules();
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  const { ApiKeyProvider, useApiKey } = await import('../ApiKeyContext');
  return { ApiKeyProvider, useApiKey };
}

let fetchMock: ReturnType<typeof vi.fn>;

function callsTo(method: string): unknown[][] {
  return fetchMock.mock.calls.filter(([, init]) => (init as { method?: string } | undefined)?.method === method);
}

beforeEach(() => {
  sessionStorage.clear();
  fetchMock = vi.fn().mockResolvedValue({ ok: true });
  vi.stubGlobal('fetch', fetchMock);
});

afterEach(() => {
  sessionStorage.clear();
  vi.unstubAllGlobals();
  vi.resetModules();
});

describe('the API key provider', () => {
  it('mints a session when a key is entered', async () => {
    const { ApiKeyProvider, useApiKey } = await load();
    let setKey: ((k: string) => void) | null = null;
    function Probe() {
      setKey = useApiKey().setApiKey;
      return null;
    }

    render(
      <ApiKeyProvider>
        <Probe />
      </ApiKeyProvider>,
    );
    expect(callsTo('POST')).toHaveLength(0);

    await act(async () => setKey?.('k1'));

    await waitFor(() => expect(callsTo('POST')).toHaveLength(1));
    expect((callsTo('POST')[0][1] as { headers: Record<string, string> }).headers).toMatchObject({
      'X-API-Key': 'k1',
    });
  });

  it('ends the session when the key is forgotten', async () => {
    // A browser that has dropped the key while still holding a live admin
    // cookie has not signed out, it has only lost the ability to say so.
    const { ApiKeyProvider, useApiKey } = await load();
    let api: { setApiKey: (k: string) => void; forgetApiKey: () => void } | null = null;
    function Probe() {
      api = useApiKey();
      return null;
    }

    render(
      <ApiKeyProvider>
        <Probe />
      </ApiKeyProvider>,
    );
    await act(async () => api?.setApiKey('k1'));
    await waitFor(() => expect(callsTo('POST')).toHaveLength(1));

    await act(async () => api?.forgetApiKey());

    await waitFor(() => expect(callsTo('DELETE')).toHaveLength(1));
  });
});
