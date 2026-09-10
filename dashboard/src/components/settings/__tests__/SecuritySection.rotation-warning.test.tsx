import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

// Echo i18n keys so assertions do not depend on an i18n instance — the same
// approach the other settings tests take.
vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (k: string) => k }),
}));

const setApiKey = vi.fn();
vi.mock('../../../contexts/ApiKeyContext', () => ({
  useApiKey: () => ({ apiKey: 'live-key', setApiKey, forgetApiKey: vi.fn() }),
}));

// The providers panel fetches on mount and has nothing to do with rotation.
vi.mock('../LlmProvidersSection', () => ({ LlmProvidersSection: () => null }));

const regenerateApiKey = vi.fn();
vi.mock('../../../hooks/useApi', () => ({
  useApi: () => ({ apiKey: 'live-key', regenerateApiKey, invalidateApiKey: vi.fn() }),
}));

import { SecuritySection } from '../SecuritySection';

const WARNING =
  'This kernel received CLOTO_API_KEY from its process environment, which takes precedence over /etc/x/.env.';

/** Walk the confirm gate the regenerate button sits behind. */
function rotate() {
  render(<SecuritySection />);
  fireEvent.click(screen.getByLabelText('security.regenerate_label'));
  fireEvent.click(screen.getByText('confirm'));
}

describe('rotating the admin key', () => {
  beforeEach(() => {
    regenerateApiKey.mockReset();
    setApiKey.mockReset();
  });

  it('shows the kernel’s warning when the new key will not survive a restart', async () => {
    regenerateApiKey.mockResolvedValue({
      api_key: 'new-key',
      persisted_to: '/etc/x/.env',
      survives_restart: false,
      warning: WARNING,
    });

    rotate();

    // Without this the rotation looks like a plain success: the call returns
    // 200, the new key works, and the old one comes back at the next restart.
    await waitFor(() => expect(screen.getByText(WARNING)).toBeTruthy());
    expect(setApiKey).toHaveBeenCalledWith('new-key');
  });

  it('says nothing when the rotation is durable', async () => {
    regenerateApiKey.mockResolvedValue({
      api_key: 'new-key',
      persisted_to: '/home/u/.clotocore/.env',
      survives_restart: true,
      warning: null,
    });

    rotate();

    await waitFor(() => expect(setApiKey).toHaveBeenCalledWith('new-key'));
    expect(screen.queryByText(WARNING)).toBeNull();
  });
});
