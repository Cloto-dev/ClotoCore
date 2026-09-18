import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

// A stable `t` that shows the key and any `date`, as the real one is stable.
const { i18n } = vi.hoisted(() => ({
  i18n: { t: (k: string, o?: { date?: string }) => (o?.date ? `${k}|${o.date}` : k) },
}));
vi.mock('react-i18next', () => ({ useTranslation: () => i18n }));

const { fns, apiObject } = vi.hoisted(() => {
  const fns = {
    getHubAccess: vi.fn(),
    setHubAccessToken: vi.fn(),
    renewHubAccessToken: vi.fn(),
    forgetHubAccessToken: vi.fn(),
  };
  return { fns, apiObject: { ...fns, apiKey: 'k' } };
});
vi.mock('../../../hooks/useApi', () => ({ useApi: () => apiObject }));

import { HubAccessToken, shortFingerprint } from '../HubAccessToken';

const TOKEN = `chubr_${'ab'.repeat(32)}`;

function status(stage: 'valid' | 'expires_soon' | 'expired', over: Record<string, unknown> = {}) {
  return {
    token_prefix: 'chubr_abab…',
    token_id: 'T1',
    connector_ids: ['acme-console', 'acme-reports'],
    fingerprint: 'f'.repeat(64),
    expires_at: '2026-12-19T00:00:00Z',
    hub_origin: 'https://hub.example',
    stage,
    ...over,
  };
}

function renderIt() {
  return render(
    <MemoryRouter>
      <HubAccessToken />
    </MemoryRouter>,
  );
}

function paste(value: string) {
  const input = screen.getByPlaceholderText('chubr_…') as HTMLInputElement;
  fireEvent.change(input, { target: { value } });
  return input;
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe('HubAccessToken', () => {
  it('with no token, offers the paste field and nothing to renew or forget', async () => {
    fns.getHubAccess.mockResolvedValue(null);
    renderIt();
    expect(await screen.findByText('security.hub_access_none')).toBeTruthy();
    expect(screen.getByText('security.hub_access_set_label')).toBeTruthy();
    expect(screen.queryByText('security.hub_access_renew')).toBeNull();
    expect(screen.queryByText('security.hub_access_forget')).toBeNull();
  });

  it('sets a pasted token, clears the field at once, and shows what it opens', async () => {
    fns.getHubAccess.mockResolvedValue(null);
    let release: (v: unknown) => void = () => {};
    fns.setHubAccessToken.mockReturnValue(new Promise((r) => (release = r)));
    renderIt();
    await screen.findByText('security.hub_access_none');

    const input = paste(TOKEN);
    fireEvent.click(screen.getByText('security.hub_access_set'));
    // Cleared while the call is still in flight: the secret is off the screen
    // before the hub has answered.
    expect(input.value).toBe('');
    expect(fns.setHubAccessToken).toHaveBeenCalledWith(TOKEN);

    release(status('valid'));
    expect(await screen.findByText('acme-console')).toBeTruthy();
    expect(screen.getByText('acme-reports')).toBeTruthy();
    const fp = screen.getByTestId('hub-access-fingerprint');
    expect(fp.textContent).toBe('ffff ffff ffff ffff…');
    expect(fp.getAttribute('title')).toBe('f'.repeat(64));
    expect(screen.getByTestId('hub-access-expiry').textContent).toMatch(/^security\.hub_access_valid\|/);
    expect(screen.getByText('security.hub_access_replace_label')).toBeTruthy();
  });

  it('clears the field even when the hub refuses, and says why', async () => {
    fns.getHubAccess.mockResolvedValue(null);
    fns.setHubAccessToken.mockRejectedValue(new Error('the hub refused the token'));
    renderIt();
    await screen.findByText('security.hub_access_none');
    const input = paste(TOKEN);
    fireEvent.click(screen.getByText('security.hub_access_set'));
    expect(await screen.findByText('the hub refused the token')).toBeTruthy();
    expect(input.value).toBe('');
  });

  it('renews a token that is running out', async () => {
    fns.getHubAccess.mockResolvedValue(status('expires_soon'));
    fns.renewHubAccessToken.mockResolvedValue(status('valid', { token_id: 'T2', expires_at: '2027-03-19T00:00:00Z' }));
    renderIt();
    const expiry = await screen.findByTestId('hub-access-expiry');
    expect(expiry.textContent).toMatch(/^security\.hub_access_expires_soon\|/);
    expect(expiry.className).toContain('warn');

    fireEvent.click(screen.getAllByText('security.hub_access_renew').find((e) => e.tagName === 'BUTTON') as Element);
    await waitFor(() => expect(fns.renewHubAccessToken).toHaveBeenCalledTimes(1));
    await waitFor(() =>
      expect(screen.getByTestId('hub-access-expiry').textContent).toMatch(/^security\.hub_access_valid\|/),
    );
  });

  it('an expired token offers no renewal, only a new token', async () => {
    fns.getHubAccess.mockResolvedValue(status('expired'));
    renderIt();
    const expiry = await screen.findByTestId('hub-access-expiry');
    expect(expiry.textContent).toMatch(/^security\.hub_access_expired\|/);
    expect(expiry.className).toContain('bad');
    expect(screen.queryByText('security.hub_access_renew')).toBeNull();
    expect(screen.getByPlaceholderText('chubr_…')).toBeTruthy();
  });

  it('forgets only after confirming', async () => {
    fns.getHubAccess.mockResolvedValue(status('valid'));
    fns.forgetHubAccessToken.mockResolvedValue(undefined);
    renderIt();
    fireEvent.click(await screen.findByText('security.hub_access_forget'));
    expect(fns.forgetHubAccessToken).not.toHaveBeenCalled();
    fireEvent.click(await screen.findByText('confirm'));
    await waitFor(() => expect(fns.forgetHubAccessToken).toHaveBeenCalledTimes(1));
    expect(await screen.findByText('security.hub_access_none')).toBeTruthy();
  });

  it('says the token could not be read rather than showing none', async () => {
    fns.getHubAccess.mockRejectedValue(new Error('boom'));
    renderIt();
    expect(await screen.findByText(/boom|hub_access_load_failed/)).toBeTruthy();
    expect(screen.queryByText('security.hub_access_none')).toBeNull();
    expect(screen.queryByPlaceholderText('chubr_…')).toBeNull();
  });
});

describe('shortFingerprint', () => {
  it('shows the first 16 characters in groups of four', () => {
    expect(shortFingerprint('e80e9dd8b72c5835231230b906dc179d')).toBe('e80e 9dd8 b72c 5835…');
  });
});
