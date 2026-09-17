import { HelpCircle } from 'lucide-react';
import { Suspense, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Outlet, useLocation, useNavigate } from 'react-router-dom';
import { ActionsProvider } from '../contexts/ActionsContext';
import { useAgentContext } from '../contexts/AgentContext';
import { ConversationProvider } from '../contexts/ConversationContext';
import { isExperimentalBuild } from '../lib/tauri';
import { AgentPage } from '../pages/AgentPage';
import { AppSidebar } from './AppSidebar';
import { CommandApprovalDeck } from './CommandApprovalDeck';
import { HelpContent } from './HelpContent';
import { Modal } from './Modal';
import { SecurityGuard } from './SecurityGuard';
import { WindowBar } from './WindowBar';

const SIDEBAR_HIDDEN_KEY = 'cloto-sidebar-hidden';

/** A convenience, not state anything depends on: a browser that refuses storage just starts with the sidebar shown. */
function readSidebarHidden(): boolean {
  try {
    return window.localStorage.getItem(SIDEBAR_HIDDEN_KEY) === '1';
  } catch {
    return false;
  }
}

export interface AppOutletContext {
  setImmersive: (v: boolean) => void;
}

export function AppLayout() {
  const { t } = useTranslation('common');
  const { t: tNav } = useTranslation('nav');
  const [helpOpen, setHelpOpen] = useState(false);
  const [immersive, setImmersive] = useState(false);
  const [sidebarHidden, setSidebarHidden] = useState(readSidebarHidden);

  const toggleSidebar = () => {
    const next = !sidebarHidden;
    setSidebarHidden(next);
    try {
      window.localStorage.setItem(SIDEBAR_HIDDEN_KEY, next ? '1' : '0');
    } catch {
      // Not remembered across launches; the toggle itself still works.
    }
  };
  const navigate = useNavigate();
  const location = useLocation();
  const { agents, setSelectedAgentId } = useAgentContext();
  const isAgentRoute = location.pathname === '/';

  // Quick setup, re-run from Settings, ends at the living room.
  useEffect(() => {
    const handler = () => navigate('/');
    window.addEventListener('cloto-setup-rerun-complete', handler);
    return () => window.removeEventListener('cloto-setup-rerun-complete', handler);
  }, [navigate]);

  // The update notice asks for Settings, at About. Settings is a page, so the
  // section it wants travels in the URL rather than in a prop.
  useEffect(() => {
    const handler = (e: Event) => {
      const section = (e as CustomEvent).detail?.section ?? 'general';
      navigate(`/settings?section=${encodeURIComponent(section)}`);
    };
    window.addEventListener('cloto-open-settings', handler);
    return () => window.removeEventListener('cloto-open-settings', handler);
  }, [navigate]);

  const handleAskAgent = () => {
    setHelpOpen(false);
    navigate('/');
    const defaultAgent = agents.find((a) => a.id === 'agent.cloto_default');
    if (defaultAgent) setSelectedAgentId(defaultAgent.id);
  };

  return (
    <ConversationProvider>
      <ActionsProvider>
        <div className="h-screen bg-surface-base flex flex-col overflow-hidden relative font-sans text-content-primary select-none">
          {/* The window's frame is the OS's; this bar is the page's own: the
              sidebar toggle, back and forward, and otherwise something to hold
              the window by. */}
          <WindowBar sidebarShown={!sidebarHidden} onToggleSidebar={toggleSidebar} immersive={immersive} />

          {/* Body — sidebar + content */}
          <div className="flex flex-1 overflow-hidden relative">
            {!immersive && !sidebarHidden && (
              <div className="relative z-10">
                <AppSidebar onSettingsClick={() => navigate('/settings')} onHelpClick={() => setHelpOpen(true)} />
              </div>
            )}
            <main className="flex-1 h-full overflow-hidden relative z-10">
              {/* AgentPage is always mounted to preserve SSE connections,
              thinking steps, and chat state across navigation.
              Thinking steps also persisted in sessionStorage for reload. */}
              <div className={isAgentRoute ? 'h-full' : 'hidden'}>
                <AgentPage />
              </div>
              {!isAgentRoute && (
                <Suspense
                  fallback={
                    <div className="flex items-center justify-center h-full text-xs font-mono text-content-tertiary">
                      {t('loading')}
                    </div>
                  }
                >
                  <Outlet context={{ setImmersive } satisfies AppOutletContext} />
                </Suspense>
              )}
            </main>
            {/* Experimental-build mark (docs/RELEASE_PIPELINE_DESIGN.md §6): out of
                the sidebar, in the window's bottom-right corner, over nothing
                that matters. Locally derived, no network. */}
            {isExperimentalBuild && (
              <div
                title={tNav('experimental_tooltip')}
                className="pointer-events-none absolute bottom-2 right-3 z-20 text-xs text-amber-500/80 select-none"
              >
                {tNav('experimental')}
              </div>
            )}
          </div>

          {/* Help modal */}
          {helpOpen && (
            <Modal title={t('help.title')} icon={HelpCircle} size="sm" onClose={() => setHelpOpen(false)}>
              <HelpContent onAskAgent={handleAskAgent} />
            </Modal>
          )}

          {/* Both live outside the routed content on purpose: a question an agent
            is blocked on is not about the screen you happen to be on, and the
            immersive view does not get to hide one either. */}
          <CommandApprovalDeck />
          <SecurityGuard />
        </div>
      </ActionsProvider>
    </ConversationProvider>
  );
}
