import React, { useState, useEffect, useRef } from 'react';
import { Cpu, ChevronRight, Puzzle, Activity, MessageSquare, Send, Zap, User as UserIcon } from 'lucide-react';
import { AgentMetadata, PluginManifest, ExivMessage } from '../types';
import { AgentPluginWorkspace } from './AgentPluginWorkspace';
import { useEventStream } from '../hooks/useEventStream';

import { api, API_BASE } from '../services/api';

export interface AgentTerminalProps {
  agents?: AgentMetadata[];
  plugins?: PluginManifest[];
  selectedAgent?: AgentMetadata | null;
  onSelectAgent?: (agent: AgentMetadata | null) => void;
}

function AgentConsole({ agent, onBack }: { agent: AgentMetadata, onBack: () => void }) {
  const [messages, setMessages] = useState<ExivMessage[]>([]);
  const [input, setInput] = useState('');
  const [isTyping, setIsTyping] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [messages]);

  // Subscribe to system-wide events
  useEventStream(`${API_BASE}/events`, (event) => {
    if (event.type === 'ThoughtResponse' && event.data.agent_id === agent.id) {
      setIsTyping(false);
      const newMsg: ExivMessage = {
        id: event.data.source_message_id + "-resp",
        source: { type: 'Agent', id: agent.id },
        content: event.data.content,
        timestamp: new Date().toISOString(),
        metadata: {}
      };
      setMessages(prev => [...prev, newMsg]);
    }
  });

  const sendMessage = async () => {
    if (!input.trim() || isTyping) return;

    const userMsg: ExivMessage = {
      id: Date.now().toString(),
      source: { type: 'User', id: 'user', name: 'User' },
      target_agent: agent.id,
      content: input,
      timestamp: new Date().toISOString(),
      metadata: { target_agent_id: agent.id }
    };

    setMessages(prev => [...prev, userMsg]);
    setInput('');
    setIsTyping(true);

    try {
      // H-16: Check response status for errors
      const res = await fetch('/api/chat', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(userMsg)
      });
      if (!res.ok) {
        throw new Error(`Chat request failed: ${res.status}`);
      }
    } catch (err) {
      console.error("Failed to send message:", err);
      setIsTyping(false);
    }
  };

  return (
    <div className="flex flex-col h-full bg-white/40 backdrop-blur-3xl animate-in fade-in duration-500">
      {/* Console Header */}
      <div className="p-4 border-b border-slate-100 flex items-center justify-between bg-white/60">
        <div className="flex items-center gap-3">
          <div className="p-2 bg-[#2e4de6] text-white rounded-lg shadow-lg shadow-[#2e4de6]/20">
            <Cpu size={18} />
          </div>
          <div>
            <h2 className="text-sm font-black text-slate-800 tracking-tight uppercase">{agent.name} Console</h2>
            <div className="flex items-center gap-2">
              <span className="w-1.5 h-1.5 bg-emerald-500 rounded-full animate-pulse" />
              <span className="text-[8px] font-mono text-slate-400 uppercase tracking-widest">Neural Link Active</span>
            </div>
          </div>
        </div>
        <button 
          onClick={onBack}
          className="px-4 py-1.5 rounded-full border border-slate-200 text-[9px] font-bold text-slate-400 hover:text-[#2e4de6] hover:border-[#2e4de6]/30 transition-all uppercase tracking-widest"
        >
          Disconnect
        </button>
      </div>

      {/* Message Stream */}
      <div ref={scrollRef} className="flex-1 overflow-y-auto p-6 space-y-4 no-scrollbar">
        {messages.length === 0 && (
          <div className="h-full flex flex-col items-center justify-center text-slate-300 space-y-4">
            <Zap size={32} strokeWidth={1} className="opacity-20" />
            <p className="text-[10px] font-mono tracking-[0.2em] uppercase">Ready for instructions</p>
          </div>
        )}
        {messages.map((msg) => {
          const isUser = msg.source.type === 'User';
          return (
            <div key={msg.id} className={`flex items-start gap-3 ${isUser ? 'flex-row-reverse' : ''}`}>
              <div className={`w-8 h-8 rounded-lg flex items-center justify-center shrink-0 shadow-sm ${
                isUser ? 'bg-white border border-slate-100 text-slate-400' : 'bg-[#2e4de6] text-white'
              }`}>
                {isUser ? <UserIcon size={14} /> : <span className="text-[10px] font-black">AI</span>}
              </div>
              <div className={`max-w-[80%] p-4 rounded-2xl text-xs leading-relaxed shadow-sm ${
                isUser 
                  ? 'bg-white text-slate-700 rounded-tr-none'
                  : 'bg-[#2e4de6] text-white rounded-tl-none'
              }`}>
                {msg.content}
              </div>
            </div>
          );
        })}
        {isTyping && (
          <div className="flex items-start gap-3 animate-pulse">
            <div className="w-8 h-8 rounded-lg bg-[#2e4de6] text-white flex items-center justify-center shrink-0">
              <Activity size={14} />
            </div>
            <div className="bg-slate-100 text-slate-400 p-3 rounded-2xl rounded-tl-none text-[10px] font-mono">
              THINKING...
            </div>
          </div>
        )}
      </div>

      {/* Input Area */}
      <div className="p-4 bg-white/60 border-t border-slate-100">
        <div className="relative flex items-center">
          <input 
            type="text"
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyPress={(e) => e.key === 'Enter' && sendMessage()}
            disabled={isTyping}
            placeholder={isTyping ? "PROCESSING..." : "ENTER COMMAND..."}
            className="w-full bg-white border border-slate-200 rounded-xl py-3 px-4 pr-12 text-xs font-mono focus:outline-none focus:border-[#2e4de6] transition-colors placeholder:text-slate-300 disabled:opacity-50 shadow-inner"
          />
          <button 
            onClick={sendMessage}
            disabled={isTyping || !input.trim()}
            className="absolute right-2 p-2 bg-[#2e4de6] text-white rounded-lg hover:scale-105 active:scale-95 transition-all disabled:opacity-30 disabled:grayscale disabled:scale-100 shadow-lg shadow-[#2e4de6]/20"
          >
            <Send size={16} />
          </button>
        </div>
      </div>
    </div>
  );
}

