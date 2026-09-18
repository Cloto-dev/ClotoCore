// What the dashboard sends when a person speaks (docs/CONVERSATIONS_DESIGN.md
// §2b): the stored row and the kernel message both name the conversation, so
// the kernel files the exchange under it and reads it back as the model's
// context. Pure, so the shape can be pinned without mounting the console.

import type { ClotoMessage, ContentBlock } from '../types';

export interface OutgoingChat {
  /** For `POST /api/chat/{agent}/messages`, used when the content has media. */
  stored: {
    id: string;
    source: 'user';
    content: ContentBlock[];
    metadata: Record<string, unknown>;
    conversation_id: string;
  };
  /** For `POST /api/chat`: what the kernel dispatches. */
  dispatched: ClotoMessage;
}

export function buildOutgoingChat(args: {
  messageId: string;
  agentId: string;
  conversationId: string;
  identity: { id: string; name: string };
  contentBlocks: ContentBlock[];
  engineOverride?: string | null;
  hasMedia: boolean;
}): OutgoingChat {
  const { messageId, agentId, conversationId, identity, contentBlocks, engineOverride, hasMedia } = args;
  const textContent = contentBlocks
    .filter((b) => b.type === 'text')
    .map((b) => b.text || '')
    .join(' ');
  return {
    stored: {
      id: messageId,
      source: 'user',
      content: contentBlocks,
      metadata: {
        user_id: identity.id,
        user_name: identity.name,
        ...(engineOverride ? { engine_override: engineOverride } : {}),
      },
      conversation_id: conversationId,
    },
    dispatched: {
      id: messageId,
      source: { type: 'User', id: identity.id, name: identity.name },
      target_agent: agentId,
      content: textContent || '[attachment]',
      timestamp: new Date().toISOString(),
      metadata: {
        target_agent_id: agentId,
        conversation_id: conversationId,
        ...(engineOverride ? { engine_override: engineOverride } : {}),
        // The stored row already exists when media rode along; tell the
        // kernel not to persist the user message a second time.
        ...(hasMedia ? { skip_user_persist: 'true' } : {}),
      },
    },
  };
}
