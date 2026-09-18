import { describe, expect, it } from 'vitest';
import { describeWrite } from '../panelWrites';

describe('describeWrite', () => {
  it('puts the chat routes into words, naming the agent', () => {
    expect(describeWrite('POST /api/chat/agent.manager/messages')).toEqual({
      kind: 'send_messages',
      agent: 'agent.manager',
    });
    expect(describeWrite('POST /api/chat/agent.manager/conversations')).toEqual({
      kind: 'start_conversations',
      agent: 'agent.manager',
    });
  });

  it('shows anything else exactly as declared rather than guessing a sentence', () => {
    for (const entry of [
      'PATCH /api/chat/agent.manager/messages',
      'POST /api/chat/agent.manager/messages/extra',
      'POST /api/chat/a/b/messages',
      'POST /api/agents/x/power',
    ]) {
      expect(describeWrite(entry)).toEqual({ kind: 'raw', entry });
    }
  });
});
