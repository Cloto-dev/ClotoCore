import { createContext, type ReactNode, useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { useApi } from '../hooks/useApi';
import { rememberConversation, rememberedConversation } from '../lib/conversations';
import type { ContentBlock, Conversation } from '../types';
import { useAgentContext } from './AgentContext';
import { useUserIdentity } from './UserIdentityContext';

/**
 * The conversations the person can see (docs/CONVERSATIONS_DESIGN.md), across
 * every agent, and which one is open per agent. The kernel owns the rows;
 * this holds the list the sidebar draws and the choice the console reads.
 */
interface ConversationContextValue {
  /** Live (unarchived) conversations of every agent, newest activity first. */
  conversations: Conversation[];
  loading: boolean;
  refresh: () => Promise<void>;
  /** The conversation open for an agent, once one has been chosen. */
  openFor: (agentId: string) => string | null;
  /** Choose the conversation to show for an agent: the remembered one if it
   * still exists, else the newest, else a draft. Answers `null` for a draft. */
  resolveOpen: (agentId: string) => Promise<string | null>;
  /** Open a conversation: select its agent and show it. */
  open: (agentId: string, conversationId: string) => void;
  /**
   * The new chat, while it is only that: a screen to choose who to talk to and
   * say the first thing. Nothing exists for it — in the kernel or in the
   * sidebar — until that first thing is said; leaving leaves nothing behind.
   * There is one at a time. `null` when no new chat is open.
   */
  draft: Draft | null;
  /** Open the new chat, on this agent (or on "create an agent" with `null`). */
  startDraft: (agentId: string | null) => void;
  /** Turn the new chat to another agent, or to "create an agent" with `null`.
   * The same draft: what was typed stays. */
  setDraftAgent: (agentId: string | null) => void;
  /** The first message was written: the console takes over and sends it. */
  sendDraft: (first: FirstMessage) => void;
  /** The console is sending the draft's first message: create its
   * conversation and answer the id. The console stays mounted across this. */
  commitDraft: (agentId: string) => Promise<string>;
  /** Close the new chat without saying anything. */
  leaveDraft: () => void;
  /** The key to mount a conversation's console under. A conversation that began
   * as a draft keeps the draft's key, so creating it does not remount the
   * console in the middle of its first exchange. */
  mountKeyFor: (agentId: string, conversationId: string) => string;
  rename: (agentId: string, conversationId: string, title: string) => Promise<void>;
  archive: (agentId: string, conversationId: string, archived: boolean) => Promise<void>;
  remove: (agentId: string, conversationId: string) => Promise<void>;
}

/** What the person wrote to begin a conversation, as the composer hands it over. */
export interface FirstMessage {
  blocks: ContentBlock[];
  rawText: string;
  engineOverride: string | null;
}

export interface Draft {
  /** What the console is mounted under, before and after the conversation exists. */
  key: string;
  /** Who the new chat is turned to; `null` is the "create an agent" face. */
  agentId: string | null;
  /** Set once the first message is written: the console sends it on mount. */
  first?: FirstMessage;
}

const ConversationContext = createContext<ConversationContextValue | null>(null);

export function ConversationProvider({ children }: { children: ReactNode }) {
  const api = useApi();
  const navigate = useNavigate();
  const { identity } = useUserIdentity();
  const { agents, setSelectedAgentId, setSystemActive } = useAgentContext();
  const [conversations, setConversations] = useState<Conversation[]>([]);
  const [loading, setLoading] = useState(true);
  const [openIds, setOpenIds] = useState<Record<string, string>>({});
  const creating = useRef<Map<string, Promise<Conversation>>>(new Map());
  const [draft, setDraft] = useState<Draft | null>(null);
  const draftSeq = useRef(0);
  const mountKeys = useRef<Map<string, string>>(new Map());

  const agentIds = useMemo(() => agents.map((a) => a.id).join('\n'), [agents]);

  const refresh = useCallback(async () => {
    const ids = agentIds ? agentIds.split('\n') : [];
    if (ids.length === 0) {
      setConversations([]);
      setLoading(false);
      return;
    }
    const lists = await Promise.all(
      ids.map((id) =>
        api.listConversations(id, identity.id).catch((err) => {
          if (import.meta.env.DEV) console.warn('Failed to list conversations:', err);
          return [] as Conversation[];
        }),
      ),
    );
    setConversations(lists.flat().sort((a, b) => b.updated_at - a.updated_at));
    setLoading(false);
  }, [agentIds, api, identity.id]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const setOpen = useCallback((agentId: string, conversationId: string) => {
    rememberConversation(agentId, conversationId);
    setOpenIds((prev) => (prev[agentId] === conversationId ? prev : { ...prev, [agentId]: conversationId }));
  }, []);

  const openFor = useCallback((agentId: string) => openIds[agentId] ?? null, [openIds]);

  const leaveDraft = useCallback(() => setDraft(null), []);

  const beginDraft = useCallback((agentId: string | null) => {
    draftSeq.current += 1;
    // A fresh key every time: pressing New chat on a half-typed draft is
    // asking for an empty one.
    setDraft({ key: `draft:${draftSeq.current}`, agentId });
  }, []);

  /** Make an agent the one present and keep the address bar in step. */
  const present = useCallback(
    (agentId: string | null, replace: boolean) => {
      setSystemActive(false);
      setSelectedAgentId(agentId);
      navigate(agentId ? `/?agent=${encodeURIComponent(agentId)}` : '/', { replace });
    },
    [navigate, setSelectedAgentId, setSystemActive],
  );

  const startDraft = useCallback(
    (agentId: string | null) => {
      beginDraft(agentId);
      present(agentId, false);
    },
    [beginDraft, present],
  );

  const setDraftAgent = useCallback(
    (agentId: string | null) => {
      setDraft((prev) => (prev && !prev.first ? { ...prev, agentId } : prev));
      present(agentId, true);
    },
    [present],
  );

  const sendDraft = useCallback((first: FirstMessage) => {
    // Only a draft turned to someone can be sent.
    setDraft((prev) => (prev?.agentId ? { ...prev, first } : prev));
  }, []);

  const commitDraft = useCallback(
    async (agentId: string) => {
      // One creation per agent at a time: a double send must not mint two.
      let pending = creating.current.get(agentId);
      if (!pending) {
        pending = api.createConversation(agentId, identity.id).finally(() => creating.current.delete(agentId));
        creating.current.set(agentId, pending);
      }
      const created = await pending;
      if (draft) mountKeys.current.set(created.id, draft.key);
      setConversations((prev) => [created, ...prev.filter((c) => c.id !== created.id)]);
      setOpen(agentId, created.id);
      setDraft(null);
      return created.id;
    },
    [api, draft, identity.id, setOpen],
  );

  const mountKeyFor = useCallback(
    (agentId: string, conversationId: string) =>
      mountKeys.current.get(conversationId) ?? `${agentId}:${conversationId}`,
    [],
  );

  const resolveOpen = useCallback(
    async (agentId: string) => {
      const current = openIds[agentId];
      const live = await api.listConversations(agentId, identity.id).catch(() => [] as Conversation[]);
      const exists = (id: string | null | undefined) => !!id && live.some((c) => c.id === id);
      if (exists(current)) return current as string;
      const remembered = rememberedConversation(agentId);
      if (exists(remembered)) {
        setOpen(agentId, remembered as string);
        return remembered as string;
      }
      if (live.length > 0) {
        setOpen(agentId, live[0].id);
        return live[0].id;
      }
      // Nobody has spoken with this agent yet. That is a draft, not a row:
      // nothing is created until something is said.
      beginDraft(agentId);
      return null;
    },
    [api, beginDraft, identity.id, openIds, setOpen],
  );

  const open = useCallback(
    (agentId: string, conversationId: string) => {
      setOpen(agentId, conversationId);
      // Choosing a conversation leaves the new chat: it held nothing to keep.
      setDraft(null);
      setSystemActive(false);
      setSelectedAgentId(agentId);
      navigate(`/?agent=${encodeURIComponent(agentId)}`);
    },
    [navigate, setOpen, setSelectedAgentId, setSystemActive],
  );

  const rename = useCallback(
    async (agentId: string, conversationId: string, title: string) => {
      const updated = await api.updateConversation(agentId, conversationId, { title });
      setConversations((prev) => prev.map((c) => (c.id === conversationId ? { ...c, title: updated.title } : c)));
    },
    [api],
  );

  const archive = useCallback(
    async (agentId: string, conversationId: string, archived: boolean) => {
      await api.updateConversation(agentId, conversationId, { archived });
      await refresh();
    },
    [api, refresh],
  );

  const remove = useCallback(
    async (agentId: string, conversationId: string) => {
      await api.deleteConversation(agentId, conversationId);
      setConversations((prev) => prev.filter((c) => c.id !== conversationId));
      setOpenIds((prev) => {
        if (prev[agentId] !== conversationId) return prev;
        const next = { ...prev };
        delete next[agentId];
        rememberConversation(agentId, null);
        return next;
      });
    },
    [api],
  );

  const value = useMemo(
    () => ({
      conversations,
      loading,
      refresh,
      openFor,
      resolveOpen,
      open,
      draft,
      startDraft,
      setDraftAgent,
      sendDraft,
      commitDraft,
      leaveDraft,
      mountKeyFor,
      rename,
      archive,
      remove,
    }),
    [
      conversations,
      loading,
      refresh,
      openFor,
      resolveOpen,
      open,
      draft,
      startDraft,
      setDraftAgent,
      sendDraft,
      commitDraft,
      leaveDraft,
      mountKeyFor,
      rename,
      archive,
      remove,
    ],
  );

  return <ConversationContext.Provider value={value}>{children}</ConversationContext.Provider>;
}

export function useConversations() {
  const ctx = useContext(ConversationContext);
  if (!ctx) throw new Error('useConversations must be used within ConversationProvider');
  return ctx;
}
