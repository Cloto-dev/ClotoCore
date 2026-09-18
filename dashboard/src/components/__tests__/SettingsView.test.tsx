import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

// Echo i18n keys so the assertions do not depend on copy.
vi.mock('react-i18next', () => ({ useTranslation: () => ({ t: (k: string) => k, i18n: { language: 'en' } }) }));

// Everything the seven sections reach for. Stable objects (vi.hoisted): a hook
// that returns a fresh array or object on every render re-arms the effects that
// depend on it, and the worker dies on the loop rather than failing a test.
const stub = vi.hoisted(() => {
  const empty = Promise.resolve([]);
  return {
    api: {
      apiKey: 'k',
      fetchJson: () => Promise.resolve({ enabled: false, value: 2 }),
      put: () => Promise.resolve({}),
      post: () => Promise.resolve({}),
      listConversations: () => empty,
      listLlmProviders: () =>
        Promise.resolve({
          providers: [
            {
              id: 'lmstudio',
              display_name: 'LM Studio',
              has_key: true,
              model_id: 'a/b',
              context_length: 8192,
              thinking_mode: 'auto',
              engine_status: 'connected',
              configured: true,
              model_placeholder: null,
            },
          ],
        }),
      listProviderModels: () => Promise.resolve({ models: [{ id: 'a/b' }, { id: 'c/d' }] }),
      scanHealth: () => Promise.resolve({ timestamp: 0, checks: [], db_size_bytes: 1, repairable: 0 }),
      getUninstallPlan: () => Promise.resolve(null),
      regenerateApiKey: () => Promise.resolve({ api_key: 'x' }),
      invalidateApiKey: () => Promise.resolve({}),
    },
    agents: { agents: [], setSelectedAgentId: () => {} },
    conversations: { refresh: () => Promise.resolve() },
    identity: { identity: { id: 'u', name: 'User' }, setIdentity: () => {} },
    apiKeyCtx: { apiKey: 'k', setApiKey: () => {}, forgetApiKey: () => {} },
    theme: {
      face: 'dark',
      mode: 'dark',
      setMode: () => {},
      themeId: 'default',
      setThemeId: () => {},
      themes: [],
      rejected: [],
      importPack: () => Promise.reject(new Error('unused')),
      removePack: () => Promise.resolve(),
    },
    languages: [{ code: 'en', label: 'English' }],
  };
});

vi.mock('../../hooks/useApi', () => ({ useApi: () => stub.api }));
vi.mock('../../hooks/useTheme', () => ({ useTheme: () => stub.theme }));
vi.mock('../../hooks/useEventStream', () => ({ useEventStream: () => {} }));
vi.mock('../../contexts/AgentContext', () => ({ useAgentContext: () => stub.agents }));
vi.mock('../../contexts/ConversationContext', () => ({ useConversations: () => stub.conversations }));
vi.mock('../../contexts/UserIdentityContext', () => ({ useUserIdentity: () => stub.identity }));
vi.mock('../../contexts/ApiKeyContext', () => ({ useApiKey: () => stub.apiKeyCtx }));
vi.mock('../../i18n', () => ({
  BUILTIN_LANGUAGES: stub.languages,
  getCustomLanguages: () => Promise.resolve([]),
  exportLanguageTemplate: () => '{}',
  importLanguagePack: () => Promise.resolve({ code: 'en', label: 'English' }),
}));
vi.mock('../../lib/tauri', () => ({
  isTauri: true,
  applyUpdate: () => Promise.resolve(''),
  checkForUpdates: () => Promise.resolve({ available: false }),
  getLanguagesDir: () => Promise.resolve(null),
  openFileDialog: () => Promise.resolve(null),
  readTextFile: () => Promise.resolve(null),
  UPDATE_CHANNEL_STORAGE_KEY: 'cloto-update-channel',
  UPDATE_CHANNELS: ['stable', 'current', 'experimental'],
}));
vi.mock('../../services/api', () => ({
  api: { listCronJobs: () => Promise.resolve([]), executeUninstall: () => Promise.resolve({}) },
  EVENTS_URL: 'http://localhost/events',
}));
vi.mock('../SetupWizard', () => ({ SetupWizard: () => null }));

