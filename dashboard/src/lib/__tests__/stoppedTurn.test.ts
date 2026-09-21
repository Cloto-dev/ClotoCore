import { describe, expect, it } from 'vitest';

import type { ChatMessage } from '../../types';
import { isStoppedMark, shouldWaitForReply } from '../stoppedTurn';

function msg(over: Partial<ChatMessage>): ChatMessage {
  return {
    id: 'x',
    agent_id: 'agent.a',
    user_id: 'u',
    source: 'agent',
    content: [],
    created_at: 0,
    ...over,
  };
}

const fromReader = msg({ source: 'user', content: [{ type: 'text', text: 'hello' }] });
const stopMark = msg({ source: 'system', metadata: { kind: 'stopped', source_message_id: 'x' } });

describe('the mark a stopped turn leaves', () => {
  it('is a system message carrying the kind, and nothing else is', () => {
    expect(isStoppedMark(stopMark)).toBe(true);
    // A notice about something else, on the same source.
    expect(isStoppedMark(msg({ source: 'system', metadata: { kind: 'summary' } }))).toBe(false);
    expect(isStoppedMark(msg({ source: 'system' }))).toBe(false);
  });

  it('is not claimed by a message from the agent or the reader', () => {
    // The kind alone must not be enough: the reader's own message is theirs to
    // write, and a mark it could set would let the page be told a turn was
    // stopped that never was.
    expect(isStoppedMark(msg({ source: 'user', metadata: { kind: 'stopped' } }))).toBe(false);
    expect(isStoppedMark(msg({ source: 'agent', metadata: { kind: 'stopped' } }))).toBe(false);
  });
});

describe('whether a freshly loaded room waits for a reply', () => {
  it('waits when the reader spoke last', () => {
    // The sign of a reply in flight that this room never saw start.
    expect(shouldWaitForReply([fromReader])).toBe(true);
  });

  it('does not wait when the turn was stopped', () => {
    // The defect this replaces: the two shapes end the same way, so reloading
    // brought the waiting state back for a reply that had been called off.
    expect(shouldWaitForReply([fromReader, stopMark])).toBe(false);
  });

  it('does not wait when the agent answered, or when there is nothing yet', () => {
    expect(shouldWaitForReply([fromReader, msg({ source: 'agent' })])).toBe(false);
    expect(shouldWaitForReply([])).toBe(false);
  });
});
