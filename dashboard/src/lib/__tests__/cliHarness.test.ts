import { describe, expect, it } from 'vitest';
import type { McpServerInfo } from '../../types';
import { buildEnvUpdate, findHarnessServer, isMeteredPlan, PROBE_TOOL, parseProbeResult } from '../cliHarness';

const server = (id: string, tools: string[]) => ({ id, tools }) as unknown as McpServerInfo;

const probeResult = (payload: unknown) => ({
  content: [{ type: 'text', text: JSON.stringify(payload) }],
});

describe('findHarnessServer', () => {
  it('finds the connector by its tool surface, not its id', () => {
    const servers = [
      server('deepseek', ['think', 'think_with_tools']),
      server('renamed-by-the-operator', ['think', PROBE_TOOL]),
    ];
    expect(findHarnessServer(servers)?.id).toBe('renamed-by-the-operator');
  });

  it('is undefined when no connected server can run a harness', () => {
    expect(findHarnessServer([server('deepseek', ['think'])])).toBeUndefined();
  });
});

describe('parseProbeResult', () => {
  it('reads the harness list out of the tool result', () => {
    const probe = parseProbeResult(
      probeResult({
        harnesses: [{ id: 'claude', installed: true, version: '2.1.0', plan: 'subscription' }],
        configured_harness: null,
        active_harness: 'claude',
        working_directory: '/srv',
      }),
    );
    expect(probe.harnesses).toHaveLength(1);
    expect(probe.harnesses[0].plan).toBe('subscription');
    expect(probe.active_harness).toBe('claude');
    expect(probe.configured_harness).toBeNull();
  });

  it('keeps a field absent rather than inventing a value for it', () => {
    // The connector omits what it could not read. "installed: undefined" has to
    // stay undefined so the screen can say "unknown" instead of "no".
    const probe = parseProbeResult(probeResult({ harnesses: [{ id: 'codex' }] }));
    expect(probe.harnesses[0].installed).toBeUndefined();
    expect(probe.harnesses[0].plan).toBeUndefined();
  });

  it('throws instead of reporting an empty host', () => {
    // Each of these once looked like "no harnesses found" to a naive reader.
    expect(() => parseProbeResult(null)).toThrow();
    expect(() => parseProbeResult({ content: [] })).toThrow();
    expect(() => parseProbeResult({ content: [{ type: 'text', text: 'not json' }] })).toThrow();
    expect(() => parseProbeResult(probeResult({ configured_harness: null }))).toThrow(/harness list/);
    expect(() => parseProbeResult({ ...probeResult({ harnesses: [] }), is_error: true })).toThrow();
    expect(() => parseProbeResult(probeResult({ error: 'binary vanished' }))).toThrow(/binary vanished/);
  });

  it('accepts a host where nothing is installed', () => {
    // Distinct from the failures above: this is a real, readable answer.
    expect(parseProbeResult(probeResult({ harnesses: [] })).harnesses).toEqual([]);
  });
});

describe('buildEnvUpdate', () => {
  it('names every stored key so the merge does not drop the untouched ones', () => {
    // The kernel rebuilds env from what it receives: a key left out is deleted.
    const out = buildEnvUpdate({ A: 'one', B: 'two', C: 'three' }, { B: 'changed' });
    expect(out).toEqual({ A: '***', B: 'changed', C: '***' });
  });

  it('passes an empty string through, which is how a key is removed', () => {
    expect(buildEnvUpdate({ A: 'one' }, { A: '' })).toEqual({ A: '' });
  });

  it('carries a key that is new to the server', () => {
    expect(buildEnvUpdate({ A: 'one' }, { B: 'two' })).toEqual({ A: '***', B: 'two' });
  });

  it('changes nothing when nothing was edited', () => {
    expect(buildEnvUpdate({ A: 'one', B: 'two' }, {})).toEqual({ A: '***', B: '***' });
  });
});

describe('isMeteredPlan', () => {
  it('flags an API key and nothing else', () => {
    expect(isMeteredPlan('api_key')).toBe(true);
    expect(isMeteredPlan('subscription')).toBe(false);
    // "unknown" means the connector could not read the account. Warning on it
    // would put an unmeasured claim on screen.
    expect(isMeteredPlan('unknown')).toBe(false);
    expect(isMeteredPlan(undefined)).toBe(false);
  });
});
