/**
 * The times the living room draws (docs/gui/samples/02-chat-conversation.html):
 * a clock beside each turn, a day between days, and a plain "2 hours ago" in
 * the agent's state line. Dates are the user's calendar days in local time,
 * as in lib/conversations.ts.
 */

/** `14:02` — the clock in the message's meta line. */
export function timeOfDay(ts: number, locale?: string): string {
  return new Intl.DateTimeFormat(locale, { hour: '2-digit', minute: '2-digit', hour12: false }).format(new Date(ts));
}

export function sameLocalDay(a: number, b: number): boolean {
  const da = new Date(a);
  const db = new Date(b);
  return da.getFullYear() === db.getFullYear() && da.getMonth() === db.getMonth() && da.getDate() === db.getDate();
}

export type DayLabel = { kind: 'today' } | { kind: 'yesterday' } | { kind: 'date'; month: number; day: number };

/** What the day separator says: today, yesterday, or the month and day. */
export function dayLabel(ts: number, now: number = Date.now()): DayLabel {
  if (sameLocalDay(ts, now)) return { kind: 'today' };
  const yesterday = new Date(now);
  yesterday.setDate(yesterday.getDate() - 1);
  if (sameLocalDay(ts, yesterday.getTime())) return { kind: 'yesterday' };
  const d = new Date(ts);
  return { kind: 'date', month: d.getMonth() + 1, day: d.getDate() };
}

/** Which of these turns start a new day, so a separator is drawn above them. */
export function dayBreaks(timestamps: number[]): boolean[] {
  return timestamps.map((ts, i) => i === 0 || !sameLocalDay(timestamps[i - 1], ts));
}

/** `2 hours ago` in the user's language, for the state line. */
export function relativeTime(ts: number, now: number = Date.now(), locale?: string): string {
  const rtf = new Intl.RelativeTimeFormat(locale, { numeric: 'auto' });
  const diffSec = Math.round((ts - now) / 1000);
  const abs = Math.abs(diffSec);
  if (abs < 60) return rtf.format(diffSec, 'second');
  if (abs < 3600) return rtf.format(Math.round(diffSec / 60), 'minute');
  if (abs < 86400) return rtf.format(Math.round(diffSec / 3600), 'hour');
  return rtf.format(Math.round(diffSec / 86400), 'day');
}

/** `18.9k` / `128k` — the context meter's numbers. Below a thousand, the number itself. */
export function compactCount(n: number): string {
  if (n < 1000) return String(n);
  if (n < 10_000) return `${(n / 1000).toFixed(1).replace(/\.0$/, '')}k`;
  if (n < 1_000_000) return `${Math.round(n / 1000)}k`;
  return `${(n / 1_000_000).toFixed(1).replace(/\.0$/, '')}M`;
}
