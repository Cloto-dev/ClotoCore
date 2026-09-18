import { render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

// Echo i18n keys so assertions do not depend on copy.
vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (k: string) => k }),
}));

vi.mock('react-router-dom', () => ({
  useParams: () => ({ id: 'demo-panel' }),
}));

const { fetchModuleDocument, callForModule, writeForModule, getModuleWriteAccess, putModuleWriteConsent, apiObject } =
  vi.hoisted(() => {
    const fns = {
      fetchModuleDocument: vi.fn(),
      callForModule: vi.fn(),
      listModules: vi.fn(),
      writeForModule: vi.fn(),
      getModuleWriteAccess: vi.fn(),
      putModuleWriteConsent: vi.fn(),
    };
    // One object for every render: a fresh one each time would re-run every
    // effect that depends on it.
    return { ...fns, apiObject: { ...fns, apiKey: 'k' } };
  });
vi.mock('../../hooks/useApi', () => ({
  useApi: () => apiObject,
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

const SEND = 'POST /api/chat/agent.manager/messages';
const WRITER = { ...VALID, writes: [SEND] };

/** Post a message as the module's own frame would. */
async function postFromFrame(data: unknown) {
  const frame = await screen.findByTitle<HTMLIFrameElement>('Demo Panel');
  window.dispatchEvent(new MessageEvent('message', { data, source: frame.contentWindow }));
  await new Promise((r) => setTimeout(r, 0));
}

describe('ModulePage — writes', () => {
  beforeEach(() => {
    modules.current = [WRITER];
    callForModule.mockResolvedValue({ status: 200, body: {} });
    writeForModule.mockResolvedValue({ status: 201, body: {} });
    putModuleWriteConsent.mockResolvedValue(undefined);
  });

  it('sends a declared write to the kernel relay, never to the route itself', async () => {
    getModuleWriteAccess.mockResolvedValue({ panel_id: 'demo-panel', writes: [SEND], eligible: true });
    render(<ModulePage />);
    await postFromFrame({
      cloto: 'module.call',
      id: 'w1',
      method: 'POST',
      path: '/api/chat/agent.manager/messages',
      body: { content: 'hi' },
    });

    expect(writeForModule).toHaveBeenCalledWith('demo-panel', 'POST', '/api/chat/agent.manager/messages', {
      content: 'hi',
    });
    expect(callForModule).not.toHaveBeenCalled();
  });

  it('sends nothing for a write the panel did not declare', async () => {
    getModuleWriteAccess.mockResolvedValue({ panel_id: 'demo-panel', writes: [SEND], eligible: true });
    render(<ModulePage />);
    await postFromFrame({ cloto: 'module.call', id: 'w2', method: 'POST', path: '/api/chat/agent.other/messages' });

    expect(writeForModule).not.toHaveBeenCalled();
    expect(callForModule).not.toHaveBeenCalled();
  });

  it('asks for consent on an eligible panel, lists what it may do, and records the answer', async () => {
    getModuleWriteAccess.mockResolvedValueOnce({ panel_id: 'demo-panel', writes: [SEND], eligible: true });
    getModuleWriteAccess.mockResolvedValueOnce({
      panel_id: 'demo-panel',
      writes: [SEND],
      eligible: true,
      consent: { granted_at: '2026-09-18T00:00:00Z', valid: true },
    });
    render(<ModulePage />);

    expect(await screen.findByText('module_write_consent_title')).toBeTruthy();
    expect(screen.getByText('module_write_send_messages')).toBeTruthy();
    screen.getByText('module_write_allow').click();

    await waitFor(() => expect(putModuleWriteConsent).toHaveBeenCalledWith('demo-panel'));
    // Once recorded, the sheet is gone and the panel says it can write.
    expect(await screen.findByText('module_can_write')).toBeTruthy();
    expect(screen.queryByText('module_write_consent_title')).toBeNull();
  });

  it('asks again, and says why, when the consent no longer holds', async () => {
    getModuleWriteAccess.mockResolvedValue({
      panel_id: 'demo-panel',
      writes: [SEND],
      eligible: true,
      consent: { granted_at: '2026-09-18T00:00:00Z', valid: false },
    });
    render(<ModulePage />);

    expect(await screen.findByText('module_write_consent_lapsed')).toBeTruthy();
    expect(screen.queryByText('module_can_write')).toBeNull();
  });

  it('gives the reason and no way to consent when the panel cannot write', async () => {
    getModuleWriteAccess.mockResolvedValue({
      panel_id: 'demo-panel',
      writes: [SEND],
      eligible: false,
      reason: 'its connector was installed without a seal',
    });
    render(<ModulePage />);

    expect(await screen.findByText('module_write_ineligible')).toBeTruthy();
    expect(screen.queryByText('module_write_allow')).toBeNull();
    expect(screen.queryByText('module_can_write')).toBeNull();
  });

  it('asks nothing of a panel that declares no writes', async () => {
    modules.current = [VALID];
    render(<ModulePage />);
    await screen.findByTitle('Demo Panel');

    expect(getModuleWriteAccess).not.toHaveBeenCalled();
    expect(screen.queryByText('module_write_consent_title')).toBeNull();
  });
});
