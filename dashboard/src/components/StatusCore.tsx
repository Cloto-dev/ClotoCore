import { useEffect, useState, useRef, memo } from 'react';
import { Link } from 'react-router-dom';
import { ArrowLeft, Activity } from 'lucide-react';
import { NeuralNetwork } from '../components/NeuralNetwork';
import { useMetrics } from '../hooks/useMetrics';
import { useStatusManager, ThoughtLine } from '../hooks/useStatusManager';
import { useTheme } from '../hooks/useTheme';
import type { StrictSystemEvent } from '../types';

// --- Sub-component: DecipherText ---
function DecipherText({ targetText }: { targetText: string }) {
  const [displayText, setDisplayText] = useState('');
  const chars = "!@#$%^&*()_+-=[]{}|;:,.<>?/0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
  const timerRef = useRef<number | null>(null);

  useEffect(() => {
    let iteration = 0;
    const update = () => {
      const deciphered = targetText
        .split("")
        .map((char, index) => {
          // スペースはそのまま表示し、それ以外の文字をランダム化
          if (index < iteration || char === " ") return char;
          return chars[Math.floor(Math.random() * chars.length)];
        })
        .join("");

      setDisplayText(deciphered);

      if (iteration < targetText.length) {
        iteration += 1; // 解読速度を少しアップ
        timerRef.current = window.requestAnimationFrame(update);
      }
    };

    timerRef.current = window.requestAnimationFrame(update);
    return () => timerRef.current && cancelAnimationFrame(timerRef.current);
  }, [targetText]);

  return <span>{displayText}</span>;
}

// --- Sub-component: ThoughtLineDisplay ---
function ThoughtLineDisplay({ line }: { line: ThoughtLine }) {
  return (
    <div
      className="text-[8vw] font-black leading-none whitespace-nowrap uppercase tracking-tighter text-center max-w-[90vw] overflow-hidden"
      style={{
        animation: 'thought-fade 30s linear forwards',
      }}
    >
      <DecipherText targetText={line.text} />
    </div>
  );
}

// --- Sub-component: TimelinePins ---
function TimelinePins({ events, startTime, endTime, onPinClick }: { 
  events: StrictSystemEvent[], 
  startTime: number, 
  endTime: number,
  onPinClick: (t: number) => void 
}) {
  const { colors } = useTheme();
  const pinEvents = events.filter(e => e.type === "MessageReceived" || e.type === "ToolStart");
  const duration = endTime - startTime;

  if (duration <= 0) return null;

  return (
    <div className="absolute inset-x-0 -top-2 h-4 pointer-events-none">
      {pinEvents.map((e, i) => {
        const pos = ((e.timestamp - startTime) / duration) * 100;
        const color = e.type === "MessageReceived" ? colors.brandHex : (e.type === "ToolStart" ? (e.payload?.color || "#2ea8e6") : "#2ea8e6");
        return (
          <div 
            key={i}
            className="absolute bottom-0 w-[1px] h-3 opacity-60 hover:opacity-100 hover:h-4 hover:w-[2px] transition-all cursor-pointer pointer-events-auto"
            style={{ left: `${pos}%`, backgroundColor: color }}
            title={`${e.type === "MessageReceived" ? "Message" : (e.type === "ToolStart" ? (e.payload?.label || "Tool") : "")} at ${new Date(e.timestamp).toLocaleTimeString()}`}
            onClick={(ev) => {
              ev.stopPropagation();
              onPinClick(e.timestamp);
            }}
          />
        );
      })}
    </div>
  );
}

