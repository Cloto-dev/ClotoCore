import { createContext, type ReactNode, useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { useApi } from '../hooks/useApi';
import { rememberConversation, rememberedConversation } from '../lib/conversations';
import type { Conversation } from '../types';
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
   * still exists, else the newest, else a new one. */
  resolveOpen: (agentId: string) => Promise<string>;
  /** Open a conversation: select its agent and show it. */
  open: (agentId: string, conversationId: string) => void;
  newChat: (agentId: string) => Promise<Conversation>;
  rename: (agentId: string, conversationId: string, title: string) => Promise<void>;
  archive: (agentId: string, conversationId: string, archived: boolean) => Promise<void>;
  remove: (agentId: string, conversationId: string) => Promise<void>;
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

  const newChat = useCallback(
    async (agentId: string) => {
      // An empty conversation is already a new chat: reuse it rather than
      // minting another, so pressing the button twice leaves one, not two.
      const empty = conversations.find(
        (c) => c.agent_id === agentId && c.message_count === 0 && c.archived_at === null,
      );
      if (empty) {
        setOpen(agentId, empty.id);
        return empty;
      }
      const created = await api.createConversation(agentId, identity.id);
      setConversations((prev) => [created, ...prev.filter((c) => c.id !== created.id)]);
      setOpen(agentId, created.id);
      return created;
    },
    [api, conversations, identity.id, setOpen],
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
      // One creation per agent at a time: two mounts must not mint two.
      let pending = creating.current.get(agentId);
      if (!pending) {
        pending = newChat(agentId).finally(() => creating.current.delete(agentId));
        creating.current.set(agentId, pending);
      }
      return (await pending).id;
    },
    [api, identity.id, newChat, openIds, setOpen],
  );

  const open = useCallback(
    (agentId: string, conversationId: string) => {
      setOpen(agentId, conversationId);
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
    () => ({ conversations, loading, refresh, openFor, resolveOpen, open, newChat, rename, archive, remove }),
    [conversations, loading, refresh, openFor, resolveOpen, open, newChat, rename, archive, remove],
  );

  return <ConversationContext.Provider value={value}>{children}</ConversationContext.Provider>;
}

export function useConversations() {
  const ctx = useContext(ConversationContext);
  if (!ctx) throw new Error('useConversations must be used within ConversationProvider');
  return ctx;
}
