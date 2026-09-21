import type { ChatMessage } from '../types';

/** The mark the kernel stores where a reply was stopped (handlers/chat.rs). */
export const STOPPED_MESSAGE_KIND = 'stopped';

/**
 * Whether this message is the mark that says a turn was stopped.
 *
 * The mark carries no text: the kernel stores what happened, and the wording is
 * the reader's, so the line appears in the reader's language.
 */
export function isStoppedMark(msg: Pick<ChatMessage, 'source' | 'metadata'>): boolean {
  return msg.source === 'system' && msg.metadata?.kind === STOPPED_MESSAGE_KIND;
}

/**
 * Whether the room should show the agent as still working, judged from the
 * history alone when it is first loaded.
 *
 * A reply may be in flight that this room never saw start, and the sign of it
 * is a message from the reader with nothing after it. A turn that was stopped
 * also ends with nothing from the agent — which is why, while the stop lived
 * only in the page, reloading brought the waiting state back for a reply that
 * had been called off. The mark is what tells the two apart.
 */
export function shouldWaitForReply(messages: readonly ChatMessage[]): boolean {
  const last = messages[messages.length - 1];
  return !!last && last.source === 'user';
}
