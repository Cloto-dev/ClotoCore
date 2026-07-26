import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

// Echo i18n keys (with interpolation) so assertions are deterministic without
// an i18n instance — same approach as RecallSection.test.tsx.
vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (k: string, o?: Record<string, unknown>) => (o ? `${k}:${JSON.stringify(o)}` : k),
  }),
}));

vi.mock('../../../contexts/ApiKeyContext', () => ({
  useApiKey: () => ({ apiKey: 'ambient-key' }),
}));

import { DangerZone } from '../DangerZone';

const TIER_NAMES = ['application', 'user_data', 'assets', 'everything'] as const;

/** A plan for `level`, shaped like the kernel's response for that scope. */
function planFor(level: number) {
  return {
    plan: {
      plan_version: 1,
      app_version: '0.6.8-beta.2',
      generated_at: '2026-07-26T00:00:00Z',
      // The kernel echoes the scope it enumerated; the UI reads the tier back
      // from here rather than trusting its own request.
      tier: TIER_NAMES[level - 1],
      data_dir: '/home/u/.clotocore',
      entries: [
        {
          id: 'db',
          kind: 'file',
          path: '/home/u/.clotocore/cloto.db',
          tier: 'user_data',
          source: 'receipt',
          size_bytes: 2048,
          secret: true,
        },
        { id: 'svc', kind: 'service', name: 'clotocore.service', tier: 'application', source: 'platform' },
      ],
      skipped: [{ id: 'models', reason: 'absent', path: '/home/u/.clotocore/models' }],
      notes: ['A path that is not valid UTF-8 cannot be written down.'],
    },
    summary: {
      entries: 2,
      skipped: 1,
      total_bytes: 2048,
      total_truncated: false,
      contains_secret: true,
      needs_elevation: true,
    },
  };
}

function tierOf(url: string): number {
  const match = /[?&]tier=(\d+)/.exec(url);
  return match ? Number(match[1]) : 1;
}

