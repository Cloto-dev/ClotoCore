import { useCallback, useEffect, useRef, useState } from 'react';
import { useApi } from '../hooks/useApi';
import { useEventStream } from '../hooks/useEventStream';
import { EVENTS_URL } from '../services/api';

export type PresenceState = 'offline' | 'asking' | 'thinking' | 'waiting';

/**
 * What the room says about the partner. An unreachable kernel outranks
 * everything, because nothing else can be known then; a question waiting for
 * an answer outranks thinking, because the partner is stopped until it is
 * answered. The first health check has not answered while `checking` is true,
 * and that is not reported as a lost connection.
 */
export function presenceState(input: {
  connected: boolean;
  checking: boolean;
  asking: boolean;
  thinking: boolean;
}): PresenceState {
  if (!input.checking && !input.connected) return 'offline';
  if (input.asking) return 'asking';
  if (input.thinking) return 'thinking';
  return 'waiting';
}

/**
 * How many approvals this partner is holding for an answer: the stored
 * blocking approvals at first, then kept current by the stream. This is the
 * same set the approval deck treats as a question being asked now.
 */
export function usePartnerQuestions(agentId: string | null): number {
  const api = useApi();
  const apiRef = useRef(api);
  apiRef.current = api;
  const [ids, setIds] = useState<ReadonlySet<string>>(() => new Set());

  const load = useCallback(async () => {
    if (!agentId) return;
    try {
      const items = await apiRef.current.getNotifications(true);
      setIds(
        new Set(
          items
            .filter((i) => i.kind === 'approval' && i.blocking && !i.resolved_at && i.agent_id === agentId)
            .map((i) => i.item_id),
        ),
      );
    } catch {
      // Keep the last answer: "could not check" is not "nothing is waiting".
    }
  }, [agentId]);

  useEffect(() => {
    // Another partner's questions are never shown while this one loads.
    setIds(new Set());
    void load();
  }, [load]);

  useEventStream(
    EVENTS_URL,
    (event) => {
      const id = event.data?.approval_id;
      if (event.type === 'CommandApprovalRequested') {
        if (typeof id === 'string' && event.data?.agent_id === agentId) setIds((prev) => new Set(prev).add(id));
      } else if (event.type === 'CommandApprovalResult') {
        // Every ending arrives here: this window, another window, or the kernel settling it.
        if (typeof id === 'string')
          setIds((prev) => {
            if (!prev.has(id)) return prev;
            const next = new Set(prev);
            next.delete(id);
            return next;
          });
      } else if (event.type === '__reconnected' || event.type === '__lagged') {
        void load();
      }
    },
    api.apiKey,
  );

  return ids.size;
}
