import { Cpu, HelpCircle, Settings } from 'lucide-react';
import { lazy, Suspense, useEffect, useRef, useState } from 'react';
import { Outlet, useLocation, useNavigate } from 'react-router-dom';
import { useAgentContext } from '../contexts/AgentContext';
import { useLocalStorage } from '../hooks/useStorage';
import { AppSidebar } from './AppSidebar';
import { HelpContent } from './HelpContent';
import { InteractiveGrid } from './InteractiveGrid';
import { Modal } from './Modal';
import { SecurityGuard } from './SecurityGuard';
import { ViewHeader } from './ViewHeader';

const SettingsView = lazy(() => import('./SettingsView').then((m) => ({ default: m.SettingsView })));

export interface AppOutletContext {
  setImmersive: (v: boolean) => void;
}

export function AppLayout() {
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsInitialSection, setSettingsInitialSection] = useState<'general' | 'about'>('general');
  const [helpOpen, setHelpOpen] = useState(false);
  const [immersive, setImmersive] = useState(false);
  const [sidebarRaw, setSidebarRaw] = useLocalStorage('sidebar-collapsed', 'false');
  const sidebarCollapsed = sidebarRaw === 'true';
  const navigate = useNavigate();
  const _location = useLocation();
  const { agents, setSelectedAgentId } = useAgentContext();

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

  const handleToggleSidebar = () => setSidebarRaw(sidebarCollapsed ? 'false' : 'true');

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
    <div className="h-screen bg-surface-base flex flex-col overflow-hidden relative font-sans text-content-primary select-none">
      {/* 1. ViewHeader — first child, full width */}
      {!immersive && (
        <ViewHeader
          icon={Cpu}
          title="Cloto System"
          onHelp={() => setHelpOpen(true)}
          navBack={() => navigate(-1)}
          navForward={() => navigate(1)}
          canGoBack={canGoBack}
          canGoForward={canGoForward}
          right={
            <span className="text-[10px] font-mono text-content-tertiary">
              {activeCount} / {agents.length} Active
            </span>
          }
        />
      )}

      {/* 2. Body — second child, sidebar + content */}
      <div className="flex flex-1 overflow-hidden relative">
        <InteractiveGrid />
        {!immersive && (
          <div className="relative z-10">
            <AppSidebar
              onSettingsClick={() => setSettingsOpen(true)}
              collapsed={sidebarCollapsed}
              onToggleCollapse={handleToggleSidebar}
            />
          </div>
        )}
        <main className="flex-1 h-full overflow-hidden relative z-10">
          <Suspense
            fallback={
              <div className="flex items-center justify-center h-full text-xs font-mono text-content-tertiary">
                LOADING CLOTO...
              </div>
            }
          >
            <Outlet context={{ setImmersive } satisfies AppOutletContext} />
          </Suspense>
        </main>
      </div>

      {/* Settings modal */}
      {settingsOpen && (
        <Modal
          title="Settings"
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
                SYNCHRONIZING...
              </div>
            }
          >
            <SettingsView initialSection={settingsInitialSection} />
          </Suspense>
        </Modal>
      )}

      {/* Help modal */}
      {helpOpen && (
        <Modal title="Help" icon={HelpCircle} size="sm" onClose={() => setHelpOpen(false)}>
          <HelpContent onAskAgent={handleAskAgent} />
        </Modal>
      )}

      <SecurityGuard />
    </div>
  );
}
