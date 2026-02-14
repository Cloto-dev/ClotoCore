import { useEffect, useRef, useState, useMemo } from 'react';
import { Suspense, lazy } from 'react';
import { useNavigate } from 'react-router-dom';
import { Activity, Database, MessageSquare, Puzzle, Settings, Cpu, Brain, Zap, Shield, Eye, Power, Play, Pause, RefreshCw, LucideIcon } from 'lucide-react';
import { InteractiveGrid } from '../components/InteractiveGrid';
import { GlassWindow } from '../components/GlassWindow';
import { CustomCursor } from '../components/CustomCursor';
import { SecurityGuard } from '../components/SecurityGuard';
import { PluginManifest } from '../types';
import { useWindowManager } from '../hooks/useWindowManager';
import { useDraggable } from '../hooks/useDraggable';
import { useEventStream } from '../hooks/useEventStream';
import { GazeTracker } from '../components/GazeTracker';

import { api, API_BASE } from '../services/api';
import { SystemUpdate } from '../components/SystemUpdate';

const StatusCore = lazy(() => import('../components/StatusCore').then(m => ({ default: m.StatusCore })));
const MemoryCore = lazy(() => import('../components/MemoryCore').then(m => ({ default: m.MemoryCore })));
const ExivWorkspace = lazy(() => import('../components/AgentWorkspace').then(m => ({ default: m.AgentWorkspace })));
const ExivPluginManager = lazy(() => import('../components/ExivPluginManager').then(m => ({ default: m.ExivPluginManager })));

