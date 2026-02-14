import { useState, useEffect } from 'react';
import { User, Plus, Cpu, Activity } from 'lucide-react';
import { AgentMetadata, PluginManifest } from '../types';

export function AgentCreator({ onAgentCreated }: { onAgentCreated?: () => void }) {
  const [agents, setAgents] = useState<AgentMetadata[]>([]);
  const [plugins, setPlugins] = useState<PluginManifest[]>([]);
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [engine, setEngine] = useState('');
  const [memory, setMemory] = useState('');
  const [isCreating, setIsCreating] = useState(false);

  useEffect(() => {
    fetch('/api/agents').then(r => r.json()).then(setAgents).catch(console.error);
    fetch('/api/plugins').then(r => r.json()).then(setPlugins).catch(console.error);
  }, []);

  const handleCreate = async () => {
    setIsCreating(true);
    try {
      const res = await fetch('/api/agents', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ 
          name, 
          description, 
          default_engine: engine,
          metadata: { preferred_memory: memory }
        })
      });
      if (res.ok) {
        // Refresh agents
        fetch('/api/agents').then(r => r.json()).then(setAgents);
        if (onAgentCreated) onAgentCreated();
        setName('');
        setDescription('');
        setEngine('');
        setMemory('');
      }
    } catch (e) {
      console.error(e);
    } finally {
      setIsCreating(false);
    }
  };

  const engines = plugins.filter(p => p.service_type === 'Reasoning' && p.is_active);
  const memories = plugins.filter(p => p.service_type === 'Memory' && p.is_active);

  return (
    <div className="bg-white/80 backdrop-blur-sm p-6 rounded-lg border border-slate-200 shadow-sm h-full overflow-auto">
      <h2 className="text-lg font-bold text-slate-700 flex items-center gap-2 mb-6">
        <User size={20} className="text-[#2e4de6]" />
        Agent Management
      </h2>

      <div className="grid grid-cols-1 md:grid-cols-2 gap-8">
        {/* List */}
        <div className="space-y-4">
          <h3 className="text-xs font-bold text-slate-400 uppercase tracking-widest mb-2">Active Agents</h3>
          <div className="space-y-3">
            {agents.map(a => (
              <div key={a.id} className="p-4 bg-white border border-slate-100 rounded-lg shadow-sm flex items-center justify-between group hover:border-[#2e4de6]/30 transition-all">
                <div>
                  <div className="font-bold text-slate-700 text-sm">{a.name}</div>
                  <div className="text-[11px] text-slate-500">{a.description}</div>
                  <div className="flex gap-2 mt-2">
                     <span className="text-[9px] bg-slate-100 px-1.5 py-0.5 rounded text-slate-400 font-mono">BRAIN: {a.metadata.default_engine_id || 'DEFAULT'}</span>
                     <span className="text-[9px] bg-slate-100 px-1.5 py-0.5 rounded text-slate-400 font-mono">MEM: {a.metadata.preferred_memory || 'DEFAULT'}</span>
                  </div>
                </div>
                <div className={`px-2 py-1 rounded text-[10px] font-bold ${a.status === 'online' ? 'bg-green-100 text-green-600' : 'bg-slate-100 text-slate-400'}`}>
                  {a.status.toUpperCase()}
                </div>
              </div>
            ))}
          </div>
        </div>

        {/* Create Form */}
        <div className="bg-slate-50 p-6 rounded-lg border border-slate-200 h-fit">
          <h3 className="text-xs font-bold text-slate-400 uppercase tracking-widest mb-4">Initialize New Agent</h3>
          
          <div className="space-y-4">
            <div>
              <label className="block text-xs font-bold text-slate-500 mb-1">Agent Name</label>
              <input 
                type="text" 
                value={name}
                onChange={e => setName(e.target.value)}
                className="w-full px-3 py-2 rounded border border-slate-200 text-sm focus:outline-none focus:border-[#2e4de6]"
                placeholder="e.g. Mike"
              />
            </div>
            
            <div>
              <label className="block text-xs font-bold text-slate-500 mb-1">Description / System Prompt</label>
              <textarea 
                value={description}
                onChange={e => setDescription(e.target.value)}
                className="w-full px-3 py-2 rounded border border-slate-200 text-sm focus:outline-none focus:border-[#2e4de6] h-16 resize-none"
                placeholder="Briefly describe the agent's personality and role."
              />
            </div>

            <div className="grid grid-cols-2 gap-4">
              <div>
                <label className="block text-xs font-bold text-slate-500 mb-1">Reasoning Engine</label>
                <select 
                  value={engine} 
                  onChange={e => setEngine(e.target.value)}
                  className="w-full px-2 py-1.5 rounded border border-slate-200 text-xs focus:outline-none focus:border-[#2e4de6] bg-white"
                >
                  <option value="">Select Engine...</option>
                  {engines.map(p => (
                    <option key={p.id} value={p.id}>{p.name}</option>
                  ))}
                </select>
              </div>

              <div>
                <label className="block text-xs font-bold text-slate-500 mb-1">Memory Engine</label>
                <select 
                  value={memory} 
                  onChange={e => setMemory(e.target.value)}
                  className="w-full px-2 py-1.5 rounded border border-slate-200 text-xs focus:outline-none focus:border-[#2e4de6] bg-white"
                >
                  <option value="">Select Memory...</option>
                  {memories.map(p => (
                    <option key={p.id} value={p.id}>{p.name}</option>
                  ))}
                </select>
              </div>
            </div>

            <button
              onClick={handleCreate}
              disabled={!name || !engine || isCreating}
              className="w-full mt-4 bg-[#2e4de6] text-white py-2.5 rounded text-sm font-bold shadow-sm hover:shadow-md transition-all disabled:opacity-50 disabled:cursor-not-allowed flex items-center justify-center gap-2"
            >
              {isCreating ? <Activity size={16} className="animate-spin" /> : <Plus size={16} />}
              CREATE NEURAL AGENT
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}