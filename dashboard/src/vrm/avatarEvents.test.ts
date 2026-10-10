import { describe, expect, it, vi } from 'vitest';
import { applyAvatarEvent } from './avatarEvents';
import type { VrmAnimationController } from './engine/VrmAnimationController';

function controller() {
  return { setExpression: vi.fn(), setPose: vi.fn(), stopVisemesSafe: vi.fn(), playVisemes: vi.fn() };
}
describe('shared avatar event bridge', () => {
  it('routes expression and pose notifications only to their owning partner', () => {
    const c = controller();
    const state = vi.fn();
    const apply = (agent_id: string, channel: string, data: Record<string, unknown>) =>
      applyAvatarEvent(
        { type: 'McpNotification', data: { params: { channel, data: { agent_id, ...data } } } },
        'agent.mio',
        c as unknown as VrmAnimationController,
        state,
      );
    apply('agent.other', 'avatar_set_expression', { expression: 'happy', intensity: 0.4 });
    expect(c.setExpression).not.toHaveBeenCalled();
    apply('agent.mio', 'avatar_set_expression', { expression: 'happy', intensity: 0.4 });
    expect(c.setExpression).toHaveBeenCalledWith('happy', 0.4);
    apply('agent.mio', 'avatar_set_pose', { pose: 'thinking', transition: 0.7 });
    expect(c.setPose).toHaveBeenCalledWith('thinking', 0.7);
  });
  it('transitions through thinking, response and stop while ignoring another agent', () => {
    const c = controller();
    const state = vi.fn();
    for (const type of ['AgentThinking', 'ThoughtResponse', 'ResponseStopped'])
      applyAvatarEvent(
        { type, data: { agent_id: 'agent.mio' } },
        'agent.mio',
        c as unknown as VrmAnimationController,
        state,
      );
    applyAvatarEvent(
      { type: 'AgentThinking', data: { agent_id: 'agent.other' } },
      'agent.mio',
      c as unknown as VrmAnimationController,
      state,
    );
    expect(state.mock.calls).toEqual([['thinking'], ['responding'], ['idle']]);
    expect(c.stopVisemesSafe).toHaveBeenCalledOnce();
  });
});