function SystemView() {
  const [logs, setLogs] = useState<string[]>([]);
  const scrollRef = useRef<HTMLDivElement>(null);

  useEventStream(`${API_BASE}/events`, (event) => {
    const timestamp = new Date().toLocaleTimeString();
    const logLine = `[${timestamp}] ${event.type}: ${JSON.stringify(event.data).slice(0, 100)}...`;
    setLogs(prev => [...prev, logLine].slice(-50));
  });

  useEffect(() => {
    if (scrollRef.current) scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
  }, [logs]);

  return (
    <div className="flex-1 flex flex-col bg-slate-900/90 text-blue-400 overflow-hidden">
      {/* Update Section */}
      <div className="border-b border-blue-900/50">
        <div className="flex items-center gap-2 px-4 pt-4 pb-2">
          <RefreshCw size={14} />
          <span className="text-[10px] font-black tracking-widest font-mono">SYSTEM_UPDATE</span>
        </div>
        <SystemUpdate />
      </div>

      {/* Live Log Section */}
      <div className="flex-1 flex flex-col p-6 font-mono text-[10px] overflow-hidden">
        <div className="flex items-center gap-2 mb-4 border-b border-blue-900/50 pb-2">
          <Cpu size={14} />
          <span className="font-black tracking-widest">KERNEL_LIVE_LOG</span>
        </div>
        <div ref={scrollRef} className="flex-1 overflow-y-auto space-y-1 no-scrollbar">
          {logs.length === 0 && <div className="opacity-30">AWAITING_SIGNAL...</div>}
          {logs.map((log, i) => (
            <div key={i} className="animate-in fade-in slide-in-from-left-1 duration-300">
              <span className="opacity-50 mr-2">&gt;</span>{log}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

export function Home() {
  const containerRef = useRef<HTMLDivElement>(null);
  const realMouse = useRef({ x: -1000, y: -1000 });
  const navigate = useNavigate();

  const [activeMainView, setActiveMainView] = useState<string | null>(null);
  const [isCursorActive, setIsCursorActive] = useState(true);
  const [isGazeActive, setIsGazeActive] = useState(false);
  const [plugins, setPlugins] = useState<PluginManifest[]>([]);

  // Phase 1 Refactor: Use Custom Hooks
  const { windows, openWindow, closeWindow, focusWindow } = useWindowManager();
  
  const handleItemClick = async (item: any) => {
    if (item.path.startsWith('api:')) {
      const command = item.path.split(':')[1];
      try {
        await api.post(`/plugin/${item.pluginId}/action/${command}`, {});
        console.log(`Action ${command} executed for ${item.pluginId}`);
        // 🆕 Handle gaze tracking toggle (flexible ID check)
        if ((item.pluginId === 'python.gaze' || item.pluginId === 'vision.gaze_webcam') && command === 'toggle') {
          console.log("👁️ Toggling GazeTracker component...");
          setIsGazeActive(prev => !prev);
        }
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

  const { ghostPos, handleMouseDown, dragItem } = useDraggable(openWindow, handleItemClick);

  useEffect(() => {
    api.getPlugins()
      .then((data: PluginManifest[]) => {
        setPlugins(data);
        const cursorPlugin = data.find(p => p.name.includes("Neural Cursor"));
        if (cursorPlugin) {
          setIsCursorActive(cursorPlugin.is_active);
        }
      })
      .catch(err => console.error("Failed to sync cursor plugin:", err));
  }, []);

  useEffect(() => {
    if (isCursorActive) {
      document.body.classList.add('neural-cursor-active');
    } else {
      document.body.classList.remove('neural-cursor-active');
    }
    return () => {
      document.body.classList.remove('neural-cursor-active');
    };
  }, [isCursorActive]);

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
      { id: 'sandbox', label: 'EXIV', path: '#', icon: MessageSquare, disabled: false },
      { id: 'plugin', label: 'PLUGIN', path: '#', icon: Puzzle, disabled: false },
    ];

    // 🚀 Dynamic Plugin Actions (Principle #6: SDK-driven UX)
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

    return [...baseItems, ...pluginItems, { id: 'system', label: 'SYSTEM', path: '#', icon: Settings, disabled: false }];
  }, [plugins]);

  return (
    <div 
      ref={containerRef}
      onMouseEnter={(e) => {
        realMouse.current = { x: e.clientX, y: e.clientY };
      }}
      className="min-h-screen bg-slate-50 flex flex-col items-center justify-center p-8 overflow-hidden relative font-sans text-slate-800 select-none"
    >
      <div className="absolute inset-0 bg-[radial-gradient(circle_at_center,_var(--tw-gradient-stops))] from-white via-slate-100 to-slate-200 opacity-90 pointer-events-none" />
      
      <InteractiveGrid />

      {/* Main View Overlay */}
      {activeMainView && (
        <div className="fixed inset-0 z-40 bg-slate-50/80 backdrop-blur-xl animate-in fade-in duration-300">
          <div className="absolute top-0 left-0 right-0 h-16 border-b border-slate-200 flex items-center justify-between px-8 bg-white/50 z-50">
            <div className="flex items-center gap-6">
               <button 
                 onClick={() => setActiveMainView(null)}
                 className="flex items-center gap-2 px-4 py-2 rounded-full bg-white border border-slate-200 shadow-sm text-[10px] font-bold text-slate-600 hover:text-[#2e4de6] transition-all hover:shadow-md active:scale-95"
               >
                 <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="m12 19-7-7 7-7"/><path d="M19 12H5"/></svg>
                 <span className="tracking-widest">BACK</span>
               </button>
               <div className="h-4 w-[1px] bg-slate-200 mx-2" />
               <div className="flex items-center gap-3">
                  <div className="w-2 h-2 rounded-full bg-[#2e4de6] animate-pulse" />
                  <h2 className="text-[11px] font-black tracking-[0.4em] text-slate-800 uppercase">{activeMainView}</h2>
               </div>
            </div>
          </div>

          <div className="absolute inset-0 flex items-center justify-center p-6 md:p-12">
            <div className="w-full max-w-4xl h-full max-h-[700px] bg-white/40 backdrop-blur-md rounded-2xl border-2 border-[#2e4de6]/50 overflow-hidden flex flex-col scale-in-center animate-in fade-in zoom-in-95 duration-500">
              <Suspense fallback={<div className="flex items-center justify-center h-full text-xs font-mono text-slate-400">SYNCHRONIZING...</div>}>
                {activeMainView === 'sandbox' && <ExivWorkspace />}
                {activeMainView === 'plugin' && <ExivPluginManager />}
                {activeMainView === 'system' && <SystemView />}
              </Suspense>
            </div>
          </div>
        </div>
      )}

      {/* Security Layer */}
      <SecurityGuard />

      {/* Windows Layer */}
      <div className="absolute inset-0 z-30 pointer-events-none">
        {windows.map(win => (
          <div key={win.id} className="pointer-events-auto">
            <GlassWindow
              id={win.id}
              title={win.title}
              initialPosition={{ x: win.x, y: win.y }}
              zIndex={win.zIndex}
              onClose={() => closeWindow(win.id)}
              onFocus={() => focusWindow(win.id)}
            >
              <Suspense fallback={<div className="flex items-center justify-center h-full text-xs font-mono text-slate-400">LOADING MODULE...</div>}>
                {win.type === 'status' && <StatusCore isWindowMode={true} />}
                {win.type === 'memory' && <MemoryCore isWindowMode={true} onClose={() => closeWindow(win.id)} />}
                {win.type === 'sandbox' && <ExivWorkspace />}
                {win.type === 'plugin' && <ExivPluginManager />}
                {win.type === 'system' && <SystemView />}
              </Suspense>
            </GlassWindow>
          </div>
        ))}
      </div>

      {/* Ghost Dragging Icon */}
      {ghostPos && dragItem && (
        <div 
          className="fixed z-50 pointer-events-none p-4 bg-white/50 backdrop-blur-md rounded-lg border border-[#2e4de6] text-[#2e4de6] shadow-xl animate-pulse"
          style={{ left: ghostPos.x, top: ghostPos.y, transform: 'translate(-50%, -50%)' }}
        >
          <dragItem.icon size={32} />
        </div>
      )}

      {/* Main Menu */}
      <div className="relative z-20 w-full max-w-5xl flex flex-col items-center">
        <div className="mb-16 text-center">
          <h1 className="text-4xl font-black tracking-[0.2em] text-slate-800">
            EXIV SYSTEM <span className="text-xl font-black tracking-widest text-[#2e4de6] ml-1">v{__APP_VERSION__}</span>
          </h1>
          <p className="text-[10px] text-slate-400 mt-3 font-mono uppercase tracking-[0.4em]">
            Neural Interface / Central Archive
          </p>
        </div>

        <div className="flex justify-center items-center gap-6">
          {menuItems.map((item) => (
            <div
              key={item.id}
              onMouseDown={(e) => handleMouseDown(e, item)}
              className={`
                group relative w-[96px] h-[224px] border-2 bg-white/60 backdrop-blur-sm
                flex flex-col items-center py-6 shadow-sm rounded-md
                transition-all duration-300 ease-out
                ${item.disabled 
                  ? 'border-slate-300 opacity-40 cursor-not-allowed grayscale bg-slate-100' 
                  : 'border-[#2e4de6] hover:bg-white hover:shadow-[0_10px_30px_-10px_rgba(46,77,230,0.5)] cursor-pointer active:scale-95'
                }
              `}
            >
              <div className={`flex-1 flex items-center justify-center transition-all ${item.disabled ? 'text-slate-300' : 'text-[#2e4de6]'}`}>
                <item.icon size={32} strokeWidth={2} />
              </div>
              <div className={`text-[10px] font-bold tracking-[0.1em] uppercase mb-2 ${item.disabled ? 'text-slate-400' : 'text-[#2e4de6]'}`}>
                {item.label}
              </div>
            </div>
          ))}
        </div>
      </div>
      {isCursorActive && <CustomCursor />}
      {isGazeActive && <GazeTracker />}
    </div>
  );
}