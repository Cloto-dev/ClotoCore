import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { AccessControlEntry, McpServerInfo } from '../../../types';

// Echo i18n keys so the assertions do not depend on copy.
vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (k: string, opts?: Record<string, unknown>) =>
      opts?.name ? `${k}:${opts.name}${opts.tool ? `:${opts.tool}` : ''}` : k,
    i18n: { language: 'en' },
  }),
}));

const api = vi.hoisted(() => ({
  apiKey: 'k',
  getMcpServerSettings: vi.fn(),
  getMcpServerTools: vi.fn(),
  getMcpServerAccess: vi.fn(),
  getAgents: vi.fn(),
  updateMcpServerSettings: vi.fn(),
  putMcpServerAccess: vi.fn(),
  getHistory: vi.fn(),
}));
vi.mock('../../../hooks/useApi', () => ({ useApi: () => api }));
vi.mock('../../../hooks/useEventStream', () => ({ useEventStream: () => {} }));
vi.mock('../../../lib/agentIdentity', () => ({ agentColor: () => 'hsl(190 70% 58%)' }));

import { McpServerDetail, withGrant } from '../McpServerDetail';

const server: McpServerInfo = {
  id: 'cpersona',
  display_name: 'CPersona',
  command: 'python',
  args: ['-m', 'cpersona'],
  status: 'Connected',
  tools: ['recall', 'store'],
  is_cloto_sdk: false,
  description: 'Long-term memory.',
  installed_at: 1_756_512_000,
  marketplace_id: 'cpersona',
  installed_version: '2.5.12',
} as McpServerInfo;

const grantSapphy: AccessControlEntry = {
  entry_type: 'server_grant',
  agent_id: 'agent.a',
  server_id: 'cpersona',
  permission: 'allow',
  granted_at: '2026-01-01T00:00:00Z',
};

function draw() {
  return render(
    <McpServerDetail
      server={server}
      onBack={vi.fn()}
      onRefresh={vi.fn()}
      onDelete={vi.fn()}
      onStart={vi.fn()}
      onStop={vi.fn()}
      onRestart={vi.fn()}
    />,
  );
}

const saveButton = () => screen.getByRole('button', { name: 'detail.save' }) as HTMLButtonElement;

beforeEach(() => {
  api.getMcpServerSettings.mockReset().mockResolvedValue({
    server_id: 'cpersona',
    default_policy: 'opt-in',
    config: {},
    env: { CPERSONA_API_KEY: '***', CPERSONA_EMBED_MODEL: 'jina' },
    auto_restart: true,
    command: 'python',
    args: ['-m', 'cpersona'],
  });
  api.getMcpServerTools.mockReset().mockResolvedValue([
    { name: 'recall', description: 'Reads memories back' },
    { name: 'store', description: null },
  ]);
  api.getMcpServerAccess.mockReset().mockResolvedValue({
    server_id: 'cpersona',
    default_policy: 'opt-in',
    tools: ['recall', 'store'],
    entries: [grantSapphy],
  });
  api.getAgents.mockReset().mockResolvedValue([
    { id: 'agent.a', name: 'Sapphy' },
    { id: 'agent.b', name: 'KS22' },
  ]);
  api.updateMcpServerSettings.mockReset().mockResolvedValue({});
  api.putMcpServerAccess.mockReset().mockResolvedValue({});
  api.getHistory.mockReset().mockResolvedValue([]);
});

