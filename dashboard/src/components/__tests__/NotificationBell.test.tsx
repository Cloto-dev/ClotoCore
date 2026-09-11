import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// Echo i18n keys so the assertions do not depend on copy.
vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (k: string, o?: Record<string, unknown>) => (o?.count !== undefined ? `${k}:${String(o.count)}` : k),
  }),
}));

const { getNotificationSummary, getNotifications, markNotificationRead } = vi.hoisted(() => ({
  getNotificationSummary: vi.fn(),
  getNotifications: vi.fn(),
  markNotificationRead: vi.fn(),
}));
vi.mock('../../hooks/useApi', () => ({
  useApi: () => ({ getNotificationSummary, getNotifications, markNotificationRead }),
}));

import { NotificationBell } from '../NotificationBell';

function item(over: Partial<Record<string, unknown>> = {}) {
  return {
    item_id: 'a1',
    kind: 'approval',
    severity: 'warning',
    agent_id: 'agent.test',
    title: 'A command is waiting',
    body: 'rm -rf /tmp/scratch',
    created_at: '2026-09-11T00:00:00Z',
    read_at: null,
    resolved_at: null,
    decision: null,
    blocking: true,
    metadata: null,
    ...over,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  getNotificationSummary.mockResolvedValue({ waiting: 0, blocking: 0 });
  getNotifications.mockResolvedValue([]);
  markNotificationRead.mockResolvedValue(undefined);
});

afterEach(() => {
  localStorage.clear();
});

describe('NotificationBell', () => {
  it('shows nothing when nothing is waiting', async () => {
    render(<NotificationBell />);
    await waitFor(() => expect(getNotificationSummary).toHaveBeenCalled());
    expect(screen.queryByTestId('notification-badge')).toBeNull();
  });

  it('counts a blocked agent on the badge even at the strictest threshold', async () => {
    // The rule this guards: a threshold decides whether an item interrupts,
    // never whether it can be found. With "high only" chosen and a medium-level
    // item holding an agent, the badge must still report it — otherwise the
    // setting is a switch that starves agents silently, and the only symptom is
    // an agent that never finishes.
    localStorage.setItem('cloto-notification-threshold', 'high');
    getNotificationSummary.mockResolvedValue({ waiting: 1, blocking: 1 });
    getNotifications.mockResolvedValue([item({ severity: 'warning' })]);

    render(<NotificationBell />);

    const badge = await screen.findByTestId('notification-badge');
    expect(badge.textContent).toBe('1');

    fireEvent.click(screen.getByRole('button', { name: 'notifications.open' }));

    // Listed as well — the panel is not filtered either.
    const items = await screen.findAllByTestId('notification-item');
    expect(items).toHaveLength(1);
    // ...but marked as something that will NOT interrupt at this threshold,
    // which is the whole and only job the setting has.
    expect(screen.queryByTestId('will-interrupt')).toBeNull();
  });

  it('marks an item as interrupting when the threshold allows it', async () => {
    localStorage.setItem('cloto-notification-threshold', 'medium');
    getNotificationSummary.mockResolvedValue({ waiting: 1, blocking: 1 });
    getNotifications.mockResolvedValue([item({ severity: 'warning' })]);

    render(<NotificationBell />);
    fireEvent.click(await screen.findByRole('button', { name: 'notifications.open' }));

    expect(await screen.findByTestId('will-interrupt')).toBeTruthy();
  });

  it('does not clear the badge when an item is only read', async () => {
    getNotificationSummary.mockResolvedValue({ waiting: 1, blocking: 1 });
    getNotifications.mockResolvedValue([item()]);

    render(<NotificationBell />);
    fireEvent.click(await screen.findByRole('button', { name: 'notifications.open' }));

    fireEvent.click(await screen.findByText('notifications.mark_read'));
    await waitFor(() => expect(markNotificationRead).toHaveBeenCalledWith('a1'));

    // Seeing that you were asked is not answering. A badge that cleared here
    // would say the agent is free while it is still held.
    expect(screen.getByTestId('notification-badge').textContent).toBe('1');
  });

  it('keeps its contents selectable despite the drag region around it', async () => {
    // The header is `select-none` so the window can be dragged by it, and
    // `user-select` inherits. An audit id or a command line nobody can copy is
    // the reason this override exists.
    getNotificationSummary.mockResolvedValue({ waiting: 1, blocking: 0 });
    getNotifications.mockResolvedValue([item({ blocking: false })]);

    render(<NotificationBell />);
    fireEvent.click(await screen.findByRole('button', { name: 'notifications.open' }));

    const panel = (await screen.findAllByTestId('notification-item'))[0].closest('.select-text');
    expect(panel).not.toBeNull();
  });

  it('renders outside the header it is mounted in', async () => {
    // Shipped trapped once. The header carries `relative z-10`, which makes it
    // a stacking context, and `<main>` is a later sibling with the same z-10 —
    // so everything in the header paints under the page content whatever
    // z-index it claims, and the panel came out from behind the agent cards.
    // jsdom cannot see paint order, but it can see the tree: a panel that is
    // still a descendant of the header is a panel that is still trapped.
    getNotificationSummary.mockResolvedValue({ waiting: 1, blocking: 0 });
    getNotifications.mockResolvedValue([item({ blocking: false })]);

    const { container } = render(
      <div data-testid="header" className="relative z-10">
        <NotificationBell />
      </div>,
    );
    fireEvent.click(await screen.findByRole('button', { name: 'notifications.open' }));

    const panel = (await screen.findAllByTestId('notification-item'))[0].closest('.select-text');
    expect(panel).not.toBeNull();
    const header = screen.getByTestId('header');
    expect(header.contains(panel as Node)).toBe(false);
    expect(container.contains(panel as Node)).toBe(false);
    expect(document.body.contains(panel as Node)).toBe(true);
  });

  it('floats on an opaque surface, not a translucent one', async () => {
    // Shipped translucent once: the panel used a glass token (60% alpha in
    // dark) and the agent cards behind it read straight through, so the panel's
    // own text and the page's text were both legible and neither was readable.
    // A panel over a static background can be glass; one floating over content
    // cannot.
    getNotificationSummary.mockResolvedValue({ waiting: 1, blocking: 0 });
    getNotifications.mockResolvedValue([item({ blocking: false })]);

    render(<NotificationBell />);
    fireEvent.click(await screen.findByRole('button', { name: 'notifications.open' }));

    const panel = (await screen.findAllByTestId('notification-item'))[0].closest('.select-text');
    expect(panel).not.toBeNull();
    const classes = (panel as HTMLElement).className;
    expect(classes).toContain('bg-surface-primary');
    expect(classes).not.toMatch(/\bbg-glass\b/);
  });

  it('keeps showing the last known count when a poll fails', async () => {
    getNotificationSummary.mockResolvedValueOnce({ waiting: 2, blocking: 0 });
    render(<NotificationBell />);
    expect((await screen.findByTestId('notification-badge')).textContent).toBe('2');

    getNotificationSummary.mockRejectedValue(new Error('offline'));
    // A failed poll must not throw the count away: a bell that drops to zero
    // when the network hiccups reports "nothing is waiting", which is a
    // different and much worse claim than "I could not check".
    await waitFor(() => expect(screen.getByTestId('notification-badge').textContent).toBe('2'));
  });
});
