/**
 * Every agent's approvals, one card, at the window level.
 *
 * The card used to be drawn inside `AgentConsole`, which made it part of one
 * agent's conversation: a question from an agent you were not looking at was
 * asked somewhere you were not. Now that requests have no deadline, "somewhere
 * you were not" can last indefinitely, so the card was moved up here, beside the
 * bell that counts the same items.
 *
 * Three rules hold this together, and each exists because its opposite fails
 * quietly:
 *
 * * **The peek window ends the card, never the item.** After
 *   {@link PEEK_MS} the deck stops showing; nothing is answered, nothing is
 *   resolved, and the bell still lists it. That is what makes a short window the
 *   right one — if the card disappearing lost anything, thirty seconds would be
 *   a trap rather than a courtesy. It is also why the bell can raise this deck
 *   again ({@link RAISE_APPROVAL_EVENT}): a peek you cannot reopen is a deadline
 *   wearing different clothes.
 * * **The timer belongs to the card, not to the item.** Paging is reading, so it
 *   restarts the window; a busy agent cannot pin the card open forever, and a
 *   reader working through a queue is not interrupted by their own attention.
 * * **New arrivals go to the end and never move the page under a reader.** They
 *   announce themselves in the count. With the deck closed there is no reader to
 *   protect, so the arrival that interrupts is the one that opens.
 *
 * The threshold decides whether an arrival raises the deck — nothing else. It
 * never decides what is in the queue, and it is not consulted by the badge.
 */

import { useCallback, useEffect, useReducer, useRef, useState } from 'react';
import { useApi } from '../hooks/useApi';
import { useEventStream } from '../hooks/useEventStream';
import { asSeverity, interrupts, loadThreshold, mostSevere } from '../lib/notificationSeverity';
import { EVENTS_URL, type NotificationItem, type NotificationSeverity } from '../services/api';
import { CommandApprovalCard } from './CommandApprovalCard';

/** How long the card stays up without being touched. */
export const PEEK_MS = 30_000;

/**
 * How long the buttons stay held after an answer.
 *
 * The next card's buttons appear where the last click landed, so a double click
 * aimed at one request would otherwise answer the one behind it.
 */
export const REARM_MS = 500;

/** Raise the deck at one approval: `detail: { approvalId }`. Sent by the bell. */
export const RAISE_APPROVAL_EVENT = 'cloto-raise-approval';

interface CommandEntry {
  command: string;
  command_name: string;
}

interface PendingApproval {
  approvalId: string;
  agentId: string;
  commands: CommandEntry[];
  severity: NotificationSeverity;
}

interface DeckState {
  queue: PendingApproval[];
  index: number;
  open: boolean;
  /** Bumped by anything that counts as reading, to restart the peek window. */
  peek: number;
}

type DeckAction =
  | { type: 'requested'; entry: PendingApproval; raises: boolean }
  | { type: 'restored'; entries: PendingApproval[] }
  | { type: 'resolved'; approvalId: string }
  | { type: 'page'; delta: number }
  | { type: 'expire' }
  | { type: 'raise'; approvalId: string };

const EMPTY: DeckState = { queue: [], index: 0, open: false, peek: 0 };

export function deckReducer(state: DeckState, action: DeckAction): DeckState {
  switch (action.type) {
    case 'requested': {
      if (state.queue.some((a) => a.approvalId === action.entry.approvalId)) return state;
      const queue = [...state.queue, action.entry];
      if (!action.raises) return { ...state, queue };
      // Open: somebody is reading, so the arrival waits its turn at the end and
      // says so through the count. Closed: nobody is mid-anything, and the
      // request that just interrupted is the one worth showing.
      return state.open ? { ...state, queue } : { queue, index: queue.length - 1, open: true, peek: state.peek + 1 };
    }

    case 'restored': {
      // What the store already held when this window opened. It never raises the
      // deck: these have been waiting, the badge says so, and a card that
      // reappears on every reload for the same unanswered request teaches people
      // to wave it away. The bell is how you ask for it.
      const known = new Set(state.queue.map((a) => a.approvalId));
      const added = action.entries.filter((e) => !known.has(e.approvalId));
      return added.length === 0 ? state : { ...state, queue: [...state.queue, ...added] };
    }

    case 'resolved': {
      const at = state.queue.findIndex((a) => a.approvalId === action.approvalId);
      if (at === -1) return state;
      const queue = state.queue.filter((a) => a.approvalId !== action.approvalId);
      if (queue.length === 0) return { ...state, queue, index: 0, open: false };
      // Something removed ahead of the cursor would otherwise slide every later
      // page back by one under the reader; removing the current one lands on
      // whatever took its place.
      const index = at < state.index ? state.index - 1 : Math.min(state.index, queue.length - 1);
      return { ...state, queue, index };
    }

    case 'page': {
      if (state.queue.length < 2) return state;
      const index = (state.index + action.delta + state.queue.length) % state.queue.length;
      return { ...state, index, peek: state.peek + 1 };
    }

    case 'expire':
      // The card goes. The request does not: still pending, still blocking, still
      // on the bell, still answerable from it.
      return { ...state, open: false };

    case 'raise': {
      const at = state.queue.findIndex((a) => a.approvalId === action.approvalId);
      if (at === -1) return state;
      return { ...state, index: at, open: true, peek: state.peek + 1 };
    }
  }
}

