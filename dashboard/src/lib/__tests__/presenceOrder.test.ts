import { describe, expect, it } from 'vitest';
import type { AgentMetadata, Conversation } from '../../types';
import { presenceOrder, stepFace, TURN_THRESHOLD_PX, turnOf } from '../presenceOrder';

const agent = (id: string): AgentMetadata =>
  ({
    id,
    name: id,
    description: '',
    required_capabilities: [],
    enabled: true,
    last_seen: 0,
    status: 'online',
    metadata: {},
  }) as AgentMetadata;
const conv = (agentId: string, updatedAt: number): Conversation => ({
  id: `${agentId}-${updatedAt}`,
  agent_id: agentId,
  user_id: 'u',
  title: '',
  created_at: updatedAt,
  updated_at: updatedAt,
  archived_at: null,
  message_count: 1,
});

describe('the order the new chat turns through', () => {
  it('puts whoever was spoken with most recently first, by their newest conversation', () => {
    const order = presenceOrder(
      [agent('a'), agent('b'), agent('c')],
      [conv('a', 100), conv('c', 300), conv('a', 200), conv('b', 250)],
    );
    expect(order.map((a) => a.id)).toEqual(['c', 'b', 'a']);
  });

  it('keeps the kernel order among agents never spoken with, after everyone who was', () => {
    const order = presenceOrder([agent('x'), agent('y'), agent('z')], [conv('z', 5)]);
    expect(order.map((a) => a.id)).toEqual(['z', 'x', 'y']);
  });

  it('does not reorder the list it was given', () => {
    const given = [agent('a'), agent('b')];
    presenceOrder(given, [conv('b', 1)]);
    expect(given.map((a) => a.id)).toEqual(['a', 'b']);
  });
});

describe('a horizontal movement', () => {
  it('turns to the next face when the content is pulled left, the previous when pulled right', () => {
    expect(turnOf(-TURN_THRESHOLD_PX)).toBe(1);
    expect(turnOf(TURN_THRESHOLD_PX)).toBe(-1);
  });

  it('means nothing until it has gone far enough', () => {
    expect(turnOf(-(TURN_THRESHOLD_PX - 1))).toBe(0);
    expect(turnOf(TURN_THRESHOLD_PX - 1)).toBe(0);
    expect(turnOf(0)).toBe(0);
  });

  it('stops at the ends instead of wrapping', () => {
    expect(stepFace(0, -1, 3)).toBe(0);
    expect(stepFace(2, 1, 3)).toBe(2);
    expect(stepFace(1, 1, 3)).toBe(2);
    expect(stepFace(1, -1, 3)).toBe(0);
    expect(stepFace(0, 1, 1)).toBe(0);
  });
});
