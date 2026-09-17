/**
 * The memory screen's time axis, computed without drawing it.
 *
 * Everything here works on local calendar days: the day a memory belongs to is
 * the day the person reading it was living through, and the distance between
 * two days is taken from their year/month/day — never from a difference in
 * milliseconds, which is wrong by an hour twice a year wherever the clocks
 * move and turns a 1-day hole into 0 or 2.
 */

/** The least a row on the axis has to be: when it happened and whose it is. */
export interface TimelineEvent {
  /** Unique across the list (the React key, and what a test pins a row by). */
  key: string;
  agentId: string;
  at: Date;
}

/** A calendar day that has something on it. */
export interface DayGroup<T extends TimelineEvent> {
  kind: 'day';
  /** Local midnight of the day, for formatting it in the reader's locale. */
  date: Date;
  /** Days since the epoch on the local calendar (the identity of the day). */
  day: number;
  isToday: boolean;
  /** Newest first. */
  events: T[];
}

/** A run of whole calendar days with nothing on them, between two that have. */
export interface GapSegment {
  kind: 'gap';
  /** How many days are missing; always 1 or more. */
  days: number;
}

export type TimelineRow<T extends TimelineEvent> = DayGroup<T> | GapSegment;

/** One day of the density band. */
export interface DensityCell {
  /** Local midnight of the day. */
  date: Date;
  count: number;
  isToday: boolean;
}

const MS_PER_DAY = 86_400_000;

/** Days since the epoch, counted on the local calendar. */
export function localDayNumber(d: Date): number {
  return Math.floor(Date.UTC(d.getFullYear(), d.getMonth(), d.getDate()) / MS_PER_DAY);
}

/** Local midnight of the day a moment falls on. */
export function startOfLocalDay(d: Date): Date {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate());
}

/**
 * Read a memory's time.
 *
 * The memory server writes `created_at` with SQLite's `datetime('now')`, which
 * is UTC with no zone on the end ("2026-09-17 05:03:22"); a browser reads such
 * a string as local time, which moves every memory by the reader's offset and
 * can move it across a day boundary. Those are pinned to UTC here. Anything
 * that already carries a zone (the `timestamp` column, an ISO string with an
 * offset) is left to the platform.
 */
export function parseMemoryTime(raw: string | null | undefined): Date | null {
  if (!raw) return null;
  const s = raw.trim();
  if (!s) return null;
  const zoneless = /^\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}(:\d{2}(\.\d+)?)?$/.test(s);
  const d = new Date(zoneless ? `${s.replace(' ', 'T')}Z` : s);
  return Number.isNaN(d.getTime()) ? null : d;
}

/**
 * Group events into local calendar days, newest first, with one compressed
 * segment standing in for every run of empty days between two days that have
 * something. Nothing is drawn before the newest day or after the oldest: a gap
 * says "nothing happened between these two", and there is no "between" at
 * either end.
 */
export function buildTimeline<T extends TimelineEvent>(events: readonly T[], now: Date): TimelineRow<T>[] {
  const today = localDayNumber(now);
  const byDay = new Map<number, T[]>();
  for (const e of events) {
    const day = localDayNumber(e.at);
    const bucket = byDay.get(day);
    if (bucket) bucket.push(e);
    else byDay.set(day, [e]);
  }

  const days = Array.from(byDay.keys()).sort((a, b) => b - a);
  const rows: TimelineRow<T>[] = [];
  for (let i = 0; i < days.length; i++) {
    const day = days[i];
    if (i > 0) {
      const missing = days[i - 1] - day - 1;
      if (missing >= 1) rows.push({ kind: 'gap', days: missing });
    }
    // Stable: two memories stored in the same second keep the order they came in.
    const dayEvents = (byDay.get(day) as T[]).slice().sort((a, b) => b.at.getTime() - a.at.getTime());
    rows.push({
      kind: 'day',
      date: startOfLocalDay(dayEvents[0].at),
      day,
      isToday: day === today,
      events: dayEvents,
    });
  }
  return rows;
}

/**
 * How many events fell on each of the last `days` local days, oldest cell
 * first and today last. A day outside the window is not counted at all, so the
 * cells sum to exactly the number of events inside it.
 */
export function buildDensity<T extends TimelineEvent>(events: readonly T[], now: Date, days = 30): DensityCell[] {
  const today = localDayNumber(now);
  const first = today - (days - 1);
  const counts = new Map<number, number>();
  for (const e of events) {
    const day = localDayNumber(e.at);
    if (day < first || day > today) continue;
    counts.set(day, (counts.get(day) ?? 0) + 1);
  }

  const cells: DensityCell[] = [];
  for (let i = 0; i < days; i++) {
    const day = first + i;
    cells.push({
      // Built by calendar arithmetic (the Date constructor normalises an
      // out-of-range day), so a DST shift inside the window cannot skew it.
      date: new Date(now.getFullYear(), now.getMonth(), now.getDate() - (days - 1 - i)),
      count: counts.get(day) ?? 0,
      isToday: day === today,
    });
  }
  return cells;
}

/**
 * How wide the bar of a day holding `count` memories is drawn, against a
 * window whose busiest day holds `busiest`.
 *
 * The mock gives three widths — nothing is a 4 % stub, one memory is 30 %, two
 * are 60 % — so a memory is worth 30 % of the band. That is kept exactly while
 * it fits (a window whose busiest day holds three or fewer), and compressed to
 * fit when it does not: at eight a day the unscaled rule would peg half the
 * window at full width and the band would stop saying anything.
 */
export function densityWidth(count: number, busiest: number): string {
  if (count <= 0) return '4%';
  const unit = Math.min(30, 96 / Math.max(busiest, 1));
  return `${Math.round(unit * count)}%`;
}

/**
 * Does this row match what was typed? The query, as one phrase, has to be
 * found in at least one of the haystacks — an empty query matches everything.
 */
export function matchesSearch(haystacks: readonly (string | undefined)[], query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  return haystacks.some((h) => (h ?? '').toLowerCase().includes(q));
}
