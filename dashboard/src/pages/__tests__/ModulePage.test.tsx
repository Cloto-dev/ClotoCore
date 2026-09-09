import { render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

// Echo i18n keys so assertions do not depend on copy.
vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (k: string) => k }),
}));

vi.mock('react-router-dom', () => ({
  useParams: () => ({ id: 'demo-panel' }),
}));

const { fetchModuleDocument, callForModule, listModules } = vi.hoisted(() => ({
  fetchModuleDocument: vi.fn(),
  callForModule: vi.fn(),
  listModules: vi.fn(),
}));
vi.mock('../../hooks/useApi', () => ({
  useApi: () => ({ fetchModuleDocument, callForModule, listModules, apiKey: 'k' }),
}));

const { modules } = vi.hoisted(() => ({ modules: { current: [] as unknown[] } }));
vi.mock('../../hooks/useModules', () => ({
  useModules: () => ({ modules: modules.current, isLoading: false, error: null, refetch: vi.fn() }),
}));

import { ModulePage } from '../ModulePage';

const VALID = {
  id: 'demo-panel',
  name: 'Demo Panel',
  entry: 'index.html',
  requires: ['GET /api/system/health'],
};

beforeEach(() => {
  vi.clearAllMocks();
  modules.current = [VALID];
  fetchModuleDocument.mockResolvedValue('<h1>hello</h1>');
});

describe('ModulePage', () => {
  it('renders the module in a frame that is denied its own origin', async () => {
    render(<ModulePage />);
    const frame = await screen.findByTitle<HTMLIFrameElement>('Demo Panel');

    expect(frame.getAttribute('srcdoc')).toBe('<h1>hello</h1>');
    // The whole isolation argument rests on this attribute: `allow-scripts`
    // alone leaves the frame in an opaque origin, so it holds no cookie and no
    // admin key. `allow-same-origin` here would silently hand every module the
    // operator's authority, and nothing else in the page would look different.
    expect(frame.getAttribute('sandbox')).toBe('allow-scripts');
  });

  it('asks the kernel for the entry the manifest names', async () => {
    modules.current = [{ ...VALID, entry: 'panel.html' }];
    render(<ModulePage />);
    await waitFor(() => expect(fetchModuleDocument).toHaveBeenCalledWith('demo-panel', 'panel.html'));
  });

  it('shows the kernel-side reason instead of a frame when the module was rejected', async () => {
    modules.current = [{ id: 'demo-panel', error: 'invalid module.json: key must be a string' }];
    render(<ModulePage />);

    expect(await screen.findByText(/invalid module.json/)).toBeTruthy();
    expect(screen.queryByTitle('Demo Panel')).toBeNull();
  });

  it('makes no kernel call for a message that did not come from its own frame', async () => {
    render(<ModulePage />);
    await screen.findByTitle('Demo Panel');

    // A message from the page itself (or any other window) carries a `source`
    // that is not this frame. Declared or not, it must not be proxied.
    window.postMessage({ cloto: 'module.call', id: 'r1', method: 'GET', path: '/api/system/health' }, '*');
    await new Promise((r) => setTimeout(r, 0));

    expect(callForModule).not.toHaveBeenCalled();
  });
});