import { MemoryRouter, Route, Routes, useLocation, useNavigationType } from 'react-router-dom';
import { SettingsView } from '../SettingsView';

/** Reads back what the router actually did, rather than what a spy was told. */
function Probe() {
  const location = useLocation();
  const type = useNavigationType();
  return <i data-testid="probe" data-search={location.search} data-type={type} />;
}

function openSettings(entry: string) {
  return render(
    <MemoryRouter initialEntries={[entry]}>
      <Routes>
        <Route
          path="/settings"
          element={
            <>
              <SettingsView />
              <Probe />
            </>
          }
        />
      </Routes>
    </MemoryRouter>,
  );
}

describe('which settings section is open', () => {
  it('opens the one the URL names', () => {
    openSettings('/settings?section=about');
    expect(screen.getByText('about.clotocore')).toBeTruthy();
    expect(screen.queryByText('general.group_display')).toBeNull();
  });

  it('falls back to the first section when the URL names one that does not exist', () => {
    openSettings('/settings?section=telemetry');
    expect(screen.getByText('general.group_display')).toBeTruthy();
    expect(screen.queryByText('about.clotocore')).toBeNull();
  });

  it('opens the first section when the URL names none', () => {
    openSettings('/settings');
    expect(screen.getByText('general.group_display')).toBeTruthy();
  });

  it('replaces the history entry on a section click, so Back leaves the page in one step', () => {
    openSettings('/settings');
    fireEvent.click(screen.getByRole('button', { name: 'sections.about' }));

    const probe = screen.getByTestId('probe');
    expect(probe.getAttribute('data-search')).toBe('?section=about');
    // PUSH here would stack one entry per click, and Back would walk the
    // sections instead of leaving the page.
    expect(probe.getAttribute('data-type')).toBe('REPLACE');
    expect(screen.getByText('about.clotocore')).toBeTruthy();
  });

  it('marks the open section in the rail, and only that one', () => {
    openSettings('/settings?section=health');
    const current = screen.getAllByRole('button').filter((b) => b.getAttribute('aria-current') === 'page');
    expect(current.map((b) => b.textContent)).toEqual(['sections.health']);
  });
});

describe('what the settings page is drawn with', () => {
  it('has no canvas and no native select in any of the seven sections', () => {
    const sections = ['general', 'conversations', 'security', 'advanced', 'health', 'log', 'about'];
    for (const section of sections) {
      const view = openSettings(`/settings?section=${section}`);
      expect(view.container.querySelector('canvas'), `canvas in ${section}`).toBeNull();
      // A native select is painted by the platform: neither the density nor
      // the surface steps this product decides reach it.
      expect(view.container.querySelector('select'), `select in ${section}`).toBeNull();
      view.unmount();
    }
  });

  it('picks a model with the page\u2019s own picker, where a native select used to be', async () => {
    const view = openSettings('/settings?section=security');
    const edit = await screen.findByRole('button', { name: 'a/b' });
    fireEvent.click(edit);
    // The list arrives from the provider probe; until it does the field is a
    // text input, and after it the control must still not be a native select.
    await waitFor(() => expect(screen.getByRole('button', { name: /llm_providers.model_label/ })).toBeTruthy());
    expect(view.container.querySelector('select')).toBeNull();
    expect(screen.getByRole('button', { name: /llm_providers.model_label/ }).getAttribute('aria-haspopup')).toBe(
      'listbox',
    );
  });

  it('names the page from the translation, never from a literal', () => {
    const view = openSettings('/settings');
    const heading = view.container.querySelector('.ws-head h1') as HTMLElement;
    expect(heading.textContent).toBe('title');
  });
});
