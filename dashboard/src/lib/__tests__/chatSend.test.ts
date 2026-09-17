import { describe, expect, it } from 'vitest';
import { buildOutgoingChat } from '../chatSend';

const identity = { id: 'u1', name: 'User' };

describe('what a sent message carries', () => {
  it('names the conversation on both the stored row and the dispatched message', () => {
    const out = buildOutgoingChat({
      messageId: 'm1',
      agentId: 'agent.a',
      conversationId: 'c-42',
      identity,
      contentBlocks: [{ type: 'text', text: 'hello' }],
      hasMedia: false,
    });
    expect(out.stored.conversation_id).toBe('c-42');
    expect(out.dispatched.metadata?.conversation_id).toBe('c-42');
    expect(out.dispatched.metadata?.target_agent_id).toBe('agent.a');
    expect(out.dispatched.content).toBe('hello');
    expect(out.dispatched.metadata?.skip_user_persist).toBeUndefined();
  });

  it('keeps the same conversation across two sends and changes only with the conversation', () => {
    const base = {
      agentId: 'agent.a',
      identity,
      contentBlocks: [{ type: 'text' as const, text: 'x' }],
      hasMedia: false,
    };
    const first = buildOutgoingChat({ ...base, messageId: 'm1', conversationId: 'c-1' });
    const second = buildOutgoingChat({ ...base, messageId: 'm2', conversationId: 'c-1' });
    const elsewhere = buildOutgoingChat({ ...base, messageId: 'm3', conversationId: 'c-2' });
    expect(second.dispatched.metadata?.conversation_id).toBe(first.dispatched.metadata?.conversation_id);
    expect(elsewhere.dispatched.metadata?.conversation_id).not.toBe(first.dispatched.metadata?.conversation_id);
  });

  it('tells the kernel not to store the user turn twice when media rode along', () => {
    const out = buildOutgoingChat({
      messageId: 'm1',
      agentId: 'agent.a',
      conversationId: 'c-1',
      identity,
      contentBlocks: [{ type: 'image', url: 'data:image/png;base64,AA==' }],
      engineOverride: 'deepseek',
      hasMedia: true,
    });
    expect(out.dispatched.metadata?.skip_user_persist).toBe('true');
    expect(out.dispatched.content).toBe('[attachment]');
    expect(out.stored.metadata.engine_override).toBe('deepseek');
    expect(out.dispatched.metadata?.engine_override).toBe('deepseek');
  });
});