export const StatusCore = memo(function StatusCore({ isWindowMode = false }: { isWindowMode?: boolean }) {
  const [seekTime, setSeekTime] = useState<number | null>(null);
  const [now, setNow] = useState(Date.now());
  const [immersive, setImmersive] = useState(false);
  const { metrics, fetchMetrics } = useMetrics();
  const { eventHistory, thoughtLines } = useStatusManager(fetchMetrics);
  const { colors } = useTheme();
  
  const realMouse = useRef({ x: -1000, y: -1000 });

  useEffect(() => {
    const handleMouseMove = (e: MouseEvent) => {
      realMouse.current = { x: e.clientX, y: e.clientY };
    };
    window.addEventListener('mousemove', handleMouseMove);
    return () => window.removeEventListener('mousemove', handleMouseMove);
  }, []);

  // Update 'now' periodically to prevent slider flickering (jitter)
  useEffect(() => {
    const interval = setInterval(() => {
      setNow(Date.now());
    }, 200); 
    return () => clearInterval(interval);
  }, []);

  // Auto-replay logic when in archive mode
  useEffect(() => {
    if (seekTime === null) return;
    
    const interval = setInterval(() => {
      setSeekTime(prev => {
        if (prev === null) return null;
        const nextTime = prev + 1000;
        // If we catch up to live time, switch back to LIVE mode
        if (nextTime >= Date.now() - 500) {
          return null;
        }
        return nextTime;
      });
    }, 1000); // 1x playback speed
    
    return () => clearInterval(interval);
  }, [seekTime]);

  const handleSeekChange = (e: any) => {
    const val = parseInt(e.target.value) || 0;
    
    // Snap logic: Find closest event within +/- 5 seconds window
    const closestEvent = eventHistory.find(ev => Math.abs(ev.timestamp - val) < 5000);
    
    if (closestEvent) {
      // Snap to 2 seconds BEFORE the event starts
      setSeekTime(closestEvent.timestamp - 2000);
    } else {
      setSeekTime(val);
    }
  };

  const startTime = eventHistory.length > 0 ? eventHistory[0].timestamp : now;
  const endTime = now;
  const currentEffectiveTime = seekTime || endTime;

  return (
    <div 
      onMouseEnter={(e) => {
        realMouse.current = { x: e.clientX, y: e.clientY };
      }}
      className={`${isWindowMode ? 'bg-transparent h-full w-full rounded-md' : 'bg-surface-base min-h-screen'} flex flex-col items-center justify-center overflow-hidden relative font-sans text-content-primary`}
    >
      {/* Header — MemoryCore design language (document flow, not absolute) */}
      {!isWindowMode && !immersive && (
        <header className="absolute top-0 left-0 right-0 z-20 p-6 md:p-12">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-6">
              <Link to="/" className="p-3 rounded-full bg-glass-subtle backdrop-blur-sm border border-edge hover:border-brand hover:text-brand transition-all shadow-sm group">
                <ArrowLeft size={20} className="group-hover:-translate-x-1 transition-transform" />
              </Link>
              <div className="flex items-center gap-4">
                <div className="w-12 h-12 bg-glass-subtle backdrop-blur-sm rounded-md flex items-center justify-center shadow-sm border border-edge">
                  <Activity className="text-brand" size={24} strokeWidth={2} />
                </div>
                <div
                  onClick={() => setImmersive(true)}
                  className="cursor-pointer select-none group/title"
                >
                  <h1 className="text-3xl font-black tracking-tighter text-content-primary uppercase group-hover/title:text-brand transition-colors">Status Core</h1>
                  <p className="text-[10px] text-content-tertiary font-mono uppercase tracking-[0.2em] flex items-center gap-2">
                    <span className="inline-block w-1.5 h-1.5 bg-brand rounded-full animate-pulse"></span>
                    Real-time Telemetry Active
                  </p>
                </div>
              </div>
            </div>
          </div>
        </header>
      )}

      {/* Immersive mode: invisible hit area spanning the full header region */}
      {!isWindowMode && immersive && (
        <button
          onClick={() => setImmersive(false)}
          className="absolute top-0 left-0 right-0 h-24 z-20 cursor-pointer"
          aria-label="Show UI"
        />
      )}

      {/* Archive Indicator */}
      {seekTime !== null && !immersive && (
        <div className="absolute top-20 left-1/2 -translate-x-1/2 z-30 px-4 py-1 bg-blue-600 text-white text-[10px] font-black uppercase tracking-[0.3em] rounded-full animate-pulse shadow-lg shadow-blue-500/20">
          REPLAYING: {new Date(seekTime).toLocaleTimeString()}
        </div>
      )}

      {/* 1. Background Scrolling Decipher Stream */}
      <div 
        className="absolute inset-0 z-0 opacity-[0.04] select-none flex flex-col justify-center items-center pointer-events-none"
        style={{
          color: colors.brandHex,
          maskImage: 'radial-gradient(circle 40vw at center, black 30%, transparent 100%)',
          WebkitMaskImage: 'radial-gradient(circle 40vw at center, black 30%, transparent 100%)'
        }}
      >
        {thoughtLines.length === 0 ? (
           <div className="text-[2vw] font-mono opacity-20">AWAITING NEURAL SIGNALS...</div>
        ) : (
          thoughtLines.map((line: any) => (
            <ThoughtLineDisplay key={line.id} line={line} />
          ))
        )}
      </div>

      {/* 2. Static Grid Background (bottom-fade, inherited from MemoryCore) */}
      <div
        className="absolute inset-0 z-0 opacity-30 pointer-events-none"
        style={{
          backgroundImage: `linear-gradient(to right, var(--canvas-grid) 1px, transparent 1px), linear-gradient(to bottom, var(--canvas-grid) 1px, transparent 1px)`,
          backgroundSize: '40px 40px',
          maskImage: 'linear-gradient(to bottom, black 40%, transparent 100%)',
          WebkitMaskImage: 'linear-gradient(to bottom, black 40%, transparent 100%)'
        }}
      />

      {/* 3. The Core Visualizer */}
      <NeuralNetwork
        mouseRef={realMouse}
        events={eventHistory}
        seekTime={seekTime}
      />

      {/* Time Seek Bar */}
      <div className={`absolute bottom-8 left-1/2 -translate-x-1/2 z-40 flex items-center gap-4 bg-glass-strong backdrop-blur-md border border-glass p-2 px-4 rounded-full shadow-lg transition-all ${isWindowMode ? 'scale-75 origin-bottom' : ''} ${immersive && !isWindowMode ? 'opacity-0 pointer-events-none' : ''}`}>
        <button 
          onClick={() => setSeekTime(null)}
          className={`px-3 py-1 rounded-full text-[9px] font-black transition-all ${seekTime === null ? 'bg-brand text-white' : 'bg-surface-secondary text-content-tertiary hover:bg-surface-secondary'}`}
        >
          LIVE
        </button>
        <div className="relative flex items-center">
          <TimelinePins 
            events={eventHistory} 
            startTime={startTime} 
            endTime={endTime} 
            onPinClick={setSeekTime} 
          />
          <input
            type="range"
            min={startTime}
            max={endTime}
            value={currentEffectiveTime}
            onChange={handleSeekChange}
            className="w-48 h-1 bg-edge rounded-lg appearance-none cursor-pointer accent-brand relative z-10"
          />
        </div>
        <div className="text-[9px] font-mono text-content-tertiary w-12 text-center">
          {seekTime === null ? 'NOW' : '- ' + Math.floor((endTime - seekTime) / 1000) + 's'}
        </div>
      </div>

      {!isWindowMode && !immersive && (
        <div className="absolute bottom-8 left-8 flex flex-col gap-1 z-20">
          <div className="flex items-center gap-2 text-[10px] font-mono text-content-tertiary">
            <span className="w-2 h-2 rounded-full bg-brand animate-pulse"></span> SYSTEM SYNCHRONIZED
          </div>
        </div>
      )}

      {/* Metrics Overlay */}
      {metrics && !immersive && (
        <div className={`absolute z-20 flex flex-col gap-2 pointer-events-none ${isWindowMode ? 'bottom-2 right-2 items-end' : 'bottom-8 right-8 text-right'}`}>
           <div className={`${isWindowMode ? 'bg-glass-subtle p-2' : 'bg-glass-subtle backdrop-blur-sm p-3'} border border-edge rounded-lg shadow-sm transition-all`}>
              <div className="text-[10px] font-mono text-content-tertiary uppercase tracking-widest mb-1">System Load</div>
              <div className={`${isWindowMode ? 'text-xs' : 'text-sm'} font-bold font-mono`}>{metrics.ram_usage} / {metrics.total_requests} REQS</div>
           </div>
           <div className={`${isWindowMode ? 'bg-glass-subtle p-2' : 'bg-glass-subtle backdrop-blur-sm p-3'} border border-edge rounded-lg shadow-sm transition-all`}>
              <div className="text-[10px] font-mono text-content-tertiary uppercase tracking-widest mb-1">Memory Banks (KS2)</div>
              <div className={`${isWindowMode ? 'text-xs' : 'text-sm'} font-bold font-mono`}>{metrics.total_memories} PROFILES / {metrics.total_episodes} EPISODES</div>
           </div>
        </div>
      )}
    </div>
  );
});
