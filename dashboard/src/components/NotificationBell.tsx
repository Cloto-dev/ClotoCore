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
import { useApi } from '../hooks/useApi';
import { type DisplayLevel, displayLevel, interrupts, loadThreshold, saveThreshold } from '../lib/notificationSeverity';
import type { NotificationItem, NotificationSummary } from '../services/api';

/** How often the badge re-asks. Cheap query, two integers. */
const POLL_MS = 15_000;

const LEVEL_STYLE: Record<DisplayLevel, string> = {
  high: 'text-red-500 bg-red-500/10',
  medium: 'text-amber-500 bg-amber-500/10',
  low: 'text-content-tertiary bg-surface-secondary',
};

export function NotificationBell() {
  const { t } = useTranslation();
  const api = useApi();
  const [summary, setSummary] = useState<NotificationSummary>({ waiting: 0, blocking: 0 });
  const [items, setItems] = useState<NotificationItem[]>([]);
  const [open, setOpen] = useState(false);
  const [threshold, setThreshold] = useState<DisplayLevel>(loadThreshold);
  const [anchor, setAnchor] = useState<{ top: number; left: number } | null>(null);
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
        className="relative p-1 rounded hover:bg-glass text-content-tertiary hover:text-content-primary transition-colors"
      >
        <Bell size={14} />
        {summary.waiting > 0 && (
          <span
            data-testid="notification-badge"
            className={`absolute -top-0.5 -right-0.5 min-w-[13px] h-[13px] px-[3px] rounded-full text-[9px] font-mono leading-[13px] text-center ${
              summary.blocking > 0 ? 'bg-red-500 text-white' : 'bg-brand text-white'
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
              <span className="text-[10px] font-mono uppercase tracking-widest text-content-secondary">
                {t('notifications.title', { defaultValue: 'Notifications' })}
              </span>
              {summary.blocking > 0 && (
                <span className="text-[9px] font-mono text-red-500">
                  {t('notifications.blocking_count', {
                    count: summary.blocking,
                    defaultValue: '{{count}} holding an agent',
                  })}
                </span>
              )}
            </div>

            <div className="px-3 py-2 border-b border-edge">
              <div className="text-[9px] font-mono text-content-tertiary mb-1">
                {t('notifications.threshold_label', { defaultValue: 'Interrupt me with a card at' })}
              </div>
              <div className="flex items-center gap-1">
                {(['high', 'medium', 'low'] as const).map((level) => (
                  <button
                    key={level}
                    type="button"
                    onClick={() => chooseThreshold(level)}
                    className={`px-2 py-0.5 rounded text-[9px] font-mono border transition-colors ${
                      threshold === level
                        ? 'border-brand text-content-primary bg-glass-strong'
                        : 'border-edge text-content-tertiary hover:border-brand'
                    }`}
                  >
                    {t(`notifications.threshold_${level}`, { defaultValue: level })}
                  </button>
                ))}
              </div>
              <div className="text-[9px] font-mono text-content-tertiary mt-1">
                {t('notifications.threshold_note', {
                  defaultValue: 'Everything waiting is listed here whatever you pick.',
                })}
              </div>
            </div>

            {items.length === 0 ? (
              <div className="px-3 py-6 text-center text-[10px] font-mono text-content-tertiary">
                {t('notifications.empty', { defaultValue: 'Nothing is waiting' })}
              </div>
            ) : (
              <ul>
                {items.map((item) => {
                  const level = displayLevel(item.severity);
                  return (
                    <li
                      key={item.item_id}
                      data-testid="notification-item"
                      className="px-3 py-2 border-b border-edge last:border-b-0"
                    >
                      <div className="flex items-start gap-2">
                        <span className={`shrink-0 px-1.5 py-0.5 rounded text-[9px] font-mono ${LEVEL_STYLE[level]}`}>
                          {t(`notifications.level_${level}`, { defaultValue: level })}
                        </span>
                        <div className="min-w-0 flex-1">
                          <div className="text-[11px] text-content-primary break-words">{item.title}</div>
                          {item.body && (
                            <div className="mt-0.5 text-[9px] font-mono text-content-tertiary break-words whitespace-pre-wrap">
                              {item.body}
                            </div>
                          )}
                          <div className="mt-1 flex items-center gap-2 text-[9px] font-mono text-content-tertiary">
                            {item.blocking && (
                              <span className="text-red-500">
                                {t('notifications.blocking', { defaultValue: 'holding an agent' })}
                              </span>
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
