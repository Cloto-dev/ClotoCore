import { type MutableRefObject, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useLocation, useNavigate, useNavigationType } from 'react-router-dom';
import { hasOverlayTitleBar } from '../lib/tauri';
import './WindowBar.css';

/**
 * The bar at the top of the window — across it, or over the sidebar's column
 * while the sidebar is shown: show or hide the sidebar, back, and forward. It draws nothing else — no product name, no line under it — and
 * everything in it that is not a button is what the window is dragged by.
 *
 * Where the OS lays its window buttons over the page (macOS) the controls
 * start to the right of them; elsewhere the OS has its own title bar above
 * and the controls start at the edge.
 */

const ICONS = {
  sidebar: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round">
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <path d="M9 4v16" />
    </svg>
  ),
  back: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round">
      <path d="m15 6-6 6 6 6" />
    </svg>
  ),
  forward: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round">
      <path d="m9 6 6 6-6 6" />
    </svg>
  ),
};

/** The router's position in this window's history, as it records it. */
function historyIndex(): number {
  const idx = (window.history.state as { idx?: unknown } | null)?.idx;
  return typeof idx === 'number' ? idx : 0;
}

interface WindowBarProps {
  sidebarShown: boolean;
  onToggleSidebar: () => void;
  /** The immersive view takes the whole window: the bar keeps holding the window, without controls. */
  immersive: boolean;
  /**
   * Where the furthest history index is kept. The layout moves the bar between the sidebar's column
   * and the top of the window, which mounts it anew, so the layout holds this and the way forward
   * survives the move. Without it the bar keeps its own.
   */
  furthest?: MutableRefObject<number>;
}

export function WindowBar({ sidebarShown, onToggleSidebar, immersive, furthest: held }: WindowBarProps) {
  const { t } = useTranslation('nav');
  const navigate = useNavigate();
  const location = useLocation();
  const navigationType = useNavigationType();

  // The browser does not say whether there is anywhere forward to go, so the
  // furthest index reached is remembered. Going somewhere new from an earlier
  // entry discards what was ahead of it, and the furthest falls back with it.
  const own = useRef(0);
  const furthest = held ?? own;
  const [canGoBack, setCanGoBack] = useState(false);
  const [canGoForward, setCanGoForward] = useState(false);

  // location.key is the trigger: the index is read from history, which changes with it.
  useEffect(() => {
    const idx = historyIndex();
    furthest.current = navigationType === 'PUSH' ? idx : Math.max(furthest.current, idx);
    setCanGoBack(idx > 0);
    setCanGoForward(idx < furthest.current);
  }, [location.key, navigationType]);

  if (immersive) {
    return hasOverlayTitleBar ? (
      <div className="winbar overlay" data-testid="window-bar" data-tauri-drag-region="" />
    ) : null;
  }

  return (
    <div className={`winbar${hasOverlayTitleBar ? ' overlay' : ''}`} data-testid="window-bar">
      <div className={`winbar-lead${sidebarShown ? ' over-side' : ''}`} data-tauri-drag-region="">
        <button
          type="button"
          onClick={onToggleSidebar}
          aria-label={sidebarShown ? t('hide_sidebar') : t('show_sidebar')}
          title={sidebarShown ? t('hide_sidebar') : t('show_sidebar')}
          aria-pressed={sidebarShown}
        >
          {ICONS.sidebar}
        </button>
        <button
          type="button"
          onClick={() => navigate(-1)}
          disabled={!canGoBack}
          aria-label={t('go_back')}
          title={t('go_back')}
        >
          {ICONS.back}
        </button>
        <button
          type="button"
          onClick={() => navigate(1)}
          disabled={!canGoForward}
          aria-label={t('go_forward')}
          title={t('go_forward')}
        >
          {ICONS.forward}
        </button>
      </div>
      {/* Over the sidebar the bar is only as wide as the sidebar's column: there is no rest. */}
      {!sidebarShown && <div className="winbar-rest" data-tauri-drag-region="" />}
    </div>
  );
}