export function AgentTerminal({ 
  agents: propAgents, 
  plugins: propPlugins,
  selectedAgent: propSelectedAgent, 
  onSelectAgent: propOnSelectAgent 
}: AgentTerminalProps = {}) {
  const [internalAgents, setInternalAgents] = useState<AgentMetadata[]>([]);
  const [internalPlugins, setInternalPlugins] = useState<PluginManifest[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [internalSelectedAgent, setInternalSelectedAgent] = useState<AgentMetadata | null>(null);
  const [configuringAgent, setConfiguringAgent] = useState<AgentMetadata | null>(null);

  const agents = propAgents || internalAgents;
  const plugins = propPlugins || internalPlugins;
  const selectedAgent = propSelectedAgent !== undefined ? propSelectedAgent : internalSelectedAgent;

  useEffect(() => {
    if (!propAgents) {
      fetchInitialData();
    } else {
      setIsLoading(false);
    }
  }, [propAgents]);

  const fetchInitialData = async () => {
    try {
      const [agentsData, pluginsData] = await Promise.all([
        api.getAgents(),
        api.getPlugins()
      ]);
      setInternalAgents(agentsData);
      setInternalPlugins(pluginsData);
    } catch (err) {
      console.error('Failed to fetch data:', err);
    } finally {
      setIsLoading(false);
    }
  };

  const handleSelectAgent = (agent: AgentMetadata | null) => {
    if (propOnSelectAgent) {
      propOnSelectAgent(agent);
    } else {
      setInternalSelectedAgent(agent);
    }
  };

  if (configuringAgent) {
    return (
      <AgentPluginWorkspace 
        agent={configuringAgent}
        availablePlugins={plugins.filter(p => p.is_active)}
        onBack={() => setConfiguringAgent(null)}
      />
    );
  }

  if (selectedAgent) {
    return <AgentConsole agent={selectedAgent} onBack={() => handleSelectAgent(null)} />;
  }

  return (
    <div className="flex flex-col h-full bg-white/20 backdrop-blur-3xl p-6 overflow-hidden">
      {/* Header */}
      <div className="mb-8 px-4 flex justify-between items-end">
        <div>
          <h2 className="text-2xl font-black tracking-tighter text-slate-800 uppercase leading-none">AI Containers</h2>
          <p className="text-[10px] text-slate-400 font-mono tracking-[0.3em] uppercase mt-2">Exiv Registered Instances</p>
        </div>
        <div className="text-[9px] font-mono text-slate-300">SYSTEM_UPTIME: 100%</div>
      </div>

      {/* Container List */}
      <div className="flex-1 overflow-y-auto space-y-2 no-scrollbar px-2">
        {isLoading ? (
          <div className="h-full flex items-center justify-center text-slate-300 font-mono text-[10px] tracking-widest uppercase animate-pulse">
            Scanning for containers...
          </div>
        ) : (
          agents.map((agent) => (
            <div 
              key={agent.id}
              className="group flex h-24 bg-white/60 hover:bg-white border border-slate-100 rounded-2xl overflow-hidden transition-all duration-500 shadow-sm"
            >
              {/* Left / Main Info */}
              <div 
                onClick={() => handleSelectAgent(agent)}
                className="flex-1 flex items-center px-8 cursor-pointer relative overflow-hidden"
              >
                <div className="absolute left-0 top-0 w-1 h-full bg-[#2e4de6] opacity-0 group-hover:opacity-100 transition-opacity" />
                <div className="w-12 h-12 rounded-xl bg-slate-50 flex items-center justify-center text-[#2e4de6] mr-6 border border-slate-100 group-hover:bg-[#2e4de6] group-hover:text-white transition-colors duration-500">
                  <Cpu size={24} />
                </div>
                <div>
                  <div className="flex items-center gap-3">
                    <h3 className="text-lg font-black text-slate-800 tracking-tight">{agent.name}</h3>
                    <span className="flex items-center gap-1 px-2 py-0.5 rounded bg-emerald-50 text-emerald-500 text-[8px] font-bold tracking-widest uppercase">
                       <Activity size={8} /> Online
                    </span>
                  </div>
                  <p className="text-[10px] text-slate-400 mt-1 font-mono">{agent.description}</p>
                </div>
                <ChevronRight size={20} className="ml-auto text-slate-200 group-hover:text-[#2e4de6] transition-colors" />
              </div>

              {/* Right Small Square (Plugin Connection Port) */}
              <button 
                title="Manage Container Plugins"
                className="w-24 h-full border-l border-slate-100 bg-slate-50/50 hover:bg-[#2e4de6]/10 flex flex-col items-center justify-center gap-2 transition-all hover:text-[#2e4de6]"
                onClick={(e) => {
                   e.stopPropagation();
                   setConfiguringAgent(agent);
                }}
              >
                <Puzzle size={20} className="text-slate-400 group-hover:text-[#2e4de6]/80 transition-colors" />
                <span className="text-[8px] font-black tracking-tighter uppercase text-slate-400 group-hover:text-[#2e4de6]">Plugins</span>
              </button>
            </div>
          ))
        )}
      </div>

      {/* Footer / Stats */}
      <div className="mt-6 pt-6 border-t border-slate-100/50 flex justify-between items-center px-4">
        <div className="flex gap-6">
           <div className="flex flex-col">
             <span className="text-[8px] font-mono text-slate-300 uppercase">Total Memory</span>
             <span className="text-[10px] font-bold text-slate-500">2.4 GB</span>
           </div>
           <div className="flex flex-col">
             <span className="text-[8px] font-mono text-slate-300 uppercase">Active Units</span>
             <span className="text-[10px] font-bold text-slate-500">{agents.length} Agent(s)</span>
           </div>
        </div>
        <div className="p-2 rounded-lg bg-slate-50 border border-slate-100">
           <MessageSquare size={16} className="text-slate-300" />
        </div>
      </div>
    </div>
  );
}