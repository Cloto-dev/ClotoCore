import { Cpu, HelpCircle, Settings } from 'lucide-react';
import { lazy, Suspense, useEffect, useRef, useState } from 'react';
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
import { ViewHeader } from './ViewHeader';

const SettingsView = lazy(() => import('./SettingsView').then((m) => ({ default: m.SettingsView })));

export interface AppOutletContext {
  setImmersive: (v: boolean) => void;
}

export function AppLayout() {
  const { t } = useTranslation('common');
  const { t: tNav } = useTranslation('nav');
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsInitialSection, setSettingsInitialSection] = useState<'general' | 'about'>('general');
  const [helpOpen, setHelpOpen] = useState(false);
  const [immersive, setImmersive] = useState(false);
  const navigate = useNavigate();
  const location = useLocation();
  const { agents, setSelectedAgentId } = useAgentContext();
  const isAgentRoute = location.pathname === '/';

  const activeCount = agents.filter((a) => a.enabled).length;

  // Track navigation history for back/forward button states
  const maxIdxRef = useRef(0);
  const [canGoBack, setCanGoBack] = useState(false);
  const [canGoForward, setCanGoForward] = useState(false);

  useEffect(() => {
    const idx = ((window.history.state as Record<string, unknown>)?.idx as number) ?? 0;
    maxIdxRef.current = Math.max(maxIdxRef.current, idx);
    setCanGoBack(idx > 0);
    setCanGoForward(idx < maxIdxRef.current);
  }, []);

  // Close settings and navigate home when quick setup completes
  useEffect(() => {
    const handler = () => {
      setSettingsOpen(false);
      navigate('/');
    };
    window.addEventListener('cloto-setup-rerun-complete', handler);
    return () => window.removeEventListener('cloto-setup-rerun-complete', handler);
  }, [navigate]);

  // Open settings (optionally to About section) when update button is clicked
  useEffect(() => {
    const handler = (e: Event) => {
      const section = (e as CustomEvent).detail?.section ?? 'general';
      setSettingsInitialSection(section);
      setSettingsOpen(true);
    };
    window.addEventListener('cloto-open-settings', handler);
    return () => window.removeEventListener('cloto-open-settings', handler);
  }, []);

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
          {/* 1. ViewHeader — first child, full width */}
          {!immersive && (
            <ViewHeader
              icon={Cpu}
              title="ClotoCore"
              onHelp={() => setHelpOpen(true)}
              navBack={() => navigate(-1)}
              navForward={() => navigate(1)}
              canGoBack={canGoBack}
              canGoForward={canGoForward}
              right={
                <span className="text-xs font-mono text-content-tertiary">
                  {activeCount} / {agents.length} Active
                </span>
              }
            />
          )}

          {/* 2. Body — second child, sidebar + content */}
          <div className="flex flex-1 overflow-hidden relative">
            {!immersive && (
              <div className="relative z-10">
                <AppSidebar onSettingsClick={() => setSettingsOpen(true)} />
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

          {/* Settings modal */}
          {settingsOpen && (
            <Modal
              title={tNav('settings')}
              icon={Settings}
              size="lg"
              onClose={() => {
                setSettingsOpen(false);
                setSettingsInitialSection('general');
              }}
            >
              <Suspense
                fallback={
                  <div className="flex items-center justify-center h-full text-xs font-mono text-content-tertiary">
                    {t('loading')}
                  </div>
                }
              >
                <SettingsView initialSection={settingsInitialSection} />
              </Suspense>
            </Modal>
          )}

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
