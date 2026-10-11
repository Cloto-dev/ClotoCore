import { describe, expect, it, vi } from 'vitest';
import type { McpServerInfo, McpServerStatus } from '../types';
import {
  type EnginePlan,
  type EngineSetupApi,
  EngineSetupError,
  type SetupStep,
  type StepState,
  setUpEngine,
  stepsFor,
  watchInstallOver,
} from './setupEngine';

function server(status: McpServerStatus, extra: Partial<McpServerInfo> = {}): McpServerInfo {
  return { id: 'groq', command: 'x', args: [], status, tools: [], is_cloto_sdk: false, ...extra };
}

const PLAN: EnginePlan = { agentId: 'agent.mio', engineId: 'groq', install: true, start: false, key: 'sk-new' };

/** An API that records every call in order, and answers the way a healthy kernel does. */
function fakeApi(over: Partial<Record<keyof EngineSetupApi, unknown>> = {}) {
  const calls: string[] = [];
  const record =
    <T>(name: string, answer: T) =>
    (...args: unknown[]) => {
      calls.push(`${name} ${JSON.stringify(args)}`);
      return typeof answer === 'function' ? (answer as (...a: unknown[]) => unknown)(...args) : Promise.resolve(answer);
    };
  const api = {
    installMarketplaceServer: record('install', { started: true }),
    listMcpServers: record('list', { servers: [server('Connected')] }),
    restartMcpServer: record('restart', undefined),
    setLlmProviderKey: record('key', undefined),
    testProviderConnection: record('test', { status: 'ok', error: null }),
    getAgentServerAccess: record('grants', { server: null, tools: [{ tool_name: 'think', permission: 'allow' }] }),
    putAgentServerAccess: record('allow', undefined),
    updateAgent: record('use', undefined),
    ...Object.fromEntries(Object.entries(over).map(([k, v]) => [k, record(k, v)])),
  } as unknown as EngineSetupApi;
  return { api, calls };
}

function run(plan: EnginePlan, api: EngineSetupApi, installError: string | null = null) {
  const steps: [SetupStep, StepState][] = [];
  const promise = setUpEngine(plan, {
    api,
    watchInstall: async (start) => {
      await start();
      return { error: installError };
    },
    onStep: (step, state) => steps.push([step, state]),
    wait: () => Promise.resolve(),
  });
  return { promise, steps };
}

describe('stepsFor', () => {
  it('installs a missing engine, starts a stopped one, and saves only a key that was entered', () => {
    expect(stepsFor(PLAN)).toEqual(['install', 'key', 'test', 'allow', 'use']);
    expect(stepsFor({ ...PLAN, install: false, start: true, key: null })).toEqual(['start', 'test', 'allow', 'use']);
    expect(stepsFor({ ...PLAN, install: false, start: false, key: null })).toEqual(['test', 'allow', 'use']);
  });
});

