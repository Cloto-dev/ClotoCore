import type { ChatMessage } from '../types';

/**
 * Flatten a branching conversation into a linear display sequence
 * based on the currently active branch selections.
 *
 * Legacy messages (parent_id = null/undefined) are returned in created_at order.
 */
export function flattenConversation(
  messages: ChatMessage[],
  activeBranches: Record<string, number>,
): ChatMessage[] {
  if (messages.length === 0) return [];

  // If all messages lack parent_id, treat as legacy linear chain
  const hasAnyParent = messages.some(m => m.parent_id != null);
  if (!hasAnyParent) return messages;

  // Build parent → children index
  const childrenOf = new Map<string, ChatMessage[]>();
  const roots: ChatMessage[] = [];

  for (const msg of messages) {
    if (msg.parent_id == null) {
      roots.push(msg);
    } else {
      const siblings = childrenOf.get(msg.parent_id) ?? [];
      siblings.push(msg);
      childrenOf.set(msg.parent_id, siblings);
    }
  }

  // Sort roots by created_at
  roots.sort((a, b) => a.created_at - b.created_at);

  // Walk the tree, picking the active branch at each fork
  const result: ChatMessage[] = [];
  const queue = [...roots];

  while (queue.length > 0) {
    const current = queue.shift()!;
    result.push(current);

    const children = childrenOf.get(current.id);
    if (!children || children.length === 0) continue;

    // Group children by source to handle user-edit branches vs agent-response branches
    const grouped = new Map<string, ChatMessage[]>();
    for (const child of children) {
      const key = child.source;
      const group = grouped.get(key) ?? [];
      group.push(child);
      grouped.set(key, group);
    }

    for (const [, group] of grouped) {
      group.sort((a, b) => (a.branch_index ?? 0) - (b.branch_index ?? 0));

      if (group.length === 1) {
        queue.unshift(group[0]);
      } else {
        // Pick active branch (default: highest branch_index = latest)
        const activeIdx = activeBranches[current.id + ':' + group[0].source]
          ?? Math.max(...group.map(m => m.branch_index ?? 0));
        const picked = group.find(m => (m.branch_index ?? 0) === activeIdx) ?? group[group.length - 1];
        queue.unshift(picked);
      }
    }
  }

  return result;
}

/**
 * Find branch points: messages whose parent has multiple same-source siblings.
 * Returns a map of "parentId:source" → { count, activeIndex }
 */
export function findBranchPoints(
  messages: ChatMessage[],
  activeBranches: Record<string, number>,
): Map<string, { parentId: string; source: string; count: number; activeIndex: number; indices: number[] }> {
  const result = new Map<string, { parentId: string; source: string; count: number; activeIndex: number; indices: number[] }>();
  if (messages.length === 0) return result;

  // Group messages by (parent_id, source)
  const groups = new Map<string, ChatMessage[]>();
  for (const msg of messages) {
    if (msg.parent_id == null) continue;
    const key = msg.parent_id + ':' + msg.source;
    const group = groups.get(key) ?? [];
    group.push(msg);
    groups.set(key, group);
  }

  for (const [key, group] of groups) {
    if (group.length < 2) continue;
    const indices = group.map(m => m.branch_index ?? 0).sort((a, b) => a - b);
    const maxIdx = Math.max(...indices);
    const activeIndex = activeBranches[key] ?? maxIdx;
    const parentId = group[0].parent_id!;
    const source = group[0].source;
    result.set(key, { parentId, source, count: group.length, activeIndex, indices });
  }

  return result;
}
