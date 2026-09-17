import { act, fireEvent, render, screen } from '@testing-library/react';
import { createMemoryRouter, RouterProvider, useNavigate } from 'react-router-dom';
import { describe, expect, it, vi } from 'vitest';

vi.mock('../../lib/tauri', () => ({ hasOverlayTitleBar: false }));
vi.mock('react-i18next', () => ({ useTranslation: () => ({ t: (k: string) => k }) }));

import { WindowBar } from '../WindowBar';

function Shell() {
  const navigate = useNavigate();
  return (
    <>
      <WindowBar sidebarShown onToggleSidebar={() => {}} immersive={false} />
      <button type="button" onClick={() => navigate('/mcp')}>
        to-mcp
      </button>
      <button type="button" onClick={() => navigate('/memory')}>
        to-memory
      </button>
    </>
  );
}

// The memory router keeps its own entries and does not write window.history,
// which is where the bar reads its position from — so the test writes it, as
// the browser router does on every navigation.
function mount() {
  const router = createMemoryRouter([{ path: '*', element: <Shell /> }]);
  window.history.replaceState({ idx: 0 }, '');
  const go = async (fn: () => void, nextIndex: number) => {
    window.history.replaceState({ idx: nextIndex }, '');
    await act(async () => fn());
  };
  render(<RouterProvider router={router} />);
  return { go };
}

const back = () => screen.getByLabelText('go_back') as HTMLButtonElement;
const forward = () => screen.getByLabelText('go_forward') as HTMLButtonElement;

describe('back and forward', () => {
  it('have nowhere to go in a window that has been nowhere', () => {
    mount();
    expect(back().disabled).toBe(true);
    expect(forward().disabled).toBe(true);
  });

  it('follow the history: back after going somewhere, forward after coming back', async () => {
    const { go } = mount();
    await go(() => fireEvent.click(screen.getByText('to-mcp')), 1);
    expect(back().disabled).toBe(false);
    expect(forward().disabled).toBe(true);

    await go(() => fireEvent.click(back()), 0);
    expect(back().disabled).toBe(true);
    expect(forward().disabled).toBe(false);

    await go(() => fireEvent.click(forward()), 1);
    expect(back().disabled).toBe(false);
    expect(forward().disabled).toBe(true);
  });

  it('lose the way forward when somewhere new is opened from an earlier entry', async () => {
    const { go } = mount();
    await go(() => fireEvent.click(screen.getByText('to-mcp')), 1);
    await go(() => fireEvent.click(screen.getByText('to-memory')), 2);
    await go(() => fireEvent.click(back()), 1);
    await go(() => fireEvent.click(back()), 0);
    expect(forward().disabled).toBe(false);
    // A new page from the first entry: the two ahead of it are gone.
    await go(() => fireEvent.click(screen.getByText('to-mcp')), 1);
    expect(forward().disabled).toBe(true);
  });
});

describe('the immersive view', () => {
  it('takes the controls away', () => {
    const router = createMemoryRouter([
      { path: '*', element: <WindowBar sidebarShown onToggleSidebar={() => {}} immersive /> },
    ]);
    render(<RouterProvider router={router} />);
    expect(screen.queryByLabelText('go_back')).toBeNull();
  });
});
