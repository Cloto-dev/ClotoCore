import { act, fireEvent, render, screen } from '@testing-library/react';
import { StrictMode } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { AgentMetadata, MarketplaceCatalogEntry, McpServerInfo } from '../types';
import { EngineSetup } from './EngineSetup';
import type { EnginePlan, EngineSetupDeps } from './setupEngine';

const m = vi.hoisted(() => ({
  api: {
    getMarketplaceCatalog: vi.fn(),
    listLlmProviders: vi.fn(),
    getMarketplaceProgressUrl: vi.fn(() => '/api/marketplace/progress'),
  },
  remote: { servers: [] as McpServerInfo[] },
  setUpEngine: vi.fn(),
}));
vi.mock('../hooks/useApi', () => ({ useApi: () => m.api }));
vi.mock('../hooks/useMcpServers', () => ({ useMcpServers: () => m.remote }));
vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, values?: Record<string, unknown>) => (values ? `${key} ${JSON.stringify(values)}` : key),
  }),
}));
vi.mock('./setupEngine', async (original) => ({
  ...(await original<typeof import('./setupEngine')>()),
  setUpEngine: m.setUpEngine,
  watchInstallOver: () => vi.fn(),
}));

import { EngineSetupError } from './setupEngine';

function entry(id: string, name: string, extra: Partial<MarketplaceCatalogEntry> = {}): MarketplaceCatalogEntry {
  return {
    id,
    name,
    description: '',
    category: 'mind' as MarketplaceCatalogEntry['category'],
    version: '1',
    directory: '',
    dependencies: [],
    env_vars: [],
    optional_env_vars: [],
    tags: [],
    trust_level: 'standard',
    auto_restart: false,
    runtime: 'python',
    installed: false,
    update_available: false,
    running: false,
    ...extra,
  };
}

const CATALOG = [
  entry('cerebras', 'Cerebras', { llm_provider: { needs_api_key: true } }),
  entry('terminal', 'Terminal'),
  entry('groq', 'Groq', { llm_provider: { needs_api_key: true }, installed: true }),
  entry('ollama', 'Ollama', { llm_provider: { needs_api_key: false } }),
];

const MIO: AgentMetadata = {
  id: 'agent.mio',
  name: 'Mio',
  description: '',
  default_engine_id: 'cerebras',
  required_capabilities: [],
  enabled: true,
  last_seen: 0,
  status: 'online',
  metadata: {},
};

function props(over: Partial<Parameters<typeof EngineSetup>[0]> = {}) {
  return { agent: MIO, onSet: vi.fn(), onClose: vi.fn(), onDetails: vi.fn(), ...over };
}

function engineButton(name: string) {
  return screen.getByText(name, { selector: '.mate-engine-name' }).closest('button') as HTMLButtonElement;
}

async function ready() {
  await screen.findByText('Cerebras', { selector: '.mate-engine-name' });
}

function useButton() {
  return screen.getByRole('button', { name: 'mate.engine_use' });
}

/** The plan the panel handed to setUpEngine on the last click. */
function lastPlan(): EnginePlan {
  return m.setUpEngine.mock.calls.at(-1)?.[0];
}

