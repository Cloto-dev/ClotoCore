import { useState, useEffect, useRef } from 'react';
import { Send, Image as ImageIcon, ShieldCheck, X } from 'lucide-react';

interface Message {
  id: string;
  role: 'user' | 'karin';
  content: string;
  timestamp: Date;
}

export function SandboxCore() {
  const [messages, setMessages] = useState<Message[]>([
    { id: '1', role: 'karin', content: 'SYSTEM READY. ISOLATED ENVIRONMENT INITIALIZED. READY FOR TESTING.', timestamp: new Date() }
  ]);
  const [input, setInput] = useState('');
  const [avatar, setAvatar] = useState<string | null>(localStorage.getItem('karin_playground_avatar'));
  const [isTyping, setIsTyping] = useState(false);
  const [currentThoughts, setCurrentThoughts] = useState<string[]>([]);
  const [lastResetTime, setLastResetTime] = useState<number>(0);
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    // 1. Fetch History on Mount
    const fetchHistory = async () => {
      console.log("🔍 [SANDBOX] Fetching history...");
      try {
        const res = await fetch('/api/sandbox/history');
        console.log("🔍 [SANDBOX] History response status:", res.status);
        if (!res.ok) return;
        const events = await res.json();
        
        // Reconstruct messages and detect ongoing state
        const historicalMessages: Message[] = [];
        let lastUserMessageIndex = -1;
        let lastAssistantMessageIndex = -1;

        events.forEach((ev: any, idx: number) => {
          // Parse both legacy and new RawMessage format
          const isUser = ev.type === 'MessageReceived' || (ev.type === 'RawMessage' && ev.payload.message.role === 'user');
          const isAssistant = ev.type === 'ResponseGenerated' || (ev.type === 'RawMessage' && ev.payload.message.role === 'assistant');

          if (isUser) {
            historicalMessages.push({
              id: ev.timestamp || idx.toString(),
              role: 'user',
              content: ev.payload.content || ev.payload.message.content,
              timestamp: new Date(ev.timestamp || Date.now())
            });
            lastUserMessageIndex = idx;
          } else if (isAssistant) {
            historicalMessages.push({
              id: ev.timestamp || idx.toString(),
              role: 'karin',
              content: ev.payload.content || ev.payload.message.content,
              timestamp: new Date(ev.timestamp || Date.now())
            });
            lastAssistantMessageIndex = idx;
          }
        });

        // Ongoing process detection
        const isStillProcessing = lastUserMessageIndex > lastAssistantMessageIndex;

        if (isStillProcessing) {
          setIsTyping(true);
          const recentThoughts = events
            .slice(lastUserMessageIndex + 1)
            .filter((ev: any) => ev.type === 'Thought')
            .map((ev: any) => ev.payload.content);
          
          if (recentThoughts.length > 0) {
            setCurrentThoughts(recentThoughts.slice(-2));
          }
        }
        
        // ALWAYS update messages, even if empty, to allow clearing the UI
        setMessages(historicalMessages.length > 0 ? historicalMessages : [
          { id: '1', role: 'karin', content: 'SYSTEM READY. ISOLATED ENVIRONMENT INITIALIZED.', timestamp: new Date() }
        ]);

      } catch (err) {
        console.error("Failed to load sandbox history", err);
      }
    };

    fetchHistory();

    // 2. Connect to Sandbox Event Stream
    const eventSource = new EventSource('/api/sandbox/events');
    
    eventSource.onmessage = (e) => {
      try {
        const event = JSON.parse(e.data);
        
        // Skip events older than the last local reset
        if (event.timestamp && event.timestamp < lastResetTime) {
          return;
        }

        if (event.type === 'Thought') {
          setCurrentThoughts(prev => [...prev.slice(-2), event.payload.content]);
        } else if (event.type === 'ResponseGenerated') {
          setIsTyping(false);
          setCurrentThoughts([]);
          setMessages(prev => {
            if (prev.some(m => m.content === event.payload.content)) return prev;
            return [...prev, {
              id: event.timestamp || Date.now().toString(),
              role: 'karin',
              content: event.payload.content,
              timestamp: new Date(event.timestamp || Date.now())
            }];
          });
        }
      } catch (err) {
        console.error('Failed to parse sandbox event', err);
      }
    };

    return () => eventSource.close();
  }, [lastResetTime]); // Depend on lastResetTime to recreate listener if needed

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [messages, currentThoughts]);

  const handleDrop = (e: DragEvent) => {
    e.preventDefault();
    const file = e.dataTransfer?.files[0];
    if (file && file.type.startsWith('image/')) {
      const reader = new FileReader();
      reader.onload = (event) => {
        const result = event.target?.result as string;
        setAvatar(result);
        localStorage.setItem('karin_playground_avatar', result);
      };
      reader.readAsDataURL(file);
    }
  };

  const handleDragOver = (e: DragEvent) => {
    e.preventDefault();
  };

  const clearAvatar = (e: MouseEvent) => {
    e.stopPropagation();
    setAvatar(null);
    localStorage.removeItem('karin_playground_avatar');
  };

  const resetSandbox = async () => {
    if (!confirm('RESET ENTIRE SANDBOX ENVIRONMENT? ALL TEMPORARY MEMORIES WILL BE LOST.')) return;
    
    try {
      // 1. Execute reset on server first
      const res = await fetch('/api/sandbox/reset', { method: 'POST' });
      if (!res.ok) throw new Error('Reset failed');

      // 2. ONLY AFTER server confirms, update local state
      const now = Date.now();
      setLastResetTime(now);
      
      // 3. Clear UI immediately
      setMessages([{ id: '1', role: 'karin', content: 'ENVIRONMENT PURGED. SYSTEM RESET TO INITIAL STATE.', timestamp: new Date() }]);
      setCurrentThoughts([]);
      console.log("✅ [SANDBOX] Environment reset confirmed at", now);
    } catch (err) {
      console.error("Failed to reset sandbox", err);
      alert("Failed to reset sandbox environment.");
    }
  };

  const sendMessage = async () => {
    if (!input.trim() || isTyping) return;
    
    const userMsg: Message = {
      id: Date.now().toString(),
      role: 'user',
      content: input,
      timestamp: new Date()
    };

    setMessages(prev => [...prev, userMsg]);
    setInput('');
    setIsTyping(true);
    setCurrentThoughts([]);

    try {
      const res = await fetch('/api/sandbox/chat', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          message: input,
          user_name: 'Experimental User',
          user_id: 999
        })
      });

      if (!res.ok) throw new Error('Sandbox API error');
      const data = await res.json();

      const karinMsg: Message = {
        id: (Date.now() + 1).toString(),
        role: 'karin',
        content: data.response,
        timestamp: new Date()
      };
      setMessages(prev => [...prev, karinMsg]);
    } catch (err) {
      console.error(err);
      setMessages(prev => [...prev, {
        id: 'err',
        role: 'karin',
        content: '⚠️ ERROR: FAILED TO PROCESS COMMAND IN ISOLATED ENVIRONMENT.',
        timestamp: new Date()
      }]);
    } finally {
      setIsTyping(false);
      setCurrentThoughts([]);
    }
  };

  return (
    <div className="flex h-full w-full bg-transparent overflow-hidden font-sans">
      {/* Left Pane: Avatar Area */}
      <div 
        className="w-1/3 border-r border-white/20 relative flex flex-col items-center justify-center p-4 group bg-white/10"
        onDrop={handleDrop}
        onDragOver={handleDragOver}
      >
        {avatar ? (
          <div className="w-full h-full relative overflow-hidden rounded-lg shadow-2xl group/avatar">
            <img src={avatar} alt="Karin Avatar" className="w-full h-full object-cover" />
            <div className="absolute inset-0 bg-gradient-to-t from-[#2e4de6]/40 to-transparent pointer-events-none" />
            <button 
              onClick={clearAvatar}
              className="absolute top-2 right-2 p-1.5 bg-black/40 hover:bg-red-500 text-white rounded-full opacity-0 group-hover/avatar:opacity-100 transition-all z-10"
              title="Reset Avatar"
            >
              <X size={14} />
            </button>
          </div>
        ) : (
          <div className="flex flex-col items-center justify-center text-[#2e4de6]/40 gap-4 border-2 border-dashed border-[#2e4de6]/20 rounded-xl w-full h-full group-hover:bg-[#2e4de6]/5 transition-colors">
            <ImageIcon size={64} strokeWidth={1} />
            <span className="text-[10px] font-bold tracking-[0.2em] uppercase">Drop Avatar Image</span>
          </div>
        )}
        
        {/* Environment Tag */}
        <div className="absolute top-4 left-4 flex flex-col gap-2">
          <div className="flex items-center gap-2 px-3 py-1 bg-[#2e4de6] text-white rounded-full shadow-lg">
            <ShieldCheck size={12} />
            <span className="text-[9px] font-bold tracking-widest uppercase">Isolated Sandbox</span>
          </div>
          <button 
            onClick={resetSandbox}
            className="px-3 py-1 bg-red-500/20 hover:bg-red-500 text-red-500 hover:text-white border border-red-500/50 rounded-full text-[8px] font-bold tracking-widest transition-all"
          >
            PURGE MEMORY
          </button>
        </div>

        <div className="mt-4 text-center">
          <h2 className="text-sm font-black tracking-widest text-[#2e4de6]">KARIN</h2>
          <p className="text-[8px] text-slate-400 font-mono uppercase tracking-[0.2em]">Sandbox Mode v2.0</p>
        </div>
      </div>

      {/* Right Pane: Chat Area */}
      <div className="flex-1 flex flex-col bg-white/20">
        <div 
          ref={scrollRef}
          className="flex-1 overflow-y-auto p-6 space-y-6 scrollbar-thin scrollbar-thumb-[#2e4de6]/20"
        >
          {messages.map((msg) => (
            <div key={msg.id} className={`flex items-start gap-3 ${msg.role === 'user' ? 'flex-row-reverse' : ''}`}>
              <div className={`w-8 h-8 rounded flex items-center justify-center shrink-0 shadow-sm ${
                msg.role === 'karin' ? 'bg-[#2e4de6] text-white' : 'bg-white text-slate-600'
              }`}>
                {msg.role === 'karin' ? <span className="text-[10px] font-bold">K</span> : <span className="text-[10px] font-bold">U</span>}
              </div>
              <div className={`max-w-[80%] p-4 rounded-xl text-xs leading-relaxed shadow-sm ${
                msg.role === 'karin' 
                  ? 'bg-[#2e4de6] text-white rounded-tl-none border-l-4 border-blue-300' 
                  : 'bg-white text-slate-700 rounded-tr-none'
              }`}>
                {msg.content}
                <div className={`mt-2 text-[8px] opacity-50 font-mono ${msg.role === 'karin' ? 'text-blue-100' : 'text-slate-400'}`}>
                  {msg.timestamp.toLocaleTimeString()}
                </div>
              </div>
            </div>
          ))}

          {/* Real-time Thoughts */}
          {currentThoughts.length > 0 && (
            <div className="flex items-start gap-3">
              <div className="w-8 h-8 rounded bg-slate-100 flex items-center justify-center shrink-0 border border-slate-200 animate-pulse">
                <span className="text-[8px] font-bold text-slate-400">THINK</span>
              </div>
              <div className="max-w-[80%] p-3 rounded-xl bg-slate-50 border border-slate-200 text-[10px] text-slate-500 font-mono italic">
                {currentThoughts.map((t, i) => (
                  <div key={i} className="animate-in fade-in slide-in-from-bottom-1 duration-300">{t}</div>
                ))}
              </div>
            </div>
          )}
        </div>

        {/* Input Area */}
        <div className="p-4 border-t border-white/20 bg-white/10">
          <div className="relative flex items-center">
            <input 
              type="text"
              value={input}
              onInput={(e) => setInput((e.target as HTMLInputElement).value)}
              onKeyPress={(e) => e.key === 'Enter' && sendMessage()}
              disabled={isTyping}
              placeholder={isTyping ? "ANALYZING..." : "ENTER COMMAND..."}
              className="w-full bg-white/60 border border-slate-200 rounded-lg py-3 px-4 pr-12 text-xs font-mono focus:outline-none focus:border-[#2e4de6] transition-colors placeholder:text-slate-300 disabled:opacity-50"
            />
            <button 
              onClick={sendMessage}
              disabled={isTyping}
              className="absolute right-2 p-2 text-[#2e4de6] hover:bg-[#2e4de6] hover:text-white rounded-md transition-all disabled:opacity-30"
            >
              <Send size={18} />
            </button>
          </div>
          <div className="mt-2 text-[8px] text-slate-400 font-mono text-center tracking-widest uppercase">
            {isTyping ? "Isolated instance processing neural pathways..." : "Sandbox: Real tools active, Output & Memory restricted"}
          </div>
        </div>
      </div>
    </div>
  );
}