describe('DangerZone gates', () => {
  let fetchMock: ReturnType<typeof vi.fn>;
  /** Scopes whose enumeration should fail, to exercise the stale-plan guard. */
  let failPlanFor: Set<number>;

  beforeEach(() => {
    failPlanFor = new Set();
    fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
      const target = String(url);
      if (target.includes('/system/uninstall/plan')) {
        const level = tierOf(target);
        if (failPlanFor.has(level)) {
          return {
            ok: false,
            json: async () => ({ error: { message: 'enumeration exploded' } }),
          } as unknown as Response;
        }
        return { ok: true, json: async () => ({ data: planFor(level) }) } as unknown as Response;
      }
      if (target.includes('/system/uninstall')) {
        return {
          ok: true,
          json: async () => ({
            data: {
              status: 'uninstalling',
              plan: planFor(2).plan,
              report_path: '/tmp/x/purge-plan.json.report.json',
            },
          }),
        } as unknown as Response;
      }
      throw new Error(`unexpected fetch ${target} ${JSON.stringify(init)}`);
    });
    vi.stubGlobal('fetch', fetchMock);
  });

  it('walks gate 1 → 2 → 3 and stops after the handoff', async () => {
    render(<DangerZone />);

    // Gate 1: nothing is fetched until the user asks to review.
    expect(fetchMock).not.toHaveBeenCalled();
    fireEvent.click(screen.getByText('health.danger.review'));

    await waitFor(() => expect(screen.getByText('/home/u/.clotocore/cloto.db')).toBeTruthy());
    expect(fetchMock.mock.calls[0][0]).toContain('/system/uninstall/plan?tier=1');
    expect((fetchMock.mock.calls[0][1] as RequestInit).headers).toMatchObject({ 'X-API-Key': 'ambient-key' });

    // Flags, verbatim notes, skipped candidates all rendered.
    expect(screen.getByText('health.danger.flag_secret')).toBeTruthy();
    expect(screen.getByText('health.danger.summary_elevation')).toBeTruthy();
    expect(screen.getByText('A path that is not valid UTF-8 cannot be written down.')).toBeTruthy();
    expect(screen.getByText('clotocore.service')).toBeTruthy();
    fireEvent.click(screen.getByText(/skipped_show/));
    expect(screen.getByText('health.danger.skip_absent')).toBeTruthy();

    // Gate 2: widening the scope re-reads the plan.
    const boxes = screen.getAllByRole('checkbox') as HTMLInputElement[];
    expect(boxes[0].checked).toBe(true);
    expect(boxes[0].disabled).toBe(true);
    expect(boxes[2].checked).toBe(false);
    fireEvent.click(boxes[2]); // tier 3
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
    expect(fetchMock.mock.calls[1][0]).toContain('tier=3');
    await waitFor(() => expect((screen.getAllByRole('checkbox') as HTMLInputElement[])[1].checked).toBe(true));

    // Unchecking tier 3 lands on tier 2 (cumulative semantics).
    fireEvent.click((screen.getAllByRole('checkbox') as HTMLInputElement[])[2]);
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(3));
    expect(fetchMock.mock.calls[2][0]).toContain('tier=2');

    // Gate 3: execute is armed only by a typed key, and uses that key.
    const execBtn = screen.getByText('health.danger.execute').closest('button') as HTMLButtonElement;
    expect(execBtn.disabled).toBe(true);
    fireEvent.change(screen.getByPlaceholderText('health.danger.sudo_placeholder'), {
      target: { value: 'typed-key' },
    });
    await waitFor(() => expect(execBtn.disabled).toBe(false));
    fireEvent.click(execBtn);
    fireEvent.click(screen.getByText('health.danger.confirm_label'));

    await waitFor(() => expect(screen.getByText('/tmp/x/purge-plan.json.report.json')).toBeTruthy());
    const post = fetchMock.mock.calls[3];
    expect(post[0]).toContain('/system/uninstall');
    expect((post[1] as RequestInit).method).toBe('POST');
    expect((post[1] as RequestInit).body).toBe(JSON.stringify({ tier: 2 }));
    expect((post[1] as RequestInit).headers).toMatchObject({ 'X-API-Key': 'typed-key' });

    // Terminal: no polling, no re-scan after the handoff.
    const after = fetchMock.mock.calls.length;
    await new Promise((r) => setTimeout(r, 300));
    expect(fetchMock.mock.calls.length).toBe(after);
    expect(screen.getByText('health.danger.running_title')).toBeTruthy();
  });

  it('refuses to execute a scope the rendered plan does not cover', async () => {
    // Widening succeeds at tier 1 and fails at tier 4: the tier-1 enumeration
    // stays on screen while the selected scope is tier 4. Executing then would
    // remove things the user was never shown, so gate 3 must close.
    failPlanFor.add(4);
    render(<DangerZone />);
    fireEvent.click(screen.getByText('health.danger.review'));
    await waitFor(() => expect(screen.getByText('/home/u/.clotocore/cloto.db')).toBeTruthy());

    // Arm gate 3 legitimately at tier 1 first.
    fireEvent.change(screen.getByPlaceholderText('health.danger.sudo_placeholder'), {
      target: { value: 'typed-key' },
    });
    const execBtn = screen.getByText('health.danger.execute').closest('button') as HTMLButtonElement;
    await waitFor(() => expect(execBtn.disabled).toBe(false));

    fireEvent.click((screen.getAllByRole('checkbox') as HTMLInputElement[])[3]); // tier 4
    await waitFor(() => expect(screen.getByText('health.danger.scope_stale')).toBeTruthy());

    // The tier-1 list is still rendered, but nothing can be executed against
    // it, and the key entry is gone with the scope it was typed for.
    expect(screen.getByText('/home/u/.clotocore/cloto.db')).toBeTruthy();
    expect(screen.queryByPlaceholderText('health.danger.sudo_placeholder')).toBeNull();
    expect((screen.getByText('health.danger.execute').closest('button') as HTMLButtonElement).disabled).toBe(true);
    // Only the two plan reads happened — no uninstall was posted.
    expect(fetchMock.mock.calls.every((c) => String(c[0]).includes('/system/uninstall/plan'))).toBe(true);
  });
});
