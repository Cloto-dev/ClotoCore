// The clock is pinned, and to a zone that is neither UTC nor free of daylight
// saving: on a machine set to UTC, "the local day" and "the UTC day" are the
// same thing, so a test written to say the grouping is local would pass over an
// implementation that is not. Every date below is built from local components,
// so the file says the same thing wherever it runs.
//
// The zone lives in the worker's environment, which outlives this file, so it
// is put back afterwards — the app's tsconfig has no Node types, hence the
// cast rather than a bare `process`.
const env = (globalThis as unknown as { process: { env: Record<string, string | undefined> } }).process.env;
const ORIGINAL_TZ = env.TZ;
env.TZ = 'America/New_York';

import { afterAll, describe, expect, it } from 'vitest';
import {
  buildDensity,
  buildTimeline,
  type DayGroup,
  densityWidth,
  localDayNumber,
  matchesSearch,
  parseMemoryTime,
  type TimelineEvent,
} from '../memoryTimeline';

afterAll(() => {
  if (ORIGINAL_TZ === undefined) delete env.TZ;
  else env.TZ = ORIGINAL_TZ;
});

const ev = (key: string, agentId: string, at: Date): TimelineEvent => ({ key, agentId, at });
const days = (rows: ReturnType<typeof buildTimeline<TimelineEvent>>) =>
  rows.filter((r): r is DayGroup<TimelineEvent> => r.kind === 'day');
const gaps = (rows: ReturnType<typeof buildTimeline<TimelineEvent>>) => rows.filter((r) => r.kind === 'gap');

describe('the zone this file reasons in', () => {
  it('is pinned, so the local day is not the UTC day', () => {
    // -240 = UTC-4, New York in September. If this is 0 the pin did not take
    // and the local-versus-UTC tests below would prove nothing.
    expect(new Date(2026, 8, 17).getTimezoneOffset()).toBe(240);
  });
});

describe('grouping memories into the days a person lived through', () => {
  const now = new Date(2026, 8, 17, 23, 0);

  it('puts two memories from the same local day in one group', () => {
    // 01:00 and 22:00 in New York are two different UTC days.
    const rows = buildTimeline(
      [ev('a', 'x', new Date(2026, 8, 17, 1, 0)), ev('b', 'x', new Date(2026, 8, 17, 22, 0))],
      now,
    );
    expect(rows).toHaveLength(1);
    const [day] = days(rows);
    expect(day.events.map((e) => e.key)).toEqual(['b', 'a']);
    expect(day.isToday).toBe(true);
  });

  it('puts a minute either side of local midnight in two groups', () => {
    // Both of these are 17 September in UTC.
    const rows = buildTimeline(
      [ev('late', 'x', new Date(2026, 8, 16, 23, 59)), ev('early', 'x', new Date(2026, 8, 17, 0, 1))],
      now,
    );
    expect(days(rows).map((d) => d.events.map((e) => e.key))).toEqual([['early'], ['late']]);
    expect(gaps(rows)).toHaveLength(0);
  });

  it('draws one segment for a three-day hole, between the two days that have something', () => {
    const rows = buildTimeline(
      [ev('new', 'x', new Date(2026, 8, 17, 10, 0)), ev('old', 'x', new Date(2026, 8, 13, 10, 0))],
      now,
    );
    expect(rows.map((r) => r.kind)).toEqual(['day', 'gap', 'day']);
    expect(rows[1]).toEqual({ kind: 'gap', days: 3 });
    expect((rows[0] as DayGroup<TimelineEvent>).events[0].key).toBe('new');
    expect((rows[2] as DayGroup<TimelineEvent>).events[0].key).toBe('old');
  });

  it('draws no segment between days that are next to each other', () => {
    const rows = buildTimeline(
      [ev('new', 'x', new Date(2026, 8, 17, 10, 0)), ev('old', 'x', new Date(2026, 8, 16, 10, 0))],
      now,
    );
    expect(rows.map((r) => r.kind)).toEqual(['day', 'day']);
  });

  it('draws a segment of one for a single missing day', () => {
    const rows = buildTimeline(
      [ev('new', 'x', new Date(2026, 8, 17, 10, 0)), ev('old', 'x', new Date(2026, 8, 15, 10, 0))],
      now,
    );
    expect(gaps(rows)).toEqual([{ kind: 'gap', days: 1 }]);
  });

  it('counts a hole in days even when the clocks moved inside it', () => {
    // 8 March 2026 is 23 hours long in New York: 7 → 9 March is one missing
    // day, but the milliseconds between the two midnights are only 47 hours.
    const rows = buildTimeline(
      [ev('after', 'x', new Date(2026, 2, 9, 23, 0)), ev('before', 'x', new Date(2026, 2, 7, 0, 30))],
      new Date(2026, 2, 10, 12, 0),
    );
    expect(gaps(rows)).toEqual([{ kind: 'gap', days: 1 }]);
  });

  it('puts the newest day first and the newest memory of a day first', () => {
    const rows = buildTimeline(
      [
        ev('old-morning', 'x', new Date(2026, 8, 15, 6, 0)),
        ev('new-evening', 'x', new Date(2026, 8, 16, 21, 0)),
        ev('old-evening', 'x', new Date(2026, 8, 15, 20, 0)),
        ev('new-morning', 'x', new Date(2026, 8, 16, 7, 0)),
      ],
      now,
    );
    expect(days(rows).map((d) => d.events.map((e) => e.key))).toEqual([
      ['new-evening', 'new-morning'],
      ['old-evening', 'old-morning'],
    ]);
  });

  it('marks today only on today', () => {
    const rows = buildTimeline(
      [ev('now', 'x', new Date(2026, 8, 17, 1, 0)), ev('then', 'x', new Date(2026, 8, 16, 1, 0))],
      now,
    );
    expect(days(rows).map((d) => d.isToday)).toEqual([true, false]);
  });

  it('numbers local days one apart across a midnight', () => {
    expect(localDayNumber(new Date(2026, 8, 17, 0, 1)) - localDayNumber(new Date(2026, 8, 16, 23, 59))).toBe(1);
  });
});

