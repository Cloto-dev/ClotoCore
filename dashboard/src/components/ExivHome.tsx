import { useEffect, useRef, useState, Suspense, lazy } from 'react';
import { Activity, Database, MessageSquare, Puzzle, Settings, ArrowLeft, Cpu, User as UserIcon } from 'lucide-react';
import { InteractiveGrid } from '../components/InteractiveGrid';
import { GlassWindow } from '../components/GlassWindow';
import { AgentNavigator } from '../components/AgentNavigator';
import { AgentTerminal } from '../components/AgentTerminal';
import { SecurityGuard } from '../components/SecurityGuard';
import { AgentMetadata } from '../types';

const StatusCore = lazy(() => import('../components/StatusCore').then(m => ({ default: (m as any).StatusCore })));
const MemoryCore = lazy(() => import('../components/MemoryCore').then(m => ({ default: (m as any).MemoryCore })));
const AgentCreator = lazy(() => import('../components/AgentCreator').then(m => ({ default: (m as any).AgentCreator })));
const ExivPluginManager = lazy(() => import('../components/ExivPluginManager').then(m => ({ default: (m as any).ExivPluginManager })));

interface WindowInstance {
  id: string;
  type: string;
  title: string;
  x: number;
  y: number;
  zIndex: number;
}

export function ExivHome() {
  const containerRef = useRef<HTMLDivElement>(null);
  const [windows, setWindows] = useState<WindowInstance[]>([]);
  const [nextZ, setNextZ] = useState(100);
  const [activeMainView, setActiveMainView] = useState<string | null>(null);
  const [systemActive, setSystemActive] = useState(false);
  const [agents, setAgents] = useState<AgentMetadata[]>([]);
  const [activeAgentId, setActiveAgentId] = useState<string>('');

  const fetchAgents = () => {
    fetch('/api/agents').then(r => r.json()).then(data => {
      setAgents(data);
      if (data.length > 0 && !activeAgentId) {
        setActiveAgentId(data[0].id);
      }
    }).catch(console.error);
  };

  useEffect(() => {
    fetchAgents();
  }, []);

  const menuItems = [
    { id: 'status', label: 'STATUS', icon: Activity, disabled: false },
    { id: 'memory', label: 'MEMORY', icon: Database, disabled: false },
    { id: 'sandbox', label: 'EXIV', icon: MessageSquare, disabled: false },
    { id: 'plugin', label: 'PLUGIN', icon: Puzzle, disabled: false },
    { id: 'system', label: 'SYSTEM', icon: Settings, disabled: false },
  ];

  const openWindow = (item: any) => {
    const id = `${item.id}-${Date.now()}`;
    setWindows(prev => [...prev, {
      id,
      type: item.id,
      title: item.label,
      x: 100 + (windows.length * 30),
      y: 100 + (windows.length * 30),
      zIndex: nextZ
    }]);
    setNextZ(z => z + 1);
  };

  const closeWindow = (id: string) => {
    setWindows(prev => prev.filter(w => w.id !== id));
  };

  const focusWindow = (id: string) => {
    setWindows(prev => prev.map(w => w.id === id ? { ...w, zIndex: nextZ } : w));
    setNextZ(z => z + 1);
  };

  return (
    <div className="flex h-screen w-screen bg-[#050505] text-white overflow-hidden font-sans select-none relative">
      <InteractiveGrid />
      <SecurityGuard />
      
      {/* 2.x Agent Navigator - Integrated into 1.6 style */}
      <div className="relative z-20 h-full">
        <AgentNavigator 
          agents={agents}
          activeAgentId={activeAgentId}
          onSelectAgent={(id) => {
            setActiveAgentId(id);
            setSystemActive(false);
            setActiveMainView('sandbox');
          }}
          onSelectSystem={() => {
            setSystemActive(true);
            setActiveMainView('system_log');
          }}
          onAddAgent={() => openWindow({ id: 'agent', label: 'AGENT INITIALIZER' })}
          systemActive={systemActive}
        />
      </div>

      <div className="flex-1 relative flex flex-col items-center justify-center p-8 overflow-hidden">
        {/* Logo / Header (1.6.12 Style) */}
        <div className="absolute top-12 text-center z-10">
          <h1 className="text-4xl font-black tracking-[0.2em] text-white drop-shadow-[0_0_15px_rgba(255,255,255,0.3)]">
            Exiv <span className="text-xl font-black tracking-widest text-[#2e4de6] ml-1">v{__APP_VERSION__}</span>
          </h1>
          <p className="text-[10px] text-white/40 mt-3 font-mono uppercase tracking-[0.4em]">
            Exiv / Unified Interface
          </p>
        </div>

        {/* Main Menu (1.6.12 Style) */}
        {!activeMainView && (
          <div className="flex justify-center items-center gap-8 animate-in fade-in zoom-in-95 duration-700">
            {menuItems.map((item) => (
              <div
                key={item.id}
                onClick={() => !item.disabled && (item.id === 'sandbox' ? setActiveMainView('sandbox') : openWindow(item))}
                className={`
                  group relative w-[110px] h-[240px] border-2 bg-white/5 backdrop-blur-md
                  flex flex-col items-center py-8 rounded-2xl
                  transition-all duration-500 ease-out
                  ${item.disabled 
                    ? 'border-white/10 opacity-20 cursor-not-allowed grayscale' 
                    : 'border-[#2e4de6]/30 hover:border-[#2e4de6] hover:bg-white/10 hover:shadow-[0_0_40px_-10px_rgba(46,77,230,0.4)] cursor-pointer active:scale-95'
                  }
                `}
              >
                <div className={`flex-1 flex items-center justify-center transition-all duration-500 ${item.disabled ? 'text-white/20' : 'text-[#2e4de6] group-hover:scale-110'}`}>
                  <item.icon size={40} strokeWidth={1.5} />
                </div>
                <div className={`text-[10px] font-bold tracking-[0.2em] uppercase mb-4 ${item.disabled ? 'text-white/20' : 'text-white/60 group-hover:text-white'}`}>
                  {item.label}
                </div>
              </div>
            ))}
          </div>
        )}

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
                <Suspense fallback={<div className="flex items-center justify-center h-full text-xs font-mono text-white/20">SYNCHRONIZING...</div>}>
                  {win.type === 'status' && <StatusCore isWindowMode={true} />}
                  {win.type === 'memory' && <MemoryCore isWindowMode={true} onClose={() => closeWindow(win.id)} />}
                  {win.type === 'agent' && <AgentCreator onAgentCreated={fetchAgents} />}
                  {win.type === 'plugin' && <ExivPluginManager />}
                  {win.type === 'sandbox' && <AgentTerminal />}
                </Suspense>
              </GlassWindow>
            </div>
          ))}
        </div>

        {/* Fullscreen Overlay Views (Exiv, System Log) */}
        {activeMainView && (
          <div className="fixed inset-0 z-40 bg-black/60 backdrop-blur-3xl animate-in fade-in duration-500">
            <div className="absolute top-0 left-0 right-0 h-20 border-b border-white/5 flex items-center justify-between px-12 bg-black/20">
               <button 
                 onClick={() => { setActiveMainView(null); setSystemActive(false); }}
                 className="flex items-center gap-3 px-6 py-2 rounded-full bg-white/5 border border-white/10 text-[10px] font-bold text-white/60 hover:text-white hover:bg-white/10 transition-all active:scale-95"
               >
                 <ArrowLeft size={16} />
                 <span className="tracking-widest">BACK TO DESKTOP</span>
               </button>
               <div className="flex items-center gap-4">
                  <Cpu size={18} className="text-[#2e4de6]" />
                  <h2 className="text-[12px] font-black tracking-[0.4em] text-white uppercase">{activeMainView}</h2>
               </div>
            </div>

            <div className="absolute inset-0 flex items-center justify-center p-24">
              <div className="w-full max-w-5xl h-full bg-white/5 backdrop-blur-md rounded-[2.5rem] border border-white/10 overflow-hidden flex flex-col shadow-2xl">
                <Suspense fallback={<div className="flex items-center justify-center h-full text-xs font-mono text-white/20">CONNECTING TO KERNEL...</div>}>
                  {activeMainView === 'sandbox' && <AgentTerminal />}
                  {activeMainView === 'system_log' && (
                    <div className="p-12 font-mono text-sm text-blue-300/60 overflow-y-auto space-y-2">
                       <div className="text-blue-400">[KERNEL] Exiv 2.0.0 Online</div>
                       <div>[SYSTEM] Memory engine Karin KS2.5 initializing...</div>
                       <div>[SYSTEM] Visual framework Color connected.</div>
                       <div>[SYSTEM] Action framework Hand initialized.</div>
                       <div className="animate-pulse">_</div>
                    </div>
                  )}
                </Suspense>
              </div>
            </div>
          </div>
        )}
      </div>

      {/* Background Decos */}
      <div className="absolute bottom-12 left-1/2 -translate-x-1/2 z-10">
         <div className="inline-flex items-center gap-3 px-6 py-2 rounded-full bg-white/5 border border-white/10 shadow-2xl text-[10px] text-white/40 font-mono tracking-widest uppercase">
            <span className="w-2 h-2 rounded-full bg-green-500 animate-pulse shadow-[0_0_8px_rgba(34,197,94,0.6)]"></span>
            Kernel Pulse: Stable
         </div>
      </div>
    </div>
  );
}