/** Read one request out of an untyped stream event. */
function fromEvent(data: Record<string, unknown>): PendingApproval | null {
  const approvalId = typeof data.approval_id === 'string' ? data.approval_id : null;
  if (!approvalId) return null;
  const rawCommands = Array.isArray(data.commands) ? (data.commands as Record<string, unknown>[]) : [];
  return {
    approvalId,
    agentId: typeof data.agent_id === 'string' ? data.agent_id : '',
    commands: rawCommands.map((c) => ({
      command: typeof c.command === 'string' ? c.command : '',
      command_name: typeof c.command_name === 'string' ? c.command_name : '',
    })),
    // The kernel puts its own derivation on every command it asks about, so the
    // batch is read from what it sent rather than classified a second time here.
    severity: mostSevere(rawCommands.map((c) => c.severity)),
  };
}

/** Read one request out of a stored notification row. */
function fromStore(item: NotificationItem): PendingApproval | null {
  if (item.kind !== 'approval' || !item.blocking) return null;
  const rawCommands = Array.isArray(item.metadata?.commands)
    ? (item.metadata.commands as Record<string, unknown>[])
    : [];
  return {
    approvalId: item.item_id,
    agentId: item.agent_id ?? '',
    commands: rawCommands.map((c) => ({
      command: typeof c.command === 'string' ? c.command : '',
      command_name: typeof c.command_name === 'string' ? c.command_name : '',
    })),
    severity: asSeverity(item.severity) ?? mostSevere(rawCommands.map((c) => c.severity)),
  };
}

export function CommandApprovalDeck() {
  const api = useApi();
  const [state, dispatch] = useReducer(deckReducer, EMPTY);
  const [armed, setArmed] = useState(true);

  const apiRef = useRef(api);
  apiRef.current = api;

  // What was already waiting before this window existed. Without it a reload
  // would strand a blocked agent: the deck only hears about arrivals, and with
  // no deadline the kernel will not give up and free it either.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const items = await apiRef.current.getNotifications(true);
        if (cancelled) return;
        const entries = items.map(fromStore).filter((e): e is PendingApproval => e !== null);
        if (entries.length > 0) dispatch({ type: 'restored', entries });
      } catch {
        // The stream still delivers everything that arrives from now on.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const handleEvent = useCallback((event: { type: string; data: Record<string, unknown> }) => {
    if (event.type === 'CommandApprovalRequested') {
      const entry = fromEvent(event.data);
      // The threshold is read here, not held in state: the bell can change it
      // while this component stays mounted, and the answer that matters is the
      // one in force when the request arrives.
      if (entry) dispatch({ type: 'requested', entry, raises: interrupts(entry.severity, loadThreshold()) });
      return;
    }
    if (event.type === 'CommandApprovalResult') {
      const approvalId = event.data.approval_id;
      // Covers every ending, not just this window's: another window answering,
      // and the kernel settling one itself, both arrive here.
      if (typeof approvalId === 'string') dispatch({ type: 'resolved', approvalId });
    }
  }, []);

  useEventStream(EVENTS_URL, handleEvent, api.apiKey);

  // The bell's way back in.
  useEffect(() => {
    const onRaise = (e: Event) => {
      const approvalId = (e as CustomEvent).detail?.approvalId;
      if (typeof approvalId === 'string') dispatch({ type: 'raise', approvalId });
    };
    window.addEventListener(RAISE_APPROVAL_EVENT, onRaise);
    return () => window.removeEventListener(RAISE_APPROVAL_EVENT, onRaise);
  }, []);

  // The peek window. Restarted by opening, by paging, and by a raise — every
  // one of those is somebody looking at it.
  useEffect(() => {
    if (!state.open) return;
    const id = setTimeout(() => dispatch({ type: 'expire' }), PEEK_MS);
    return () => clearTimeout(id);
  }, [state.open, state.index, state.peek]);

  useEffect(() => {
    if (armed) return;
    const id = setTimeout(() => setArmed(true), REARM_MS);
    return () => clearTimeout(id);
  }, [armed]);

  const onResolved = useCallback((approvalId: string) => {
    setArmed(false);
    dispatch({ type: 'resolved', approvalId });
  }, []);

  if (!state.open || state.queue.length === 0) return null;

  const current = state.queue[Math.min(state.index, state.queue.length - 1)];
  const total = state.queue.length;

  return (
    // Where SecurityGuard sits, for the same reason: over the page, out of the
    // way of the content, and reachable from any route.
    <div
      data-testid="approval-deck"
      className="fixed bottom-8 right-8 z-[1000] max-w-md w-full animate-in slide-in-from-bottom-4 duration-300"
    >
      <CommandApprovalCard
        // Keyed so a new request gets a fresh card. Without this the component
        // is reused and its `resolved` state carries over, blanking the very
        // request that replaced the answered one.
        key={current.approvalId}
        approvalId={current.approvalId}
        agentId={current.agentId}
        commands={current.commands}
        onResolved={onResolved}
        actionsDisabled={!armed}
        pager={
          total > 1 ? (
            <span className="flex items-center gap-1 text-[10px] font-mono text-content-tertiary">
              <button
                type="button"
                onClick={() => dispatch({ type: 'page', delta: -1 })}
                aria-label="Previous request"
                className="px-1 rounded hover:bg-glass hover:text-content-primary transition-colors"
              >
                ‹
              </button>
              <span data-testid="approval-pager">
                {state.index + 1} / {total}
              </span>
              <button
                type="button"
                onClick={() => dispatch({ type: 'page', delta: 1 })}
                aria-label="Next request"
                className="px-1 rounded hover:bg-glass hover:text-content-primary transition-colors"
              >
                ›
              </button>
            </span>
          ) : null
        }
      />
    </div>
  );
}
