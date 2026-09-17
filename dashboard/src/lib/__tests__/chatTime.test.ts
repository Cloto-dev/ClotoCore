import { describe, expect, it } from 'vitest';
import { compactCount, dayBreaks, dayLabel, relativeTime, timeOfDay } from '../chatTime';

// A fixed "now": Wednesday 2026-09-16 15:00 local time.
const NOW = new Date(2026, 8, 16, 15, 0, 0).getTime();
const HOUR = 60 * 60 * 1000;
const DAY = 24 * HOUR;

describe('the day separator', () => {
  it('says today and yesterday by calendar day, else the month and day', () => {
    expect(dayLabel(NOW - HOUR, NOW)).toEqual({ kind: 'today' });
    // 20 hours ago is within 24 hours but on yesterday's calendar day.
    expect(dayLabel(NOW - 20 * HOUR, NOW)).toEqual({ kind: 'yesterday' });
    expect(dayLabel(new Date(2026, 8, 11, 9, 0).getTime(), NOW)).toEqual({ kind: 'date', month: 9, day: 11 });
  });

  it('is drawn above the first turn and above each turn that starts a new day', () => {
    const t = [NOW - 2 * DAY, NOW - 2 * DAY + HOUR, NOW - DAY, NOW - HOUR, NOW];
    expect(dayBreaks(t)).toEqual([true, false, true, true, false]);
    expect(dayBreaks([])).toEqual([]);
  });
});

describe('the clock and the state line', () => {
  it('draws hours and minutes on a 24-hour clock', () => {
    expect(timeOfDay(new Date(2026, 8, 16, 14, 2).getTime(), 'en')).toBe('14:02');
  });

  it('says how long ago in the coarsest unit that fits', () => {
    expect(relativeTime(NOW - 2 * HOUR, NOW, 'en')).toBe('2 hours ago');
    expect(relativeTime(NOW - 5 * 60 * 1000, NOW, 'en')).toBe('5 minutes ago');
    expect(relativeTime(NOW - 3 * DAY, NOW, 'en')).toBe('3 days ago');
  });
});

describe('the context meter', () => {
  it('writes thousands as k with one decimal below ten thousand', () => {
    expect(compactCount(812)).toBe('812');
    expect(compactCount(18_930)).toBe('19k');
    expect(compactCount(1_240)).toBe('1.2k');
    expect(compactCount(128_000)).toBe('128k');
    expect(compactCount(1_000_000)).toBe('1M');
  });
});
