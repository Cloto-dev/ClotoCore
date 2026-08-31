import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

// Echo i18n keys so assertions are deterministic without an i18n instance —
// same approach as RecallSection.test.tsx.
vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (k: string) => k }),
}));

import { Modal } from '../Modal';
import { type PillOption, PillSelect } from '../ui/PillSelect';

// Escape is dispatched at document.body, where a real keydown lands when
// nothing is focused. Firing at `document` itself would put every listener in
// the at-target phase, which orders them by registration rather than by
// capture — the exact distinction this modal relies on.
const pressEscape = () => fireEvent.keyDown(document.body, { key: 'Escape' });

describe('Modal', () => {
  it('closes on Escape', () => {
    const onClose = vi.fn();
    render(
      <Modal title="Settings" onClose={onClose}>
        body
      </Modal>,
    );
    pressEscape();
    expect(onClose).toHaveBeenCalledOnce();
  });

  it('gives the close button an accessible name', () => {
    render(
      <Modal title="Settings" onClose={() => {}}>
        body
      </Modal>,
    );
    // 'close' is the echoed i18n key; the button carried no name at all before.
    expect(screen.getByRole('button', { name: 'close' })).toBeInTheDocument();
  });

  it('exposes the dialog by its title', () => {
    render(
      <Modal title="Settings" onClose={() => {}}>
        body
      </Modal>,
    );
    expect(screen.getByRole('dialog', { name: 'Settings' })).toBeInTheDocument();
  });

  it('closes only the topmost modal when two are stacked', () => {
    const onCloseOuter = vi.fn();
    const onCloseInner = vi.fn();
    const { rerender } = render(
      <>
        <Modal title="Settings" onClose={onCloseOuter}>
          outer
        </Modal>
        <Modal title="Confirm" onClose={onCloseInner}>
          inner
        </Modal>
      </>,
    );
    pressEscape();
    expect(onCloseInner).toHaveBeenCalledOnce();
    expect(onCloseOuter).not.toHaveBeenCalled();

    // With the confirm dismissed, the next press belongs to the settings modal.
    rerender(
      <Modal title="Settings" onClose={onCloseOuter}>
        outer
      </Modal>,
    );
    pressEscape();
    expect(onCloseOuter).toHaveBeenCalledOnce();
    expect(onCloseInner).toHaveBeenCalledOnce();
  });

  it('does not close when a popover inside it takes the key', () => {
    const onClose = vi.fn();
    const options: PillOption<'a' | 'b'>[] = [
      { value: 'a', label: 'Alpha' },
      { value: 'b', label: 'Bravo' },
    ];
    render(
      <Modal title="Settings" onClose={onClose}>
        <PillSelect value="a" options={options} onSelect={() => {}} />
      </Modal>,
    );
    fireEvent.click(screen.getByRole('button', { name: /Alpha/ }));
    expect(screen.getByText('Bravo')).toBeInTheDocument();

    pressEscape();

    // The popover is gone and the modal it sits in is still open.
    expect(screen.queryByText('Bravo')).not.toBeInTheDocument();
    expect(onClose).not.toHaveBeenCalled();
  });
});
