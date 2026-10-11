import type { AuthenticatedApi } from '../services/api';
import type { McpServerInfo } from '../types';

/**
 * Setting up an engine for one partner, as the settings panel runs it after "Use this engine".
 *
 * Every step is the kernel's own API, in an order chosen so that the partner's configuration is
 * the last thing written: the install and the key change no agent, and the grant and the engine
 * choice are only made once the connection test has passed. A wrong key therefore leaves the
 * partner exactly as it was. The key is saved after the install because the provider row that
 * holds it is created by the install.
 */
export type SetupStep = 'install' | 'start' | 'key' | 'test' | 'allow' | 'use';
export type StepState = 'running' | 'done' | 'failed';

export interface EnginePlan {
  agentId: string;
  engineId: string;
  /** The engine is not installed yet. */
  install: boolean;
  /** The engine is installed but not answering. */
  start: boolean;
  /** A new API key to save, or null to keep whatever is stored. */
  key: string | null;
}

export type FailureReason = 'rejected_key' | 'unreachable' | 'not_installed' | 'not_started' | 'timeout' | 'other';

export class EngineSetupError extends Error {
  constructor(
    readonly step: SetupStep,
    readonly reason: FailureReason,
    /** The kernel's own words, shown as given; empty when it said nothing. */
    readonly detail: string,
  ) {
    super(detail || reason);
  }
}

export type EngineSetupApi = Pick<
  AuthenticatedApi,
  | 'installMarketplaceServer'
  | 'listMcpServers'
  | 'restartMcpServer'
  | 'setLlmProviderKey'
  | 'testProviderConnection'
  | 'getAgentServerAccess'
  | 'putAgentServerAccess'
  | 'updateAgent'
>;

export interface EngineSetupDeps {
  api: EngineSetupApi;
  /**
   * Runs `start` (the install request) and settles when the kernel says the install is over,
   * with the first error it reported. Resolving with no error does not mean the server is there:
   * the server list is read afterwards either way.
   */
  watchInstall: (start: () => Promise<void>) => Promise<{ error: string | null }>;
  onStep: (step: SetupStep, state: StepState) => void;
  wait?: (ms: number) => Promise<void>;
}

/** The steps a plan will run, in order — what the panel lists before anything starts. */
export function stepsFor(plan: EnginePlan): SetupStep[] {
  const steps: SetupStep[] = [];
  if (plan.install) steps.push('install');
  else if (plan.start) steps.push('start');
  if (plan.key) steps.push('key');
  steps.push('test', 'allow', 'use');
  return steps;
}

const POLL_MS = 1000;
/** An install that downloads and builds an environment can take minutes. */
const INSTALL_POLLS = 600;
const START_POLLS = 60;

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

async function untilConnected(
  deps: EngineSetupDeps,
  step: SetupStep,
  engineId: string,
  polls: number,
  missing: FailureReason,
): Promise<void> {
  const wait = deps.wait ?? sleep;
  let seen: McpServerInfo | undefined;
  for (let i = 0; i < polls; i++) {
    const { servers } = await deps.api.listMcpServers();
    seen = servers.find((s) => s.id === engineId);
    if (seen?.status === 'Connected') return;
    if (seen?.status === 'Error') throw new EngineSetupError(step, missing, seen.status_message ?? '');
    await wait(POLL_MS);
  }
  throw new EngineSetupError(step, seen ? 'timeout' : missing, '');
}

async function runStep(deps: EngineSetupDeps, plan: EnginePlan, step: SetupStep): Promise<void> {
  const { api } = deps;
  switch (step) {
    case 'install': {
      const { error } = await deps.watchInstall(() =>
        api.installMarketplaceServer({ server_id: plan.engineId, auto_start: true }).then(() => undefined),
      );
      if (error) throw new EngineSetupError(step, 'not_installed', error);
      await untilConnected(deps, step, plan.engineId, INSTALL_POLLS, 'not_installed');
      return;
    }
    case 'start':
      await api.restartMcpServer(plan.engineId);
      await untilConnected(deps, step, plan.engineId, START_POLLS, 'not_started');
      return;
    case 'key':
      await api.setLlmProviderKey(plan.engineId, plan.key ?? '');
      return;
    case 'test': {
      const result = await api.testProviderConnection(plan.engineId);
      // Reachable with the key accepted is enough; some providers do not list their models.
      if (result.status === 'ok' || result.status === 'model_list_unavailable') return;
      throw new EngineSetupError(
        step,
        result.status === 'auth_failed' ? 'rejected_key' : 'unreachable',
        result.error ?? '',
      );
    }
    case 'allow': {
      // Read first and keep the tool answers: the PUT replaces this agent's grants on this server.
      const current = await api.getAgentServerAccess(plan.agentId, plan.engineId);
      if (current.server !== 'allow') {
        await api.putAgentServerAccess(plan.agentId, plan.engineId, { server: 'allow', tools: current.tools });
      }
      return;
    }
    case 'use':
      await api.updateAgent(plan.agentId, { default_engine_id: plan.engineId });
      return;
  }
}

export async function setUpEngine(plan: EnginePlan, deps: EngineSetupDeps): Promise<void> {
  for (const step of stepsFor(plan)) {
    deps.onStep(step, 'running');
    try {
      await runStep(deps, plan, step);
    } catch (err) {
      deps.onStep(step, 'failed');
      if (err instanceof EngineSetupError) throw err;
      throw new EngineSetupError(step, 'other', err instanceof Error ? err.message : String(err));
    }
    deps.onStep(step, 'done');
  }
}

/**
 * `watchInstall` over the kernel's install progress stream. The stream is opened first and the
 * install is requested on its handshake, so no event can be sent before anyone listens. If the
 * stream is unavailable or drops, the install is still requested (or left running) and the
 * caller's server-list polling decides the outcome.
 */
export function watchInstallOver(
  progressUrl: string,
  open: (url: string) => EventSource = (url) => new EventSource(url),
): EngineSetupDeps['watchInstall'] {
  return (start) =>
    new Promise((resolve, reject) => {
      const source = open(progressUrl);
      let started: Promise<void> | null = null;
      let error: string | null = null;
      const request = () => {
        started ??= start();
        return started;
      };
      // Settles only once the request itself has answered, so a refused request is reported
      // as such instead of being left to the polling.
      const finish = () => {
        source.close();
        request().then(() => resolve({ error }), reject);
      };
      source.addEventListener('handshake', () => {
        request().catch(finish);
      });
      source.addEventListener('setup', (event) => {
        let data: { type?: string; error?: string };
        try {
          data = JSON.parse((event as MessageEvent).data);
        } catch {
          return;
        }
        if (data.type === 'StepError' && error === null) error = data.error ?? '';
        if (data.type === 'Complete') finish();
      });
      source.onerror = finish;
    });
}