describe('EngineSetup', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    m.api.getMarketplaceCatalog.mockResolvedValue({ servers: CATALOG, cached_at: '' });
    m.api.listLlmProviders.mockResolvedValue({ providers: [{ id: 'groq', has_key: true }] });
    m.remote = {
      servers: [{ id: 'groq', command: 'x', args: [], status: 'Connected', tools: [], is_cloto_sdk: false }],
    };
    m.setUpEngine.mockResolvedValue(undefined);
  });

  it("offers the catalog's engines only, says which need a key, and marks the one in use", async () => {
    render(<EngineSetup {...props()} />);
    await ready();
    expect(screen.queryByText('Terminal')).not.toBeInTheDocument();
    expect(engineButton('Cerebras')).toHaveAttribute('aria-pressed', 'true');
    expect(engineButton('Cerebras')).toHaveTextContent('mate.engine_in_use');
    expect(engineButton('Groq')).toHaveTextContent('mate.engine_installed');
    expect(engineButton('Ollama')).toHaveTextContent('mate.engine_no_key');
    expect(engineButton('Groq')).toHaveTextContent('mate.engine_needs_key');
    expect(engineButton('Ollama')).not.toHaveTextContent('mate.engine_installed');
  });

  it('shows the engines when mounted the way the app mounts it, under StrictMode', async () => {
    render(
      <StrictMode>
        <EngineSetup {...props()} />
      </StrictMode>,
    );
    expect(await screen.findByText('Cerebras', { selector: '.mate-engine-name' })).toBeInTheDocument();
    expect(screen.queryByText('mate.engine_loading')).not.toBeInTheDocument();
  });

  it('sends nothing on Cancel', async () => {
    const p = props();
    render(<EngineSetup {...p} />);
    await ready();
    fireEvent.click(screen.getByRole('button', { name: 'mate.engine_cancel' }));
    expect(p.onClose).toHaveBeenCalledTimes(1);
    expect(m.setUpEngine).not.toHaveBeenCalled();
  });

  it('asks for a key only when the engine needs one and none is saved', async () => {
    const { container } = render(<EngineSetup {...props()} />);
    await ready();
    expect(screen.getByText('mate.engine_key_new {"engine":"Cerebras"}')).toBeInTheDocument();
    expect(useButton()).toBeDisabled();
    fireEvent.change(container.querySelector('input[type="password"]') as HTMLInputElement, {
      target: { value: '  sk-1  ' },
    });
    expect(useButton()).toBeEnabled();
    fireEvent.click(engineButton('Groq'));
    expect(screen.getByText('mate.engine_key_saved')).toBeInTheDocument();
    expect(useButton()).toBeEnabled();
    fireEvent.click(engineButton('Ollama'));
    expect(container.querySelector('input[type="password"]')).toBeNull();
  });

  it('installs an engine that is missing and passes the trimmed key', async () => {
    const { container } = render(<EngineSetup {...props()} />);
    await ready();
    fireEvent.change(container.querySelector('input[type="password"]') as HTMLInputElement, {
      target: { value: '  sk-1  ' },
    });
    await act(async () => fireEvent.click(useButton()));
    expect(lastPlan()).toEqual({
      agentId: 'agent.mio',
      engineId: 'cerebras',
      install: true,
      start: false,
      key: 'sk-1',
    });
  });

  it('keeps the saved key and does not install an engine already there', async () => {
    render(<EngineSetup {...props()} />);
    await ready();
    fireEvent.click(engineButton('Groq'));
    await act(async () => fireEvent.click(useButton()));
    expect(lastPlan()).toEqual({ agentId: 'agent.mio', engineId: 'groq', install: false, start: false, key: null });
  });

  it('starts an installed engine that is not answering', async () => {
    m.remote = {
      servers: [{ id: 'groq', command: 'x', args: [], status: 'Disconnected', tools: [], is_cloto_sdk: false }],
    };
    render(<EngineSetup {...props()} />);
    await ready();
    fireEvent.click(engineButton('Groq'));
    await act(async () => fireEvent.click(useButton()));
    expect(lastPlan()).toMatchObject({ install: false, start: true });
  });

  it('forgets a typed key when another engine is chosen', async () => {
    const { container } = render(<EngineSetup {...props()} />);
    await ready();
    fireEvent.change(container.querySelector('input[type="password"]') as HTMLInputElement, {
      target: { value: 'sk-1' },
    });
    fireEvent.click(engineButton('Groq'));
    await act(async () => fireEvent.click(useButton()));
    expect(lastPlan().key).toBeNull();
  });

  it('lists the steps as they run, and tells the caller when the engine is set', async () => {
    m.setUpEngine.mockImplementation(async (_plan: EnginePlan, deps: EngineSetupDeps) => {
      deps.onStep('test', 'running');
      deps.onStep('test', 'done');
    });
    const p = props();
    render(<EngineSetup {...p} />);
    await ready();
    fireEvent.click(engineButton('Groq'));
    await act(async () => fireEvent.click(useButton()));
    const steps = screen.getByRole('list', { name: 'mate.engine_steps' });
    expect(steps).toHaveTextContent('mate.step_test');
    expect(steps.querySelector('.mate-step-done')).not.toBeNull();
    expect(steps.querySelector('.mate-step-waiting')).not.toBeNull();
    expect(p.onSet).toHaveBeenCalledTimes(1);
    expect(screen.getByRole('status')).toHaveTextContent('mate.engine_set {"engine":"Groq","name":"Mio"}');
    fireEvent.click(screen.getByRole('button', { name: 'mate.back_to_settings' }));
    expect(p.onClose).toHaveBeenCalledTimes(1);
  });

  it("says why it failed in its own words and the kernel's, and lets the user try again", async () => {
    m.setUpEngine.mockRejectedValueOnce(new EngineSetupError('test', 'rejected_key', 'HTTP 401'));
    const p = props();
    render(<EngineSetup {...p} />);
    await ready();
    fireEvent.click(engineButton('Groq'));
    await act(async () => fireEvent.click(useButton()));
    const alert = screen.getByRole('alert');
    expect(alert).toHaveTextContent('mate.engine_fail_rejected_key');
    expect(alert).toHaveTextContent('HTTP 401');
    expect(p.onSet).not.toHaveBeenCalled();
    expect(useButton()).toBeEnabled();
    await act(async () => fireEvent.click(useButton()));
    expect(m.setUpEngine).toHaveBeenCalledTimes(2);
  });

  it('cannot be left or started again while a setup is running', async () => {
    let finish: () => void = () => {};
    m.setUpEngine.mockReturnValue(new Promise<void>((resolve) => (finish = resolve)));
    render(<EngineSetup {...props()} />);
    await ready();
    fireEvent.click(engineButton('Groq'));
    fireEvent.click(useButton());
    expect(useButton()).toBeDisabled();
    expect(screen.getByRole('button', { name: 'mate.engine_cancel' })).toBeDisabled();
    await act(async () => finish());
  });

  it('says the list could not be read, and reads it again on request', async () => {
    m.api.getMarketplaceCatalog.mockRejectedValueOnce(new Error('Marketplace catalog: 503'));
    render(<EngineSetup {...props()} />);
    expect(await screen.findByText('mate.engine_list_failed')).toBeInTheDocument();
    expect(screen.getByText('Marketplace catalog: 503')).toBeInTheDocument();
    await act(async () => fireEvent.click(screen.getByRole('button', { name: 'mate.retry' })));
    expect(await screen.findByText('Cerebras')).toBeInTheDocument();
  });

  it('opens the detailed settings on request', async () => {
    const p = props();
    render(<EngineSetup {...p} />);
    await ready();
    fireEvent.click(screen.getByRole('button', { name: 'mate.engine_details' }));
    expect(p.onDetails).toHaveBeenCalledTimes(1);
  });
});