describe("the server's page", () => {
  it('draws the description, the origin, the tools with their descriptions, and each agent', async () => {
    draw();
    expect(await screen.findByText('Reads memories back')).toBeTruthy();
    expect(screen.getByText('Long-term memory.')).toBeTruthy();
    expect(screen.getByText('detail.origin_marketplace')).toBeTruthy();
    expect(screen.getByText('Sapphy')).toBeTruthy();
    expect(screen.getByText('KS22')).toBeTruthy();
    // Sapphy is allowed the whole server; KS22 follows the default.
    expect(screen.getByText('access.all_tools')).toBeTruthy();
  });

  it('changes nothing on the kernel until save is pressed, then saves what changed', async () => {
    draw();
    await screen.findByText('Reads memories back');
    expect(saveButton().disabled).toBe(true);

    // Policy, an env value, and a grant — three edits, no request.
    fireEvent.click(screen.getByRole('button', { name: 'detail.policy_opt_out' }));
    const values = screen.getAllByLabelText('detail.env_value') as HTMLInputElement[];
    fireEvent.change(values[1], { target: { value: 'jina-v5' } });
    const ks22 = screen.getByRole('group', { name: 'access.server_grant_for:KS22' });
    fireEvent.click(ks22.querySelector('button.deny') as HTMLButtonElement);
    expect(api.updateMcpServerSettings).not.toHaveBeenCalled();
    expect(api.putMcpServerAccess).not.toHaveBeenCalled();
    expect(saveButton().disabled).toBe(false);

    fireEvent.click(saveButton());
    await vi.waitFor(() => expect(api.updateMcpServerSettings).toHaveBeenCalledTimes(1));
    expect(api.updateMcpServerSettings).toHaveBeenCalledWith('cpersona', {
      default_policy: 'opt-out',
      env: { CPERSONA_API_KEY: '***', CPERSONA_EMBED_MODEL: 'jina-v5' },
    });
    await vi.waitFor(() => expect(api.putMcpServerAccess).toHaveBeenCalledTimes(1));
    const saved = api.putMcpServerAccess.mock.calls[0][1] as AccessControlEntry[];
    expect(saved).toHaveLength(2);
    expect(saved.find((e) => e.agent_id === 'agent.b')).toMatchObject({
      entry_type: 'server_grant',
      permission: 'deny',
    });
  });

  it('saves only the part that changed', async () => {
    draw();
    await screen.findByText('Reads memories back');
    const sapphy = screen.getByRole('group', { name: 'access.server_grant_for:Sapphy' });
    fireEvent.click(sapphy.querySelector('button:not(.allow):not(.deny)') as HTMLButtonElement);
    fireEvent.click(saveButton());
    await vi.waitFor(() => expect(api.putMcpServerAccess).toHaveBeenCalledTimes(1));
    expect(api.updateMcpServerSettings).not.toHaveBeenCalled();
    expect(api.putMcpServerAccess.mock.calls[0][1]).toEqual([]);
  });

  it('a policy change alone does not rewrite the access entries', async () => {
    draw();
    await screen.findByText('Reads memories back');
    fireEvent.click(screen.getByRole('button', { name: 'detail.policy_opt_out' }));
    fireEvent.click(saveButton());
    await vi.waitFor(() => expect(api.updateMcpServerSettings).toHaveBeenCalledTimes(1));
    expect(api.putMcpServerAccess).not.toHaveBeenCalled();
  });

  it('discard puts the edits back and saves nothing', async () => {
    draw();
    await screen.findByText('Reads memories back');
    fireEvent.click(screen.getByRole('button', { name: 'detail.policy_opt_out' }));
    fireEvent.click(screen.getByRole('button', { name: 'detail.discard' }));
    expect(saveButton().disabled).toBe(true);
    expect((screen.getByRole('button', { name: 'detail.policy_opt_in' }) as HTMLButtonElement).className).toContain(
      'on',
    );
    expect(api.updateMcpServerSettings).not.toHaveBeenCalled();
  });

  it('opens an agent to grant tool by tool', async () => {
    draw();
    await screen.findByText('Reads memories back');
    expect(screen.queryByRole('group', { name: 'access.tool_grant_for:KS22:recall' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'KS22' }));
    const recall = screen.getByRole('group', { name: 'access.tool_grant_for:KS22:recall' });
    fireEvent.click(recall.querySelector('button.allow') as HTMLButtonElement);
    expect(screen.getByText('access.overrides')).toBeTruthy();
    fireEvent.click(saveButton());
    await vi.waitFor(() => expect(api.putMcpServerAccess).toHaveBeenCalledTimes(1));
    const saved = api.putMcpServerAccess.mock.calls[0][1] as AccessControlEntry[];
    expect(saved.find((e) => e.entry_type === 'tool_grant')).toMatchObject({
      agent_id: 'agent.b',
      tool_name: 'recall',
      permission: 'allow',
    });
  });
});

describe('a grant', () => {
  it('replaces the matching entry and leaves the others', () => {
    const toolGrant: AccessControlEntry = {
      ...grantSapphy,
      entry_type: 'tool_grant',
      tool_name: 'store',
      permission: 'deny',
    };
    const next = withGrant([grantSapphy, toolGrant], 'agent.a', 'cpersona', null, 'deny');
    expect(next).toHaveLength(2);
    expect(next.find((e) => e.entry_type === 'server_grant')?.permission).toBe('deny');
    expect(next.find((e) => e.entry_type === 'tool_grant')?.permission).toBe('deny');
    // Back to default removes the entry rather than writing "inherit".
    expect(withGrant(next, 'agent.a', 'cpersona', 'store', 'inherit')).toHaveLength(1);
  });
});
