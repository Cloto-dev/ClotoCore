/**
 * The bell on the window's top edge, and what opens under it.
 *
 * It exists so that a question an agent is waiting on reaches a person who is
 * not looking at the screen it was asked on. Two rules shape everything here:
 *
 * * **The badge counts what is waiting, never what is loud.** The threshold
 *   below governs interruption only. A count that respected it would turn a
 *   display preference into a switch that starves agents nobody can see are
 *   stuck — and the symptom is an agent that simply never finishes.
 * * **The panel is rendered outside the header, through a portal.** Two
 *   separate reasons, and the second one is the one that bites. The header is
 *   `select-none` so the window can be dragged by it, and an audit id nobody
 *   can copy is an audit id nobody can use. But the header also carries
 *   `relative z-10`, which makes it a stacking context — and `<main>` is a
 *   later sibling with the same `z-10`, so the whole header paints *under* the
 *   page content no matter what z-index the panel gives itself. A panel that
 *   stays inside the header is occluded by the agent cards it floats over.
 */

import { Bell } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { useApi } from '../hooks/useApi';
import { notificationText } from '../lib/notificationMessage';
import { type DisplayLevel, displayLevel, interrupts, loadThreshold, saveThreshold } from '../lib/notificationSeverity';
import type { NotificationItem, NotificationSummary } from '../services/api';
import { RAISE_APPROVAL_EVENT } from './CommandApprovalDeck';

/**
 * The in-app page a notice leads to, from its metadata's `link`. Only a path
 * inside this app is accepted: a notice's metadata can come from an agent, and
 * the bell must not become a way to send the reader somewhere else.
 */
export function inAppLink(metadata: Record<string, unknown> | null): string | null {
  const link = metadata?.link;
  if (typeof link !== 'string') return null;
  if (!link.startsWith('/') || link.startsWith('//') || link.includes('\\')) return null;
  return link;
}

/** How often the badge re-asks. Cheap query, two integers. */
const POLL_MS = 15_000;

const LEVEL_STYLE: Record<DisplayLevel, string> = {
  high: 'text-red-500 bg-red-500/10',
  medium: 'text-amber-500 bg-amber-500/10',
  low: 'text-content-tertiary bg-surface-secondary',
};

