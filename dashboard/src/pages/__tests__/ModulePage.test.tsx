import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

// Echo i18n keys so assertions do not depend on copy.
vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (k: string) => k }),
}));

const { params, navigate } = vi.hoisted(() => ({ params: { current: { id: 'demo-panel' } }, navigate: vi.fn() }));
vi.mock('react-router-dom', () => ({
  useParams: () => params.current,
  useNavigate: () => navigate,
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
  params.current = { id: 'demo-panel' };
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

// Listed sorted by id, declared in the other order: the head has to follow the
// declaration.
const page = (panel: string, position: number, extra: Record<string, unknown> = {}) => ({
  id: `cil-${panel}`,
  name: panel === 'zeta' ? 'Decisions' : 'Chart',
  description: `about ${panel}`,
  entry: 'index.html',
  connector: { id: 'cil', name: 'CIL Console', position },
  ...extra,
});

describe('ModulePage — pages', () => {
  beforeEach(() => {
    modules.current = [page('alpha', 1), page('zeta', 0)];
  });

  it('titles a page with its connector and says which page of how many it is', async () => {
    params.current = { id: 'cil-zeta' };
    render(<ModulePage />);

    expect(await screen.findByRole('heading', { name: 'CIL Console' })).toBeTruthy();
    expect(screen.getByText('about zeta')).toBeTruthy();
    const current = screen.getByText('1/2').parentElement;
    expect(current?.textContent).toBe('Decisions 1/2');
    // The frame keeps the page's own name: it is what a screen reader announces for it.
    expect(await screen.findByTitle('Decisions')).toBeTruthy();
  });

  it('goes to the next page, and has no previous page on the first', async () => {
    params.current = { id: 'cil-zeta' };
    render(<ModulePage />);

    const prev = await screen.findByLabelText<HTMLButtonElement>('module_page_prev');
    expect(prev.disabled).toBe(true);
    fireEvent.click(prev);
    expect(navigate).not.toHaveBeenCalled();
    fireEvent.click(screen.getByLabelText('module_page_next'));
    expect(navigate).toHaveBeenCalledWith('/modules/cil-alpha');
  });

  it('goes back from the last page, and does not wrap past it', async () => {
    params.current = { id: 'cil-alpha' };
    render(<ModulePage />);

    expect(await screen.findByText('2/2')).toBeTruthy();
    const next = screen.getByLabelText<HTMLButtonElement>('module_page_next');
    expect(next.disabled).toBe(true);
    fireEvent.click(next);
    expect(navigate).not.toHaveBeenCalled();
    fireEvent.click(screen.getByLabelText('module_page_prev'));
    expect(navigate).toHaveBeenCalledWith('/modules/cil-zeta');
  });

  it('shows no page controls for a connector with one panel', async () => {
    modules.current = [page('zeta', 0)];
    params.current = { id: 'cil-zeta' };
    render(<ModulePage />);

    expect(await screen.findByRole('heading', { name: 'Decisions' })).toBeTruthy();
    expect(screen.queryByLabelText('module_page_next')).toBeNull();
  });

  // Each page is its own panel with its own consent. An answer given on one —
  // here "not now" — must not silence the question on the next.
  it('asks again on the next page after the question was put off on this one', async () => {
    modules.current = [page('alpha', 1, { writes: [SEND] }), page('zeta', 0, { writes: [SEND] })];
    getModuleWriteAccess.mockResolvedValue({ panel_id: 'x', writes: [SEND], eligible: true });
    params.current = { id: 'cil-zeta' };
    const { rerender } = render(<ModulePage />);

    fireEvent.click(await screen.findByText('module_write_not_now'));
    expect(screen.queryByText('module_write_allow')).toBeNull();

    params.current = { id: 'cil-alpha' };
    rerender(<ModulePage />);
    expect(await screen.findByText('module_write_allow')).toBeTruthy();
  });
});