describe('the last thirty days', () => {
  const now = new Date(2026, 8, 17, 12, 0);
  const ago = (n: number, hour = 12) => new Date(2026, 8, 17 - n, hour, 0);

  it('is thirty cells ending on today', () => {
    const cells = buildDensity([], now, null);
    expect(cells).toHaveLength(30);
    expect(cells[29].isToday).toBe(true);
    expect(cells.filter((c) => c.isToday)).toHaveLength(1);
    expect(cells[0].date.getDate()).toBe(ago(29).getDate());
    expect(cells[0].date.getMonth()).toBe(ago(29).getMonth());
  });

  it('takes the twenty-ninth day back into the first cell and leaves the thirtieth out', () => {
    const cells = buildDensity([ev('in', 'x', ago(29)), ev('out', 'x', ago(30))], now, null);
    expect(cells[0].count).toBe(1);
    expect(cells.reduce((sum, c) => sum + c.count, 0)).toBe(1);
  });

  it('sums to exactly the memories that fall inside the window', () => {
    const inside = [ev('a', 'x', ago(0)), ev('b', 'x', ago(0, 23)), ev('c', 'x', ago(29)), ev('d', 'x', ago(7))];
    const outside = [ev('e', 'x', ago(30)), ev('f', 'x', ago(45)), ev('g', 'x', new Date(2026, 8, 18, 1, 0))];
    const cells = buildDensity([...inside, ...outside], now, null);
    expect(cells.reduce((sum, c) => sum + c.count, 0)).toBe(inside.length);
    expect(cells[29].count).toBe(2);
    expect(cells[22].count).toBe(1);
  });

  it('colours only the days the agent who is present has a memory on', () => {
    const cells = buildDensity([ev('mine', 'me', ago(3)), ev('theirs', 'you', ago(4))], now, 'me');
    expect(cells.filter((c) => c.present).map((c) => c.date.getDate())).toEqual([ago(3).getDate()]);
    expect(cells[26].count).toBe(1);
  });

  it('colours nothing when nobody is present', () => {
    const cells = buildDensity([ev('mine', 'me', ago(3))], now, null);
    expect(cells.some((c) => c.present)).toBe(false);
  });
});

describe('how wide a day of the band is drawn', () => {
  it('keeps the three widths the mock draws while they fit', () => {
    // The mock's own window: the busiest day held two.
    expect(densityWidth(0, 2)).toBe('4%');
    expect(densityWidth(1, 2)).toBe('30%');
    expect(densityWidth(2, 2)).toBe('60%');
    expect(densityWidth(3, 3)).toBe('90%');
  });

  it('compresses the scale rather than pegging a busy window at full width', () => {
    // Eight a day: 30 % each would put every day of four or more at the end of
    // the band, and the band would stop telling them apart.
    expect(densityWidth(8, 8)).toBe('96%');
    expect(densityWidth(4, 8)).toBe('48%');
    expect(densityWidth(1, 8)).toBe('12%');
  });

  it('draws nothing as a stub, whatever the window holds', () => {
    expect(densityWidth(0, 8)).toBe('4%');
    expect(densityWidth(0, 0)).toBe('4%');
  });
});

describe('reading the time a memory carries', () => {
  it('reads a time written without a zone as the memory server writes it, in UTC', () => {
    expect(parseMemoryTime('2026-09-17 05:03:22')?.toISOString()).toBe('2026-09-17T05:03:22.000Z');
    expect(parseMemoryTime('2026-09-17T05:03:22')?.toISOString()).toBe('2026-09-17T05:03:22.000Z');
  });

  it('leaves a time that carries its own offset alone', () => {
    expect(parseMemoryTime('2026-09-17T05:03:22+09:00')?.toISOString()).toBe('2026-09-16T20:03:22.000Z');
    expect(parseMemoryTime('2026-09-17T05:03:22Z')?.toISOString()).toBe('2026-09-17T05:03:22.000Z');
  });

  it('answers nothing when there is nothing to read', () => {
    expect(parseMemoryTime(undefined)).toBeNull();
    expect(parseMemoryTime('')).toBeNull();
    expect(parseMemoryTime('  ')).toBeNull();
    expect(parseMemoryTime('whenever')).toBeNull();
  });
});

describe('narrowing by what was typed', () => {
  it('keeps everything while nothing is typed', () => {
    expect(matchesSearch(['anything'], '')).toBe(true);
    expect(matchesSearch(['anything'], '   ')).toBe(true);
  });

  it('keeps a row when any one of its words holds the query, whatever the case', () => {
    expect(matchesSearch(['the morning greeting', 'Karin'], 'KARIN')).toBe(true);
    expect(matchesSearch(['the morning greeting', 'Karin'], 'morning')).toBe(true);
  });

  it('drops a row that holds the query nowhere', () => {
    expect(matchesSearch(['the morning greeting', 'Karin', undefined], 'cron')).toBe(false);
  });
});
