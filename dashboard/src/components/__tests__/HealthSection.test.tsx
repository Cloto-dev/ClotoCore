import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { HealthReport } from '../../services/api';

// Echo i18n keys (with the interpolated id) so assertions do not depend on copy.
vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (k: string, o?: Record<string, unknown>) => (o?.id ? `${k}:${String(o.id)}` : k),
  }),
}));

// The uninstall panel scans on mount and is not what these assertions are about.
vi.mock('../settings/DangerZone', () => ({ DangerZone: () => null }));

const { scanHealth, repairHealth, installMarketplaceServer } = vi.hoisted(() => ({
  scanHealth: vi.fn(),
  repairHealth: vi.fn(),
  installMarketplaceServer: vi.fn(),
}));
// One object for every render. The real useApi memoises on the API key, and a
// fresh object per call would instead re-fire the load effect on every state
// change — the component would keep re-scanning and the test would be asserting
// against whichever mock response that race landed on.
vi.mock('../../hooks/useApi', () => {
  const api = { scanHealth, repairHealth, installMarketplaceServer };
  return { useApi: () => api };
});

import { HealthSection } from '../settings/HealthSection';

const report = (stale: string[]): HealthReport => ({
  status: 'degraded',
  timestamp: '2026-09-05T00:00:00Z',
  db_size_bytes: 1024,
  checks: [
    {
      name: 'llm_proxy_untrusted_callers',
      status: 'degraded',
      message: 'served without a token',
      repairable: false,
      detail: { served_without_token: 3, stale_connectors: stale },
    },
    { name: 'port_availability', status: 'healthy', message: 'port ok', repairable: false },
  ],
});

describe('HealthSection — the update path for a connector the proxy named', () => {
  beforeEach(() => {
    scanHealth.mockReset();
    installMarketplaceServer.mockReset();
    installMarketplaceServer.mockResolvedValue({ started: true });
  });

  it('offers one update per named connector', async () => {
    scanHealth.mockResolvedValue(report(['groq', 'claude']));
    render(<HealthSection />);

    expect(await screen.findByText('health.update_connector:groq')).toBeTruthy();
    expect(screen.getByText('health.update_connector:claude')).toBeTruthy();
  });

  // The whole point of driving this from what the proxy observed: an
  // installation with nothing stale must not be told to update anything.
  it('offers nothing when the check named nobody', async () => {
    scanHealth.mockResolvedValue(report([]));
    render(<HealthSection />);

    await screen.findByText('served without a token');
    expect(screen.queryByText(/health\.update_connector/)).toBeNull();
  });

  it('updates the connector in place and re-scans, so the list can shrink', async () => {
    scanHealth.mockResolvedValueOnce(report(['groq'])).mockResolvedValueOnce(report([]));
    render(<HealthSection />);

    fireEvent.click(await screen.findByText('health.update_connector:groq'));

    await waitFor(() => {
      expect(installMarketplaceServer).toHaveBeenCalledWith({ server_id: 'groq', update: true });
    });
    // update=true is what re-vendors in place; a plain install is refused for
    // an already-installed server, so the flag is the whole action.
    await waitFor(() => expect(scanHealth).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(screen.queryByText('health.update_connector:groq')).toBeNull());
  });
});
