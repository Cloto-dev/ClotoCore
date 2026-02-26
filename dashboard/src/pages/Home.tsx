import { useRef, useState, useMemo } from 'react';
import { Suspense, lazy } from 'react';
import { useNavigate } from 'react-router-dom';
import { Activity, Database, MessageSquare, Puzzle, Settings, Cpu, Brain, Zap, Shield, Eye, Power, Play, Pause, RefreshCw, LucideIcon } from 'lucide-react';
import { InteractiveGrid } from '../components/InteractiveGrid';
import { SecurityGuard } from '../components/SecurityGuard';
import { SettingsView } from '../components/SettingsView';
import { usePlugins } from '../hooks/usePlugins';
import { api } from '../services/api';
import { useApiKey } from '../contexts/ApiKeyContext';
import { ThemeToggle } from '../components/ThemeToggle';

const ClotoWorkspace = lazy(() => import('../components/AgentWorkspace').then(m => ({ default: m.AgentWorkspace })));

export function Home() {
  const { apiKey } = useApiKey();
  const containerRef = useRef<HTMLDivElement>(null);
  const navigate = useNavigate();

  const [activeMainView, setActiveMainView] = useState<string | null>(null);
  const { plugins } = usePlugins();

  const handleItemClick = async (item: any) => {
    if (item.path.startsWith('api:')) {
      const command = item.path.split(':')[1];
      try {
        await api.post(`/plugin/${item.pluginId}/action/${command}`, {}, apiKey);
        console.log(`Action ${command} executed for ${item.pluginId}`);
      } catch (err) {
        console.error(`Failed to execute action ${command}:`, err);
      }
      return;
    }

    if (item.path === '#') {
      setActiveMainView(item.id);
    } else {
      navigate(item.path);
    }
  };

  const iconMap: Record<string, LucideIcon> = {
    'Activity': Activity,
    'Database': Database,
    'MessageSquare': MessageSquare,
    'Puzzle': Puzzle,
    'Settings': Settings,
    'Cpu': Cpu,
    'Brain': Brain,
    'Zap': Zap,
    'Shield': Shield,
    'Eye': Eye,
    'Power': Power,
    'Play': Play,
    'Pause': Pause,
    'RefreshCw': RefreshCw,
  };

  const menuItems = useMemo(() => {
    const baseItems = [
      { id: 'status', label: 'STATUS', path: '/status', icon: Activity, disabled: false },
      { id: 'memory', label: 'MEMORY', path: '/dashboard', icon: Database, disabled: false },
      { id: 'sandbox', label: 'CLOTO', path: '#', icon: MessageSquare, disabled: false },
      { id: 'mcp', label: 'MCP', path: '/mcp-servers', icon: Puzzle, disabled: false },
    ];

    // Dynamic Plugin Actions (Principle #6: SDK-driven UX)
    const pluginItems = plugins
      .filter(p => p.is_active && p.action_icon && p.action_target)
      .map(p => ({
        id: p.id,
        label: p.name.split('.').pop()?.toUpperCase() || p.name.toUpperCase(),
        path: (p.action_target?.includes(':') || p.action_target?.startsWith('/')) ? p.action_target : '#',
        icon: iconMap[p.action_icon || 'Puzzle'] || Puzzle,
        disabled: false,
        pluginId: p.id
      }));

    return [...baseItems, ...pluginItems, { id: 'settings', label: 'SETTINGS', path: '#', icon: Settings, disabled: false }];
  }, [plugins]);

  return (
    <div
      ref={containerRef}
      className="min-h-screen bg-surface-base flex flex-col items-center justify-center p-8 overflow-hidden relative font-sans text-content-primary select-none"
    >
      <div className="absolute inset-0 bg-[radial-gradient(circle_at_center,_var(--tw-gradient-stops))] from-surface-primary via-surface-secondary to-edge opacity-90 pointer-events-none" />

      <InteractiveGrid />

      {/* Main View Overlay */}
      {activeMainView && (
        <div className="fixed inset-0 z-40 bg-surface-base animate-in fade-in duration-300">
          <div className="absolute top-0 left-0 right-0 h-16 border-b border-[var(--border-strong)] flex items-center justify-between px-8 bg-surface-primary z-50">
            <div className="flex items-center gap-6">
               <button
                 onClick={() => setActiveMainView(null)}
                 className="flex items-center gap-2 px-4 py-2 rounded-full bg-surface-primary border border-edge shadow-sm text-[10px] font-bold text-content-secondary hover:text-brand transition-all hover:shadow-md active:scale-95"
               >
                 <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="m12 19-7-7 7-7"/><path d="M19 12H5"/></svg>
                 <span className="tracking-widest">BACK</span>
               </button>
               <div className="h-4 w-[1px] bg-edge mx-2" />
               <div className="flex items-center gap-3">
                  <div className="w-2 h-2 rounded-full bg-brand animate-pulse" />
                  <h2 className="text-[11px] font-black tracking-[0.4em] text-content-primary uppercase">{activeMainView}</h2>
               </div>
            </div>
            <ThemeToggle />
          </div>

          <div className="absolute inset-0 top-16 flex flex-col">
            <div className="flex-1 overflow-hidden animate-in fade-in duration-300">
              <Suspense fallback={<div className="flex items-center justify-center h-full text-xs font-mono text-content-tertiary">SYNCHRONIZING...</div>}>
                {activeMainView === 'sandbox' && <ClotoWorkspace />}
                {activeMainView === 'settings' && <SettingsView />}
              </Suspense>
            </div>
          </div>
        </div>
      )}

      {/* Security Layer */}
      <SecurityGuard />

      {/* Main Menu */}
      <div className="relative z-20 w-full max-w-5xl flex flex-col items-center">
        <div className="mb-16 text-center">
          <h1 className="text-4xl font-black tracking-[0.2em] text-content-primary">
            CLOTO SYSTEM <span className="text-xl font-black tracking-widest text-brand ml-1">v{__APP_VERSION__}</span>
          </h1>
          <p className="text-[10px] text-content-tertiary mt-3 font-mono uppercase tracking-[0.4em]">
            Neural Interface / Central Archive
          </p>
        </div>

        <div className="flex justify-center items-center gap-6">
          {menuItems.map((item) => (
            <div
              key={item.id}
              onClick={() => handleItemClick(item)}
              className={`
                group relative w-[96px] h-[224px] border-2 bg-glass-strong backdrop-blur-sm
                flex flex-col items-center py-6 shadow-sm rounded-md
                transition-all duration-300 ease-out
                ${item.disabled
                  ? 'border-content-muted opacity-40 cursor-not-allowed grayscale bg-surface-secondary'
                  : 'border-brand hover:bg-surface-primary hover:shadow-[0_10px_30px_-10px_rgba(46,77,230,0.5)] cursor-pointer active:scale-95'
                }
              `}
            >
              <div className={`flex-1 flex items-center justify-center transition-all ${item.disabled ? 'text-content-muted' : 'text-brand'}`}>
                <item.icon size={32} strokeWidth={2} />
              </div>
              <div className={`text-[10px] font-bold tracking-[0.1em] uppercase mb-2 ${item.disabled ? 'text-content-tertiary' : 'text-brand'}`}>
                {item.label}
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
