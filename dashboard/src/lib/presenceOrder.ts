import type { AgentMetadata, Conversation } from '../types';

/**
 * The order the new chat turns through: whoever was spoken with most recently
 * first, then the agents never spoken with, in the order the kernel lists them.
 * The "create an agent" face is not an agent and is not in this list — it is
 * always the first face, to the left of everyone.
 */
export function presenceOrder(agents: AgentMetadata[], conversations: Conversation[]): AgentMetadata[] {
  const lastSpoken = new Map<string, number>();
  for (const c of conversations) {
    const seen = lastSpoken.get(c.agent_id);
    if (seen === undefined || c.updated_at > seen) lastSpoken.set(c.agent_id, c.updated_at);
  }
  // Array#sort is stable, so agents with no conversation keep the kernel's order.
  return [...agents].sort((a, b) => (lastSpoken.get(b.id) ?? -1) - (lastSpoken.get(a.id) ?? -1));
}

/** How far a drag or a swipe has to travel, in px, before it turns the face. */
export const TURN_THRESHOLD_PX = 60;

/**
 * Which way a horizontal movement turns the faces: `1` is the next face (the
 * content was pulled to the left), `-1` the previous one, `0` not far enough
 * to mean anything.
 */
export function turnOf(dx: number): -1 | 0 | 1 {
  if (dx <= -TURN_THRESHOLD_PX) return 1;
  if (dx >= TURN_THRESHOLD_PX) return -1;
  return 0;
}

/** The face `step` away from `index` among `count` faces. It does not wrap:
 * the ends are ends, so "create an agent" stays where the row begins. */
export function stepFace(index: number, step: -1 | 0 | 1, count: number): number {
  return Math.min(Math.max(index + step, 0), Math.max(count - 1, 0));
}
