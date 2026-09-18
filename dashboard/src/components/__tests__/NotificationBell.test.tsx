import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// Echo i18n keys so the assertions do not depend on copy.
vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (k: string, o?: Record<string, unknown>) => (o?.count !== undefined ? `${k}:${String(o.count)}` : k),
  }),
}));

const { getNotificationSummary, getNotifications, markNotificationRead, answerNotification } = vi.hoisted(() => ({
  getNotificationSummary: vi.fn(),
  getNotifications: vi.fn(),
  markNotificationRead: vi.fn(),
  answerNotification: vi.fn(),
}));
vi.mock('../../hooks/useApi', () => ({
  useApi: () => ({ getNotificationSummary, getNotifications, markNotificationRead, answerNotification }),
}));

import { MemoryRouter, Route, Routes, useLocation } from 'react-router-dom';
import { inAppLink, NotificationBell } from '../NotificationBell';

/** Where the router is now, rendered so a test can read it. */
function Where() {
  const loc = useLocation();
  return <span data-testid="where">{loc.pathname + loc.search + loc.hash}</span>;
}

function renderBell() {
  return render(
    <MemoryRouter initialEntries={['/']}>
      <NotificationBell />
      <Routes>
        <Route path="*" element={<Where />} />
      </Routes>
    </MemoryRouter>,
  );
}

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
  answerNotification.mockResolvedValue(undefined);
});

afterEach(() => {
  localStorage.clear();
});

describe('NotificationBell', () => {
  it('shows nothing when nothing is waiting', async () => {
    renderBell();
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

    renderBell();

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

    renderBell();
    fireEvent.click(await screen.findByRole('button', { name: 'notifications.open' }));

    expect(await screen.findByTestId('will-interrupt')).toBeTruthy();
  });

  it('does not clear the badge when an item is only read', async () => {
    getNotificationSummary.mockResolvedValue({ waiting: 1, blocking: 1 });
    getNotifications.mockResolvedValue([item()]);

    renderBell();
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

    renderBell();
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
      <MemoryRouter>
        <div data-testid="header" className="relative z-10">
          <NotificationBell />
        </div>
      </MemoryRouter>,
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

    renderBell();
    fireEvent.click(await screen.findByRole('button', { name: 'notifications.open' }));

    const panel = (await screen.findAllByTestId('notification-item'))[0].closest('.select-text');
    expect(panel).not.toBeNull();
    const classes = (panel as HTMLElement).className;
    expect(classes).toContain('bg-surface-primary');
    expect(classes).not.toMatch(/\bbg-glass\b/);
  });

  it('keeps showing the last known count when a poll fails', async () => {
    getNotificationSummary.mockResolvedValueOnce({ waiting: 2, blocking: 0 });
    renderBell();
    expect((await screen.findByTestId('notification-badge')).textContent).toBe('2');

    getNotificationSummary.mockRejectedValue(new Error('offline'));
    // A failed poll must not throw the count away: a bell that drops to zero
    // when the network hiccups reports "nothing is waiting", which is a
    // different and much worse claim than "I could not check".
    await waitFor(() => expect(screen.getByTestId('notification-badge').textContent).toBe('2'));
  });

  it('offers a way back to the card for a request that is holding an agent', async () => {
    // The deck stops showing a card after half a minute. That is only a
    // courtesy rather than a second deadline because this list leads back to it
    // — without this button, an unanswered request would be visible here and
    // answerable nowhere.
    getNotificationSummary.mockResolvedValue({ waiting: 1, blocking: 1 });
    getNotifications.mockResolvedValue([item({ blocking: true, kind: 'approval' })]);
    const raised: string[] = [];
    const onRaise = (e: Event) => raised.push((e as CustomEvent).detail?.approvalId);
    window.addEventListener('cloto-raise-approval', onRaise);

    try {
      renderBell();
      fireEvent.click(await screen.findByRole('button', { name: 'notifications.open' }));
      fireEvent.click(await screen.findByTestId('raise-approval'));

      expect(raised).toEqual(['a1']);
      // The panel gets out of the way of the card it just raised.
      await waitFor(() => expect(screen.queryByTestId('notification-item')).not.toBeInTheDocument());
    } finally {
      window.removeEventListener('cloto-raise-approval', onRaise);
    }
  });

  it('a proposal is answered here, without opening anything', async () => {
    // The opposite of the approval above, and deliberately so: a proposal stops
    // nothing, so making someone open a card to say yes would add a step to the
    // cheap case and train them to click through the expensive one.
    getNotificationSummary.mockResolvedValue({ waiting: 1, blocking: 0 });
    getNotifications.mockResolvedValue([
      item({ item_id: 'p1', kind: 'proposal', blocking: false, title: 'Shall I retire the timer?' }),
    ]);

    renderBell();
    fireEvent.click(await screen.findByRole('button', { name: 'notifications.open' }));
    fireEvent.click(await screen.findByTestId('answer-yes'));

    await waitFor(() => expect(answerNotification).toHaveBeenCalledWith('p1', 'yes'));
    // It leaves the list because it is settled — not because it was seen.
    await waitFor(() => expect(screen.queryByTestId('notification-item')).not.toBeInTheDocument());
  });

  it('a proposal carries no way to raise a card, and an approval no way to answer in place', async () => {
    getNotificationSummary.mockResolvedValue({ waiting: 2, blocking: 1 });
    getNotifications.mockResolvedValue([
      item({ item_id: 'p1', kind: 'proposal', blocking: false }),
      item({ item_id: 'a1', kind: 'approval', blocking: true }),
    ]);

    renderBell();
    fireEvent.click(await screen.findByRole('button', { name: 'notifications.open' }));
    await screen.findAllByTestId('notification-item');

    // One of each, never two of either: the surface a destructive command is
    // approved on stays single.
    expect(screen.getAllByTestId('answer-yes')).toHaveLength(1);
    expect(screen.getAllByTestId('raise-approval')).toHaveLength(1);
  });

  it('offers no such button for an item nothing is waiting on', async () => {
    getNotificationSummary.mockResolvedValue({ waiting: 1, blocking: 0 });
    getNotifications.mockResolvedValue([item({ blocking: false, kind: 'notice' })]);

    renderBell();
    fireEvent.click(await screen.findByRole('button', { name: 'notifications.open' }));
    await screen.findByTestId('notification-item');

    expect(screen.queryByTestId('raise-approval')).not.toBeInTheDocument();
  });
});

describe('a notice that links somewhere in the app', () => {
  it('offers to open the page it names and goes there', async () => {
    getNotificationSummary.mockResolvedValue({ waiting: 1, blocking: 0 });
    getNotifications.mockResolvedValue([
      item({
        item_id: 'hub-access:expiry:T1:2026-11-19',
        kind: 'notice',
        blocking: false,
        metadata: { link: '/settings?section=security#hub-access' },
      }),
    ]);
    renderBell();
    fireEvent.click(await screen.findByRole('button', { name: 'notifications.open' }));
    fireEvent.click(await screen.findByTestId('open-link'));
    await waitFor(() => expect(screen.getByTestId('where').textContent).toBe('/settings?section=security#hub-access'));
  });

  it('accepts only a path inside this app', () => {
    expect(inAppLink({ link: '/settings?section=security' })).toBe('/settings?section=security');
    for (const bad of ['https://evil.example/', '//evil.example/x', '/\\evil.example', 'settings', 42]) {
      expect(inAppLink({ link: bad })).toBeNull();
    }
    expect(inAppLink(null)).toBeNull();
    expect(inAppLink({})).toBeNull();
  });
});