export function NotificationBell() {
  const { t, i18n } = useTranslation();
  const api = useApi();
  const navigate = useNavigate();
  const [summary, setSummary] = useState<NotificationSummary>({ waiting: 0, blocking: 0 });
  const [items, setItems] = useState<NotificationItem[]>([]);
  const [open, setOpen] = useState(false);
  const [threshold, setThreshold] = useState<DisplayLevel>(loadThreshold);
  const [anchor, setAnchor] = useState<{ top: number; left: number } | null>(null);
  const [answering, setAnswering] = useState<string | null>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);

  // Read through a ref rather than depending on the object's identity. `useApi`
  // memoizes today, but a polling loop that restarts whenever a caller happens
  // to hand back a fresh object is a runaway that nothing would report — the
  // bell would simply ask the kernel for the count on every render.
  const apiRef = useRef(api);
  apiRef.current = api;

  const refreshSummary = useCallback(async () => {
    try {
      setSummary(await apiRef.current.getNotificationSummary());
    } catch {
      // A bell that throws away its count because one poll failed reports
      // "nothing is waiting", which is a different and much worse claim than
      // "I could not check".
    }
  }, []);

  useEffect(() => {
    void refreshSummary();
    const id = setInterval(() => void refreshSummary(), POLL_MS);
    return () => clearInterval(id);
  }, [refreshSummary]);

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    void (async () => {
      try {
        const fetched = await apiRef.current.getNotifications(true);
        if (!cancelled) setItems(fetched);
      } catch {
        if (!cancelled) setItems([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [open]);

  // Where to put the panel, in viewport coordinates. Recomputed while it is
  // open because the window can be resized under it, and a panel anchored to
  // where the bell used to be is worse than one that is simply closed.
  useEffect(() => {
    if (!open) {
      setAnchor(null);
      return;
    }
    const place = () => {
      const rect = buttonRef.current?.getBoundingClientRect();
      if (rect) setAnchor({ top: rect.bottom + 4, left: rect.left });
    };
    place();
    window.addEventListener('resize', place);
    return () => window.removeEventListener('resize', place);
  }, [open]);

  // Close on a click anywhere else, the way the rest of the chrome behaves.
  // Both refs are consulted: the panel is portaled out of this subtree, so a
  // click inside it is not inside the button's container.
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (e: PointerEvent) => {
      const target = e.target as Node;
      if (panelRef.current?.contains(target) || buttonRef.current?.contains(target)) return;
      setOpen(false);
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false);
    };
    document.addEventListener('pointerdown', onPointerDown);
    document.addEventListener('keydown', onKeyDown);
    return () => {
      document.removeEventListener('pointerdown', onPointerDown);
      document.removeEventListener('keydown', onKeyDown);
    };
  }, [open]);

  const chooseThreshold = (level: DisplayLevel) => {
    setThreshold(level);
    saveThreshold(level);
  };

  /**
   * Put the card for this item back on screen.
   *
   * This is what makes the deck's peek window a courtesy rather than a second
   * deadline. The card stops showing after half a minute and the item stays
   * here, which is only true in any useful sense if here leads back to it. The
   * answering controls are not duplicated into this list on purpose: two places
   * to approve a destructive command is one place too many to keep honest.
   */
  const raise = (itemId: string) => {
    window.dispatchEvent(new CustomEvent(RAISE_APPROVAL_EVENT, { detail: { approvalId: itemId } }));
    setOpen(false);
  };

  /**
   * Reply to a proposal.
   *
   * Answered in place rather than by raising a card, which is the opposite of
   * what the button above does for an approval — and the difference is the
   * point. An approval releases a blocked agent and can destroy something, so it
   * gets one deliberate surface with the command in front of you. A proposal
   * stops nothing; making someone open a card to say "yes" would add a step to
   * the cheap case and teach them to skip the expensive one.
   */
  const answer = async (itemId: string, decision: string) => {
    setAnswering(itemId);
    try {
      await apiRef.current.answerNotification(itemId, decision);
      setItems((prev) => prev.filter((i) => i.item_id !== itemId));
      void refreshSummary();
    } catch {
      // Leave it in the list: the next open re-reads what actually happened,
      // and an item that vanished without being answered is the one thing this
      // panel must never show.
    } finally {
      setAnswering(null);
    }
  };

  const markRead = async (itemId: string) => {
    try {
      await apiRef.current.markNotificationRead(itemId);
      setItems((prev) => prev.map((i) => (i.item_id === itemId ? { ...i, read_at: new Date().toISOString() } : i)));
      // Deliberately no summary refresh shortcut here: reading is not
      // answering, so the count does not move and pretending otherwise would
      // teach the reader that clearing the badge clears the work.
    } catch {
      // Leave the item as unread; the next open re-reads the truth.
    }
  };

  return (
    <div className="relative">
      <button
        ref={buttonRef}
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-label={t('notifications.open', { defaultValue: 'Notifications' })}
        aria-expanded={open}
        title={t('notifications.open', { defaultValue: 'Notifications' })}
        className="relative p-1 rounded hover:bg-surface-panel text-content-tertiary hover:text-content-primary transition-colors"
      >
        <Bell size={14} />
        {summary.waiting > 0 && (
          <span
            data-testid="notification-badge"
            className={`absolute -top-0.5 -right-0.5 min-w-[13px] h-[13px] px-[3px] rounded-full text-xs font-mono leading-[13px] text-center ${
              summary.blocking > 0 ? 'bg-red-500 text-agent-ink' : 'bg-agent text-agent-ink'
            }`}
          >
            {summary.waiting > 99 ? '99+' : summary.waiting}
          </span>
        )}
      </button>

      {open &&
        anchor &&
        createPortal(
          // Portaled to the body: inside the header this panel is trapped in a
          // stacking context that the page content paints over, whatever
          // z-index it claims. See the note at the top of this file.
          //
          // `bg-surface-primary` is opaque, and has to be. The glass tokens are
          // 60-80% alpha, which reads well for a panel sitting over a static
          // background — the sidebar, the actions rail — and not at all for one
          // floating over agent cards, where high-contrast text behind it stays
          // legible straight through. The status tooltip in the header answers
          // this question the same way.
          //
          // `select-text` is kept even though the portal already escapes the
          // header's `select-none`: it states the requirement where the panel
          // is, so moving the panel back does not quietly lose it.
          <div
            ref={panelRef}
            style={{ top: anchor.top, left: anchor.left }}
            className="fixed w-[340px] max-h-[420px] overflow-y-auto rounded-lg border border-edge bg-surface-primary shadow-lg z-[9999] select-text"
          >
            <div className="flex items-center justify-between px-3 py-2 border-b border-edge">
              <span className="text-xs font-mono text-content-secondary">
                {t('notifications.title', { defaultValue: 'Notifications' })}
              </span>
              {summary.blocking > 0 && (
                <span className="text-xs font-mono text-red-500">
                  {t('notifications.blocking_count', {
                    count: summary.blocking,
                    defaultValue: '{{count}} holding an agent',
                  })}
                </span>
              )}
            </div>

            <div className="px-3 py-2 border-b border-edge">
              <div className="text-xs font-mono text-content-tertiary mb-1">
                {t('notifications.threshold_label', { defaultValue: 'Interrupt me with a card at' })}
              </div>
              <div className="flex items-center gap-1">
                {(['high', 'medium', 'low'] as const).map((level) => (
                  <button
                    key={level}
                    type="button"
                    onClick={() => chooseThreshold(level)}
                    className={`px-2 py-0.5 rounded text-xs font-mono border transition-colors ${
                      threshold === level
                        ? 'border-agent text-content-primary bg-surface-field'
                        : 'border-edge text-content-tertiary hover:border-agent'
                    }`}
                  >
                    {t(`notifications.threshold_${level}`, { defaultValue: level })}
                  </button>
                ))}
              </div>
              <div className="text-xs font-mono text-content-tertiary mt-1">
                {t('notifications.threshold_note', {
                  defaultValue: 'Everything waiting is listed here whatever you pick.',
                })}
              </div>
            </div>

            {items.length === 0 ? (
              <div className="px-3 py-6 text-center text-xs font-mono text-content-tertiary">
                {t('notifications.empty', { defaultValue: 'Nothing is waiting' })}
              </div>
            ) : (
              <ul>
                {items.map((item) => {
                  const level = displayLevel(item.severity);
                  // The kernel writes these in English and cannot know who is
                  // reading; `metadata.message` is how a notice says which
                  // sentence it is, so the pack can say it in this language.
                  const said = notificationText(item, t, i18n.language);
                  return (
                    <li
                      key={item.item_id}
                      data-testid="notification-item"
                      className="px-3 py-2 border-b border-edge last:border-b-0"
                    >
                      <div className="flex items-start gap-2">
                        <span className={`shrink-0 px-1.5 py-0.5 rounded text-xs font-mono ${LEVEL_STYLE[level]}`}>
                          {t(`notifications.level_${level}`, { defaultValue: level })}
                        </span>
                        <div className="min-w-0 flex-1">
                          <div className="text-xs text-content-primary break-words">{said.title}</div>
                          {said.body && (
                            <div className="mt-0.5 text-xs font-mono text-content-tertiary break-words whitespace-pre-wrap">
                              {said.body}
                            </div>
                          )}
                          <div className="mt-1 flex items-center gap-2 text-xs font-mono text-content-tertiary">
                            {item.blocking && (
                              <span className="text-red-500">
                                {t('notifications.blocking', { defaultValue: 'holding an agent' })}
                              </span>
                            )}
                            {item.kind === 'proposal' && !item.resolved_at && (
                              <>
                                <button
                                  type="button"
                                  data-testid="answer-yes"
                                  disabled={answering === item.item_id}
                                  onClick={() => void answer(item.item_id, 'yes')}
                                  className="text-emerald-400 hover:text-emerald-300 disabled:opacity-50 transition-colors"
                                >
                                  {t('notifications.answer_yes', { defaultValue: 'Yes' })}
                                </button>
                                <button
                                  type="button"
                                  data-testid="answer-no"
                                  disabled={answering === item.item_id}
                                  onClick={() => void answer(item.item_id, 'no')}
                                  className="text-content-secondary hover:text-red-400 disabled:opacity-50 transition-colors"
                                >
                                  {t('notifications.answer_no', { defaultValue: 'No' })}
                                </button>
                              </>
                            )}
                            {item.kind === 'approval' && item.blocking && (
                              <button
                                type="button"
                                data-testid="raise-approval"
                                onClick={() => raise(item.item_id)}
                                className="text-content-primary hover:text-agent transition-colors"
                              >
                                {t('notifications.answer', { defaultValue: 'Answer' })}
                              </button>
                            )}
                            {inAppLink(item.metadata) && (
                              <button
                                type="button"
                                data-testid="open-link"
                                onClick={() => {
                                  navigate(inAppLink(item.metadata) as string);
                                  setOpen(false);
                                }}
                                className="text-content-primary hover:text-agent transition-colors"
                              >
                                {t('notifications.open_link', { defaultValue: 'Open' })}
                              </button>
                            )}
                            {interrupts(item.severity, threshold) && (
                              <span data-testid="will-interrupt">
                                {t('notifications.will_interrupt', { defaultValue: 'will interrupt' })}
                              </span>
                            )}
                            {!item.read_at && (
                              <button
                                type="button"
                                onClick={() => void markRead(item.item_id)}
                                className="hover:text-content-primary transition-colors"
                              >
                                {t('notifications.mark_read', { defaultValue: 'Mark read' })}
                              </button>
                            )}
                          </div>
                        </div>
                      </div>
                    </li>
                  );
                })}
              </ul>
            )}
          </div>,
          document.body,
        )}
    </div>
  );
}
