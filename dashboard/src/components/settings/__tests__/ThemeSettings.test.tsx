import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import '../../../i18n';
import type { LoadedTheme } from '../../../themes/load';
import type { Theme } from '../../../themes/validate';

const stub = vi.hoisted(() => ({
  theme: {
    face: 'dark' as const,
    mode: 'dark' as 'light' | 'dark' | 'system',
    setMode: vi.fn(),
    themeId: 'one',
    setThemeId: vi.fn(),
    themes: [] as LoadedTheme[],
    rejected: [] as { name: string; errors: string[] }[],
    importPack: vi.fn(),
    removePack: vi.fn(async () => {}),
  },
}));
vi.mock('../../../hooks/useTheme', () => ({ useTheme: () => stub.theme }));

import { ThemePackGroup, ThemeRows } from '../ThemeSettings';

function loaded(
  id: string,
  label: string,
  over: Partial<LoadedTheme> = {},
  faces: string[] = ['light', 'dark'],
): LoadedTheme {
  const theme = { id, label, accent: null, faces: Object.fromEntries(faces.map((f) => [f, {}])) } as unknown as Theme;
  return { theme, warnings: [], source: 'bundled', ...over };
}

beforeEach(() => {
  vi.clearAllMocks();
  stub.theme.themeId = 'one';
  stub.theme.mode = 'dark';
  stub.theme.rejected = [];
  stub.theme.themes = [
    loaded('one', 'One'),
    loaded('two', 'Two', { warnings: ['light: text-tertiary on surface-base 2.45:1'] }),
  ];
});

describe('the theme rows', () => {
  it('offer what the loader found, marking a theme with faint text', () => {
    render(<ThemeRows />);
    fireEvent.click(screen.getByRole('button', { name: 'Theme' }));
    expect(screen.getAllByRole('option').map((o) => o.textContent)).toEqual(['One', 'Two (low contrast)']);
  });

  it('switch the theme and the mode through the hook', () => {
    render(<ThemeRows />);
    fireEvent.click(screen.getByRole('button', { name: 'Theme' }));
    fireEvent.pointerDown(screen.getByRole('option', { name: 'Two (low contrast)' }));
    expect(stub.theme.setThemeId).toHaveBeenCalledWith('two');
    fireEvent.click(screen.getByRole('button', { name: 'System' }));
    expect(stub.theme.setMode).toHaveBeenCalledWith('system');
  });

  it('say so when the theme on screen has one face only', () => {
    stub.theme.themes = [loaded('one', 'One', {}, ['dark'])];
    render(<ThemeRows />);
    expect(screen.getByText('This theme is Dark only.')).toBeInTheDocument();
  });
});

describe('the theme pack group', () => {
  it('lists external packs with a way to remove them, and never the bundled ones', () => {
    stub.theme.themes = [loaded('one', 'One'), loaded('night', 'Night', { source: 'browser' })];
    render(<ThemePackGroup />);
    expect(screen.queryByText('One')).not.toBeInTheDocument();
    expect(screen.getByText('Night')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Remove' }));
    expect(stub.theme.removePack).toHaveBeenCalledWith('night');
  });

  it('says why a pack in the directory was not loaded', () => {
    stub.theme.rejected = [{ name: 'broken', errors: ['dark: missing "text-primary"'] }];
    render(<ThemePackGroup />);
    expect(screen.getByText('"broken" was not loaded: dark: missing "text-primary"')).toBeInTheDocument();
  });

  it('shows the reasons when an imported file is refused', async () => {
    stub.theme.importPack.mockRejectedValue(new Error('unknown key "css"'));
    const { container } = render(<ThemePackGroup />);
    const input = container.querySelector('input[type="file"]') as HTMLInputElement;
    fireEvent.change(input, { target: { files: [new File(['{"css":1}'], 'x.json', { type: 'application/json' })] } });
    await waitFor(() => expect(screen.getByText('This theme can\'t be used: unknown key "css"')).toBeInTheDocument());
    expect(stub.theme.importPack).toHaveBeenCalledWith('{"css":1}');
  });

  it('says which theme was imported', async () => {
    stub.theme.importPack.mockResolvedValue(loaded('night', 'Night', { source: 'browser' }));
    const { container } = render(<ThemePackGroup />);
    const input = container.querySelector('input[type="file"]') as HTMLInputElement;
    fireEvent.change(input, { target: { files: [new File(['{}'], 'x.json')] } });
    await waitFor(() => expect(screen.getByText('"Night" imported and applied.')).toBeInTheDocument());
  });
});
