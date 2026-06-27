import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { type PillOption, PillSelect } from '../PillSelect';

type V = 'a' | 'b' | 'c';
const OPTIONS: PillOption<V>[] = [
  { value: 'a', label: 'Alpha', hint: 'first' },
  { value: 'b', label: 'Bravo', hint: 'second' },
  { value: 'c', label: 'Charlie' },
];

describe('PillSelect', () => {
  it('renders the selected option label on the pill', () => {
    render(<PillSelect value="b" options={OPTIONS} onSelect={() => {}} />);
    expect(screen.getByRole('button', { name: /Bravo/ })).toBeInTheDocument();
    // Popover is closed: other options not yet rendered.
    expect(screen.queryByText('Alpha')).not.toBeInTheDocument();
  });

  it('opens the popover and lists every option with its hint', () => {
    render(<PillSelect value="a" options={OPTIONS} onSelect={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: /Alpha/ }));
    expect(screen.getByText('Bravo')).toBeInTheDocument();
    expect(screen.getByText('Charlie')).toBeInTheDocument();
    expect(screen.getByText('second')).toBeInTheDocument();
  });

  it('calls onSelect with the chosen value and closes the popover', () => {
    const onSelect = vi.fn();
    render(<PillSelect value="a" options={OPTIONS} onSelect={onSelect} />);
    fireEvent.click(screen.getByRole('button', { name: /Alpha/ }));
    fireEvent.click(screen.getByText('Charlie'));
    expect(onSelect).toHaveBeenCalledExactlyOnceWith('c');
    // Popover closed again.
    expect(screen.queryByText('Bravo')).not.toBeInTheDocument();
  });

  it('does not open when disabled', () => {
    const onSelect = vi.fn();
    render(<PillSelect value="a" options={OPTIONS} onSelect={onSelect} disabled />);
    const btn = screen.getByRole('button', { name: /Alpha/ });
    expect(btn).toBeDisabled();
    fireEvent.click(btn);
    expect(screen.queryByText('Bravo')).not.toBeInTheDocument();
    expect(onSelect).not.toHaveBeenCalled();
  });

  it('closes the popover on Escape', () => {
    render(<PillSelect value="a" options={OPTIONS} onSelect={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: /Alpha/ }));
    expect(screen.getByText('Bravo')).toBeInTheDocument();
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(screen.queryByText('Bravo')).not.toBeInTheDocument();
  });
});
