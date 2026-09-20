import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { unreadAgentIds, unreadItemsFor } from '../lib/unreadAgents';
import { EVENTS_URL, type NotificationItem } from '../services/api';
import { useApi } from './useApi';
import { useEventStream } from './useEventStream';

/** How often the marks are re-read when no event prompts it (the bell's badge uses the same period). */
const UNREAD_POLL_MS = 15_000;

/**
 * The roster's unread marks.
 *
 * Reads the same store the bell reads (`GET /notifications?unresolved=true`)
 * rather than keeping a count of its own: two places counting the same
 * questions is two places to disagree about what is waiting. It re-reads on the
 * events that change that store — a command approval arriving and one being
 * settled — plus a reconnect, because a stream that dropped is the case where a
 * mark would otherwise sit stale until the next navigation.
 */
export function useUnreadAgents() {
  const api = useApi();
  const [items, setItems] = useState<NotificationItem[]>([]);
  // The second source: agents who have *said* something unread. A waiting
  // question and unread words are different states, and the notification store
  // only knows the first — an agent that reported and asked nothing left no mark
  // anywhere before this.
  const [spoken, setSpoken] = useState<string[]>([]);

  // Through a ref: `useApi` memoizes today, but an effect that restarts on a
  // fresh object identity would re-ask the kernel on every render.
  const apiRef = useRef(api);
  apiRef.current = api;

  const refresh = useCallback(async () => {
    // Settled separately: one source failing must not discard the other's
    // answer, and neither may clear a mark it did not check.
    const [asked, said] = await Promise.allSettled([
      apiRef.current.getNotifications(true),
      apiRef.current.getUnreadAgents(),
    ]);
    // Keep the previous answer on failure. Dropping it would clear every mark on
    // one failed read, which claims "nothing is waiting" — a different and worse
    // statement than "I could not check".
    if (asked.status === 'fulfilled') setItems(asked.value);
    if (said.status === 'fulfilled') setSpoken(said.value);
  }, []);

  // A question an agent asks (`proposal`) raises no event this hook hears, so
  // it also asks on a timer — the same period the bell uses for its badge.
  useEffect(() => {
    void refresh();
    const id = setInterval(() => void refresh(), UNREAD_POLL_MS);
    return () => clearInterval(id);
  }, [refresh]);

  const handleEvent = useCallback(
    (event: { type: string }) => {
      if (
        event.type === 'CommandApprovalRequested' ||
        event.type === 'CommandApprovalResult' ||
        event.type === 'MessageReceived' ||
        event.type === 'AgentResponse' ||
        event.type === '__reconnected'
      ) {
        void refresh();
      }
    },
    [refresh],
  );
  useEventStream(EVENTS_URL, handleEvent, api.apiKey);

  // Memoized on the items themselves: a fresh Set every render is a new
  // identity, and a caller that holds it in an effect's deps would loop.
  const unread = useMemo(() => {
    const ids = unreadAgentIds(items);
    for (const id of spoken) ids.add(id);
    return ids;
  }, [items, spoken]);

  return { unread, refreshUnread: refresh };
}

/**
 * Opening an agent's conversation reads what was waiting on its row.
 *
 * Lives with whoever knows a conversation is open, not with the roster: a chat
 * is reached from the sidebar as often as from the roster, and a mark that only
 * one of those doors clears is a mark that stays after the person has looked.
 * It asks the store afresh instead of trusting a list from earlier, because the
 * roster is not mounted while a chat is, and the question may have arrived since.
 */
export function useReadOnOpen(agentId: string | null) {
  const api = useApi();
  const apiRef = useRef(api);
  apiRef.current = api;

  useEffect(() => {
    if (!agentId) return;
    let cancelled = false;
    void (async () => {
      try {
        const items = await apiRef.current.getNotifications(true);
        if (cancelled) return;
        await Promise.all(
          unreadItemsFor(items, agentId).map((item) =>
            apiRef.current.markNotificationRead(item.item_id).catch(() => {
              // Left unread; the roster's next read reports it truthfully.
            }),
          ),
        );
      } catch {
        // Could not check. Nothing is marked, so nothing is claimed.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [agentId]);
}