describe('setUpEngine', () => {
  it('runs the kernel calls in order and reports each step', async () => {
    const { api, calls } = fakeApi();
    const { promise, steps } = run(PLAN, api);
    await promise;
    expect(calls.map((c) => c.split(' ')[0])).toEqual(['install', 'list', 'key', 'test', 'grants', 'allow', 'use']);
    expect(calls).toContain('install [{"server_id":"groq","auto_start":true}]');
    expect(calls).toContain('key ["groq","sk-new"]');
    expect(calls).toContain('use ["agent.mio",{"default_engine_id":"groq"}]');
    expect(steps.filter(([, s]) => s === 'done').map(([step]) => step)).toEqual([
      'install',
      'key',
      'test',
      'allow',
      'use',
    ]);
  });

  it('leaves the partner untouched when the key is refused', async () => {
    const { api, calls } = fakeApi({ testProviderConnection: { status: 'auth_failed', error: 'HTTP 401' } });
    const { promise, steps } = run(PLAN, api);
    await expect(promise).rejects.toMatchObject({ step: 'test', reason: 'rejected_key', detail: 'HTTP 401' });
    expect(calls.some((c) => /^(grants|allow|use) /.test(c))).toBe(false);
    expect(steps.at(-1)).toEqual(['test', 'failed']);
  });

  it('calls a provider it cannot reach unreachable, and accepts one that only lacks a model list', async () => {
    const unreachable = fakeApi({ testProviderConnection: { status: 'unreachable', error: 'connection refused' } });
    await expect(run(PLAN, unreachable.api).promise).rejects.toMatchObject({ reason: 'unreachable' });
    const listless = fakeApi({ testProviderConnection: { status: 'model_list_unavailable', error: null } });
    await expect(run(PLAN, listless.api).promise).resolves.toBeUndefined();
  });

  it("stops at the install with the kernel's reason, before any key or grant", async () => {
    const { api, calls } = fakeApi();
    await expect(run(PLAN, api, 'seal verification failed').promise).rejects.toMatchObject({
      step: 'install',
      reason: 'not_installed',
      detail: 'seal verification failed',
    });
    expect(calls.map((c) => c.split(' ')[0])).toEqual(['install']);
  });

  it('reports a server that came up in error with its message', async () => {
    const { api } = fakeApi({ listMcpServers: { servers: [server('Error', { status_message: 'spawn failed' })] } });
    await expect(run(PLAN, api).promise).rejects.toMatchObject({ reason: 'not_installed', detail: 'spawn failed' });
  });

  it('gives up on a server that never appears, and on one that never connects', async () => {
    const missing = fakeApi({ listMcpServers: { servers: [] } });
    await expect(run(PLAN, missing.api).promise).rejects.toMatchObject({ step: 'install', reason: 'not_installed' });
    const stuck = fakeApi({ listMcpServers: { servers: [server('Connecting')] } });
    const plan = { ...PLAN, install: false, start: true };
    await expect(run(plan, stuck.api).promise).rejects.toMatchObject({ step: 'start', reason: 'timeout' });
  });

  it('starts an installed engine that is not answering, and waits until it is', async () => {
    let reads = 0;
    const { api, calls } = fakeApi({
      listMcpServers: () => Promise.resolve({ servers: [server(++reads < 3 ? 'Connecting' : 'Connected')] }),
    });
    await run({ ...PLAN, install: false, start: true, key: null }, api).promise;
    expect(calls[0]).toBe('restart ["groq"]');
    expect(reads).toBe(3);
  });

  it("keeps the agent's tool answers when it allows the server, and writes nothing when already allowed", async () => {
    const first = fakeApi();
    await run(PLAN, first.api).promise;
    expect(first.calls).toContain(
      'allow ["agent.mio","groq",{"server":"allow","tools":[{"tool_name":"think","permission":"allow"}]}]',
    );
    const allowed = fakeApi({ getAgentServerAccess: { server: 'allow', tools: [] } });
    await run(PLAN, allowed.api).promise;
    expect(allowed.calls.some((c) => c.startsWith('allow '))).toBe(false);
  });

  it('reports an unexpected failure as the step it happened in', async () => {
    const { api } = fakeApi({ setLlmProviderKey: () => Promise.reject(new Error('Provider not found')) });
    const failure = await run(PLAN, api).promise.catch((e) => e);
    expect(failure).toBeInstanceOf(EngineSetupError);
    expect(failure).toMatchObject({ step: 'key', reason: 'other', detail: 'Provider not found' });
  });
});

/** A stand-in for EventSource that the test drives by hand. */
class FakeSource {
  listeners: Record<string, ((event: { data: string }) => void)[]> = {};
  onerror: (() => void) | null = null;
  closed = false;
  addEventListener(name: string, fn: (event: { data: string }) => void) {
    this.listeners[name] = [...(this.listeners[name] ?? []), fn];
  }
  emit(name: string, data = '') {
    for (const fn of this.listeners[name] ?? []) fn({ data });
  }
  close() {
    this.closed = true;
  }
}

describe('watchInstallOver', () => {
  function watch() {
    const source = new FakeSource();
    const watchInstall = watchInstallOver('/api/marketplace/progress', () => source as unknown as EventSource);
    return { source, watchInstall };
  }

  it('asks for the install only once the stream is listening, and once', async () => {
    const { source, watchInstall } = watch();
    const start = vi.fn().mockResolvedValue(undefined);
    const done = watchInstall(start);
    expect(start).not.toHaveBeenCalled();
    source.emit('handshake');
    source.emit('handshake');
    expect(start).toHaveBeenCalledTimes(1);
    source.emit('setup', JSON.stringify({ type: 'StepError', error: 'first' }));
    source.emit('setup', JSON.stringify({ type: 'StepError', error: 'second' }));
    source.emit('setup', JSON.stringify({ type: 'Complete' }));
    await expect(done).resolves.toEqual({ error: 'first' });
    expect(source.closed).toBe(true);
  });

  it('reports a refused install request as a failure', async () => {
    const { source, watchInstall } = watch();
    const done = watchInstall(() => Promise.reject(new Error('An install is already running')));
    source.emit('handshake');
    await expect(done).rejects.toThrow('An install is already running');
  });

  it('still asks for the install when the stream fails, and settles with the request', async () => {
    const { source, watchInstall } = watch();
    const start = vi.fn().mockResolvedValue(undefined);
    const done = watchInstall(start);
    source.onerror?.();
    expect(start).toHaveBeenCalledTimes(1);
    await expect(done).resolves.toEqual({ error: null });
  });
});
