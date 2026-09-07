import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

// Echo i18n keys (with the interpolated count) so assertions do not depend on copy.
vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (k: string, o?: Record<string, unknown>) => (o?.n !== undefined ? `${k}:${String(o.n)}` : k),
  }),
}));

const { fetchDiagnosticsReport, readStoredApiKey } = vi.hoisted(() => ({
  fetchDiagnosticsReport: vi.fn(),
  readStoredApiKey: vi.fn(),
}));
vi.mock('../../services/api', () => ({ fetchDiagnosticsReport, readStoredApiKey }));

import { DiagnosticsModal } from '../DiagnosticsModal';

const writeText = vi.fn();

beforeEach(() => {
  vi.clearAllMocks();
  readStoredApiKey.mockReturnValue('a-key');
  fetchDiagnosticsReport.mockResolvedValue({
    markdown: '**Description**\nthe kernel composed this',
    mode: 'safe',
    masked: 2,
    log_lines: 80,
  });
  writeText.mockResolvedValue(undefined);
  Object.assign(navigator, { clipboard: { writeText } });
});

describe('DiagnosticsModal', () => {
  it('shows the report the kernel composed', async () => {
    render(<DiagnosticsModal message="it broke" onClose={() => {}} />);

    const box = await screen.findByRole('textbox');
    expect((box as HTMLTextAreaElement).value).toContain('the kernel composed this');
    expect(screen.getByText('diagnostics_masked:2')).toBeTruthy();
  });

  it('passes what the UI knows about the failure to the kernel', async () => {
    render(
      <DiagnosticsModal context="Marketplace install" message="it broke" componentStack="at Foo" onClose={() => {}} />,
    );

    await waitFor(() => expect(fetchDiagnosticsReport).toHaveBeenCalled());
    expect(fetchDiagnosticsReport).toHaveBeenCalledWith('a-key', {
      context: 'Marketplace install',
      message: 'it broke',
      component_stack: 'at Foo',
      mode: 'safe',
    });
  });

  // The one that matters: a fallback report has not been through the kernel's
  // masking, and a user who believes otherwise pastes secrets into a public
  // issue. The warning is the feature, not the fallback text.
  it('warns that the fallback carries no masking when the kernel does not answer', async () => {
    fetchDiagnosticsReport.mockRejectedValue(new Error('kernel is gone'));
    render(<DiagnosticsModal message="it broke" onClose={() => {}} />);

    await screen.findByText('diagnostics_kernel_unreachable');
    const box = screen.getByRole('textbox') as HTMLTextAreaElement;
    expect(box.value).toContain('**Description**');
    // No masked count is claimed for text the kernel never saw.
    expect(screen.queryByText(/^diagnostics_masked/)).toBeNull();
  });

  it('does not call the kernel at all when no API key is stored', async () => {
    readStoredApiKey.mockReturnValue('');
    render(<DiagnosticsModal message="it broke" onClose={() => {}} />);

    await screen.findByText('diagnostics_kernel_unreachable');
    expect(fetchDiagnosticsReport).not.toHaveBeenCalled();
  });

  it('re-composes at the full level when the level is switched', async () => {
    render(<DiagnosticsModal message="it broke" onClose={() => {}} />);
    await screen.findByRole('textbox');

    fireEvent.click(screen.getByText('diagnostics_level_full'));

    await waitFor(() =>
      expect(fetchDiagnosticsReport).toHaveBeenLastCalledWith('a-key', expect.objectContaining({ mode: 'full' })),
    );
    expect(screen.getByText('diagnostics_full_warning')).toBeTruthy();
  });

  // The user is the last check before this reaches a public issue, so what they
  // edited is what must be copied — not what the kernel first returned.
  it('copies what the user edited, not the original text', async () => {
    render(<DiagnosticsModal message="it broke" onClose={() => {}} />);
    const box = await screen.findByRole('textbox');

    fireEvent.change(box, { target: { value: 'edited by the reporter' } });
    fireEvent.click(screen.getByText('diagnostics_copy'));

    await waitFor(() => expect(writeText).toHaveBeenCalledWith('edited by the reporter'));
  });

  it('closes when asked', async () => {
    const onClose = vi.fn();
    render(<DiagnosticsModal message="it broke" onClose={onClose} />);
    await screen.findByRole('textbox');

    fireEvent.click(screen.getByText('diagnostics_close'));
    expect(onClose).toHaveBeenCalled();
  });
});
