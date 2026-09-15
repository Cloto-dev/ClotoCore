import type { McpServerInfo } from '../types';

/** Reading an external CLI harness engine, without hard-coding which server it is.
 *
 * The connector that runs a harness is found the way every other capability in
 * this dashboard is found — by the tools a server exposes, never by its id
 * (ARCHITECTURE §1.2). Its settable options are read from the catalog's
 * `optional_env_vars`, so a connector that adds one shows up here without a
 * dashboard release, and nothing in this file has to know what any of them mean.
 */

/** The tool that reports which harnesses exist on the host. A server exposing
 * it is, by definition, the one that can run them. */
export const PROBE_TOOL = 'probe_harnesses';

/** One harness the connector found (or did not find) on the kernel's host.
 *
 * Every field past `id` is optional on purpose: the connector reports what it
 * could read and omits the rest, because a caller that renders a missing value
 * as "not installed" or "logged out" would be inventing a fact. */
export interface HarnessEntry {
  id: string;
  binary?: string;
  installed?: boolean;
  path?: string | null;
  version?: string | null;
  /** Where the credential lives (`keychain` / `file` / `unknown`). Never the credential. */
  credential_store?: string;
  /** `subscription` / `api_key` / `unknown` — how a run would be billed. */
  plan?: string;
  auth_mode?: string;
  api_key_configured?: boolean;
  last_refresh?: string;
}

export interface HarnessProbe {
  harnesses: HarnessEntry[];
  /** What the connector was configured to use, or null for "first one installed". */
  configured_harness: string | null;
  /** What it would actually run right now. */
  active_harness: string | null;
  working_directory: string;
  /** Connector-owned editor for settings stored on one agent. */
  agent_config?: AgentConfigSchema;
}

export interface AgentConfigField {
  key: string;
  label: string;
  description?: string;
  input: 'text' | 'select';
  options?: string[];
  default?: string;
}

export interface AgentConfigSchema {
  /** Key in the kernel's string-valued agent metadata map. */
  metadata_key: string;
  fields: AgentConfigField[];
}

/** The connector that can run CLI harnesses, or undefined when none is installed. */
export function findHarnessServer(servers: readonly McpServerInfo[]): McpServerInfo | undefined {
  return servers.find((s) => s.tools.includes(PROBE_TOOL));
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

/**
 * Read a probe out of an MCP `CallToolResult`.
 *
 * The kernel returns the tool result verbatim — `{content: [{type, text}], is_error?}`
 * — and the connector puts its JSON in the first text block. Throws rather than
 * returning a half-filled probe: a screen that renders "no harnesses found"
 * because it could not parse the answer is worse than one that says it failed.
 */
export function parseProbeResult(raw: unknown): HarnessProbe {
  if (!isRecord(raw)) throw new Error('Harness probe returned no result');
  if (raw.is_error === true) throw new Error('The connector reported an error probing harnesses');

  const content = Array.isArray(raw.content) ? raw.content : [];
  const text = content
    .map((block) => (isRecord(block) && typeof block.text === 'string' ? block.text : ''))
    .find((t) => t.length > 0);
  if (!text) throw new Error('Harness probe returned no text content');

  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    throw new Error('Harness probe returned text that is not JSON');
  }
  if (!isRecord(parsed)) throw new Error('Harness probe returned a non-object');
  if (typeof parsed.error === 'string') throw new Error(parsed.error);
  if (!Array.isArray(parsed.harnesses)) throw new Error('Harness probe carried no harness list');

  let agentConfig: AgentConfigSchema | undefined;
  if (parsed.agent_config !== undefined) {
    if (
      !isRecord(parsed.agent_config) ||
      Array.isArray(parsed.agent_config) ||
      typeof parsed.agent_config.metadata_key !== 'string'
    ) {
      throw new Error('Harness probe carried an invalid agent configuration schema');
    }
    const fields = Array.isArray(parsed.agent_config.fields) ? parsed.agent_config.fields : [];
    agentConfig = {
      metadata_key: parsed.agent_config.metadata_key,
      fields: fields.map((field) => {
        if (
          !isRecord(field) ||
          typeof field.key !== 'string' ||
          typeof field.label !== 'string' ||
          (field.input !== 'text' && field.input !== 'select')
        ) {
          throw new Error('Harness probe carried an invalid agent configuration field');
        }
        return {
          key: field.key,
          label: field.label,
          input: field.input,
          ...(typeof field.description === 'string' && { description: field.description }),
          ...(typeof field.default === 'string' && { default: field.default }),
          ...(Array.isArray(field.options) && {
            options: field.options.filter((option): option is string => typeof option === 'string'),
          }),
        };
      }),
    };
  }

  return {
    harnesses: parsed.harnesses.filter(isRecord).map((h) => h as unknown as HarnessEntry),
    configured_harness: typeof parsed.configured_harness === 'string' ? parsed.configured_harness : null,
    active_harness: typeof parsed.active_harness === 'string' ? parsed.active_harness : null,
    working_directory: typeof parsed.working_directory === 'string' ? parsed.working_directory : '',
    ...(agentConfig && { agent_config: agentConfig }),
  };
}

/** Decode a connector-owned object from the kernel's string-valued metadata. */
export function parseAgentConfig(raw: string | undefined): Record<string, string> {
  if (!raw) return {};
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    throw new Error('The saved agent binding is not valid JSON');
  }
  if (!isRecord(parsed) || Array.isArray(parsed)) throw new Error('The saved agent binding is not an object');

  const out: Record<string, string> = {};
  for (const [key, value] of Object.entries(parsed)) {
    if (typeof value !== 'string') throw new Error(`The saved agent binding field "${key}" is not text`);
    out[key] = value;
  }
  return out;
}

/** Merge edited connector-owned fields without dropping other agent metadata. */
export function buildAgentMetadataUpdate(
  metadata: Readonly<Record<string, string>>,
  metadataKey: string,
  edits: Readonly<Record<string, string>>,
): Record<string, string> {
  const next = { ...metadata };
  const binding = parseAgentConfig(metadata[metadataKey]);
  for (const [key, value] of Object.entries(edits)) {
    const normalized = value.trim();
    if (normalized) binding[key] = normalized;
    else delete binding[key];
  }
  if (Object.keys(binding).length > 0) next[metadataKey] = JSON.stringify(binding);
  else delete next[metadataKey];
  return next;
}

/**
 * Build the `env` object to send when saving.
 *
 * The kernel merges by rebuilding from what it is sent: a key absent from the
 * body is dropped from the server's environment, `"***"` keeps the stored value
 * and an empty string removes the key (`handlers/mcp.rs`, update path). So every
 * key that should survive has to be named on every save — sending only the
 * edited ones silently wipes the rest, which is exactly the bug this function
 * exists to make impossible.
 *
 * @param stored keys currently held by the server
 * @param edits values the operator changed; `''` means "remove this key"
 */
export function buildEnvUpdate(
  stored: Readonly<Record<string, string>>,
  edits: Readonly<Record<string, string>>,
): Record<string, string> {
  const out: Record<string, string> = {};
  for (const key of Object.keys(stored)) {
    out[key] = '***';
  }
  for (const [key, value] of Object.entries(edits)) {
    out[key] = value;
  }
  return out;
}

/** Whether a plan string means "this run is billed per token".
 *
 * Only `api_key` is treated as metered. `unknown` is left alone: the connector
 * says unknown when it could not read the account, and turning that into a
 * warning would put a claim on screen that nothing measured. */
export function isMeteredPlan(plan: string | undefined): boolean {
  return plan === 'api_key';
}
