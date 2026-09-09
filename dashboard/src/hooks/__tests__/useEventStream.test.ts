import { renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const EVENTS_URL = 'http://localhost:8081/api/events';

/** Records every stream the hook opens, and the URL it opened it with. */
class FakeEventSource {
  static instances: FakeEventSource[] = [];
  static readonly CLOSED = 2;
  readyState = 0;
  onmessage: ((e: MessageEvent) => void) | null = null;
  onerror: (() => void) | null = null;

  constructor(public url: string) {
    FakeEventSource.instances.push(this);
  }
  addEventListener(): void {}
  close(): void {
    this.readyState = FakeEventSource.CLOSED;
  }
}

async function load(shell: 'browser' | 'tauri') {
  vi.resetModules();
  if (shell === 'tauri') {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  } else {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  }
  const { startBrowserSession } = await import('../../services/session');
  const { useEventStream } = await import('../useEventStream');
  return { startBrowserSession, useEventStream };
}

beforeEach(() => {
  FakeEventSource.instances = [];
  vi.stubGlobal('EventSource', FakeEventSource);
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true }));
});

afterEach(() => {
  vi.unstubAllGlobals();
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  vi.resetModules();
});

describe('the event stream in a browser', () => {
  it('opens with a bare URL once the session cookie is in place', async () => {
    const { startBrowserSession, useEventStream } = await load('browser');
    await startBrowserSession('a-secret-admin-key');

    renderHook(() => useEventStream(EVENTS_URL, () => {}, 'a-secret-admin-key'));

    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(1));
    const { url } = FakeEventSource.instances[0];
    expect(url).toBe(EVENTS_URL);
    expect(url).not.toContain('token=');
    expect(url).not.toContain('a-secret-admin-key');
  });

  it('does not open the stream before there is a credential for it', async () => {
    // A refused EventSource does not retry promptly, it starts backing off, so
    // opening one early costs seconds of missing events rather than a retry.
    const { useEventStream } = await load('browser');

    renderHook(() => useEventStream(EVENTS_URL, () => {}, 'a-secret-admin-key'));
    await new Promise((resolve) => setTimeout(resolve, 20));

    expect(FakeEventSource.instances).toHaveLength(0);
  });

  it('opens it as soon as the session arrives', async () => {
    const { startBrowserSession, useEventStream } = await load('browser');
    const { rerender } = renderHook(({ key }: { key: string }) => useEventStream(EVENTS_URL, () => {}, key), {
      initialProps: { key: '' },
    });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(FakeEventSource.instances).toHaveLength(0);

    await startBrowserSession('a-secret-admin-key');
    rerender({ key: 'a-secret-admin-key' });

    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(1));
    expect(FakeEventSource.instances[0].url).toBe(EVENTS_URL);
  });
});

describe('the event stream under Tauri', () => {
  it('still carries the key in the URL, because no cookie is ever sent there', async () => {
    const { useEventStream } = await load('tauri');

    renderHook(() => useEventStream(EVENTS_URL, () => {}, 'k1'));

    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(1));
    expect(FakeEventSource.instances[0].url).toBe(`${EVENTS_URL}?token=k1`);
  });
});
