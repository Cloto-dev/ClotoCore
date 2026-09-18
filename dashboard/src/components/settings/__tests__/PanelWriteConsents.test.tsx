import { render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

// A stable `t`, as the real one is: a fresh function on every render would
// rebuild `load` and re-run the effect that depends on it.
const { i18n } = vi.hoisted(() => ({ i18n: { t: (k: string) => k } }));
vi.mock('react-i18next', () => ({
  useTranslation: () => i18n,
}));

const { listModuleWriteConsents, deleteModuleWriteConsent, apiObject } = vi.hoisted(() => {
  const fns = { listModuleWriteConsents: vi.fn(), deleteModuleWriteConsent: vi.fn() };
  return { ...fns, apiObject: { ...fns, apiKey: 'k' } };
});
vi.mock('../../../hooks/useApi', () => ({ useApi: () => apiObject }));

import { PanelWriteConsents } from '../PanelWriteConsents';

const ROW = {
  panel_id: 'ops-console-console',
  writes_digest: 'd',
  connector_version: '1.0.0',
  granted_at: '2026-09-18T00:00:00Z',
  granted_by: 'operator',
};

beforeEach(() => {
  vi.clearAllMocks();
  deleteModuleWriteConsent.mockResolvedValue(undefined);
});

describe('PanelWriteConsents', () => {
  it('lists each allowed panel and withdraws the one asked for', async () => {
    listModuleWriteConsents.mockResolvedValueOnce([ROW]).mockResolvedValueOnce([]);
    render(<PanelWriteConsents />);

    expect(await screen.findByText('ops-console-console')).toBeTruthy();
    screen.getByText('security.panel_consents_revoke').click();

    await waitFor(() => expect(deleteModuleWriteConsent).toHaveBeenCalledWith('ops-console-console'));
    expect(await screen.findByText('security.panel_consents_empty')).toBeTruthy();
  });

  it('says the list could not be read rather than showing it as empty', async () => {
    listModuleWriteConsents.mockRejectedValue(new Error('boom'));
    render(<PanelWriteConsents />);

    expect(await screen.findByText(/boom|panel_consents_load_failed/)).toBeTruthy();
    expect(screen.queryByText('security.panel_consents_empty')).toBeNull();
  });
});
