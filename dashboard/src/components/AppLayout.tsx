import { useState, useEffect, useRef, Suspense, lazy } from 'react';
import { Outlet, useNavigate, useLocation } from 'react-router-dom';
import { Cpu, Settings, HelpCircle } from 'lucide-react';
import { ViewHeader } from './ViewHeader';
import { InteractiveGrid } from './InteractiveGrid';
import { AppSidebar } from './AppSidebar';
import { Modal } from './Modal';
import { HelpContent } from './HelpContent';
import { SecurityGuard } from './SecurityGuard';
import { useAgentContext } from '../contexts/AgentContext';

const SettingsView = lazy(() => import('./SettingsView').then(m => ({ default: m.SettingsView })));

export interface AppOutletContext {
  setImmersive: (v: boolean) => void;
}

export function AppLayout() {
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [helpOpen, setHelpOpen] = useState(false);
  const [immersive, setImmersive] = useState(false);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(() => localStorage.getItem('sidebar-collapsed') === 'true');
  const navigate = useNavigate();
  const location = useLocation();
  const { agents, setSelectedAgentId } = useAgentContext();

  const activeCount = agents.filter(a => a.enabled).length;

  // Track navigation history for back/forward button states
  const maxIdxRef = useRef(0);
  const [canGoBack, setCanGoBack] = useState(false);
  const [canGoForward, setCanGoForward] = useState(false);

  useEffect(() => {
    const idx = ((window.history.state as Record<string, unknown>)?.idx as number) ?? 0;
    maxIdxRef.current = Math.max(maxIdxRef.current, idx);
    setCanGoBack(idx > 0);
    setCanGoForward(idx < maxIdxRef.current);
  }, [location]);

  const handleToggleSidebar = () => {
    setSidebarCollapsed(prev => {
      const next = !prev;
      localStorage.setItem('sidebar-collapsed', String(next));
      return next;
    });
  };

  const handleAskAgent = () => {
    setHelpOpen(false);
    navigate('/');
    const defaultAgent = agents.find(a => a.id === 'agent.cloto_default');
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
          right={<span className="text-[10px] font-mono text-content-tertiary">{activeCount} / {agents.length} Active</span>}
        />
      )}

      {/* 2. Body — second child, sidebar + content */}
      <div className="flex flex-1 overflow-hidden relative">
        <InteractiveGrid />
        {!immersive && (
          <div className="relative z-10">
            <AppSidebar onSettingsClick={() => setSettingsOpen(true)} collapsed={sidebarCollapsed} onToggleCollapse={handleToggleSidebar} />
          </div>
        )}
        <main className="flex-1 h-full overflow-hidden relative z-10">
          <Suspense fallback={<div className="flex items-center justify-center h-full text-xs font-mono text-content-tertiary">LOADING CLOTO...</div>}>
            <Outlet context={{ setImmersive } satisfies AppOutletContext} />
          </Suspense>
        </main>
      </div>

      {/* Settings modal */}
      {settingsOpen && (
        <Modal title="Settings" icon={Settings} size="lg" onClose={() => setSettingsOpen(false)}>
          <Suspense fallback={<div className="flex items-center justify-center h-full text-xs font-mono text-content-tertiary">SYNCHRONIZING...</div>}>
            <SettingsView />
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
