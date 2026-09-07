import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

// Echo i18n keys so assertions do not depend on copy.
vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (k: string) => k }),
}));

const { getMarketplaceCatalog, uninstallMarketplaceServer } = vi.hoisted(() => ({
  getMarketplaceCatalog: vi.fn(),
  uninstallMarketplaceServer: vi.fn(),
}));
vi.mock('../../hooks/useApi', () => ({
  useApi: () => ({ apiKey: 'a-key', getMarketplaceCatalog, uninstallMarketplaceServer }),
}));
vi.mock('../../hooks/useMarketplace', () => ({
  useMarketplace: () => ({ servers: [], isLoading: false, error: null, refetch: vi.fn() }),
}));

import { renderHook } from '@testing-library/react';
import { useUnlistedInstalls } from '../../hooks/useUnlistedInstalls';
import { MarketplaceTab } from '../mcp/MarketplaceTab';

beforeEach(() => {
  vi.clearAllMocks();
  uninstallMarketplaceServer.mockResolvedValue({});
});

describe('useUnlistedInstalls', () => {
  it('reports what the kernel sent', async () => {
    getMarketplaceCatalog.mockResolvedValue({
      servers: [],
      unlisted_installs: [{ name: 'stt', installed_version: '0.1.0', running: true }],
    });

    const { result } = renderHook(() => useUnlistedInstalls());
    await waitFor(() => expect(result.current).toHaveLength(1));
    expect(result.current[0].name).toBe('stt');
  });

  // A kernel that predates the field, and a catalog call that fails, must both
  // read as "nothing to say" rather than crash or invent a clean install set.
  it('says nothing when the field is absent', async () => {
    getMarketplaceCatalog.mockResolvedValue({ servers: [] });
    const { result } = renderHook(() => useUnlistedInstalls());
    await waitFor(() => expect(getMarketplaceCatalog).toHaveBeenCalled());
    expect(result.current).toEqual([]);
  });

  it('says nothing when the catalog cannot be read', async () => {
    getMarketplaceCatalog.mockRejectedValue(new Error('offline'));
    const { result } = renderHook(() => useUnlistedInstalls());
    await waitFor(() => expect(getMarketplaceCatalog).toHaveBeenCalled());
    expect(result.current).toEqual([]);
  });
});

describe('the unlisted-installs section', () => {
  it('offers each one for removal under the id the endpoint takes', async () => {
    getMarketplaceCatalog.mockResolvedValue({
      servers: [],
      unlisted_installs: [{ name: 'imagegen', installed_version: '0.1.0', running: false }],
    });

    render(<MarketplaceTab />);
    expect(await screen.findByText('imagegen')).toBeTruthy();

    // The dev build locks destructive actions behind an explicit unlock.
    fireEvent.click(screen.getByText('DEV LOCKED'));
    fireEvent.click(screen.getByText('marketplace.uninstall'));

    // The confirmation dialog labels its confirm button with the same word as
    // the row button, so it is the one that appears second.
    const buttons = screen.getAllByText('marketplace.uninstall');
    expect(buttons.length).toBeGreaterThan(1);
    fireEvent.click(buttons[buttons.length - 1]);

    await waitFor(() => expect(uninstallMarketplaceServer).toHaveBeenCalled());
    // The row name is the id: an unlisted install has no catalog id to send.
    expect(uninstallMarketplaceServer).toHaveBeenCalledWith('imagegen');
  });
});
