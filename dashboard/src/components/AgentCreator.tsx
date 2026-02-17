import { useState, useEffect } from 'react';
import { Plus, Activity, Lock } from 'lucide-react';
import { AgentMetadata, PluginManifest } from '../types';
import { AgentIcon, agentColor, AgentTypeIcon, agentTypeColor, AgentType, statusBadgeClass } from '../lib/agentIdentity';

export function AgentCreator({ onAgentCreated }: { onAgentCreated?: () => void }) {
  const [agents, setAgents] = useState<AgentMetadata[]>([]);
  const [plugins, setPlugins] = useState<PluginManifest[]>([]);
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [engine, setEngine] = useState('');
  const [memory, setMemory] = useState('');
  const [agentType, setAgentType] = useState<AgentType>('ai');
  const [password, setPassword] = useState('');
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
          metadata: { preferred_memory: memory, agent_type: agentType },
          password: password || undefined
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
        setAgentType('ai');
        setPassword('');
      }
    } catch (e) {
      console.error(e);
    } finally {
      setIsCreating(false);
    }
  };

  const isLlmPlugin = (p: PluginManifest) => p.tags.includes('#MIND') || p.tags.includes('#LLM');
  const allEngines = plugins.filter(p => p.service_type === 'Reasoning' && p.is_active && p.category === 'Agent');
  const engines = allEngines.filter(p => agentType === 'ai' ? isLlmPlugin(p) : !isLlmPlugin(p));
  const allMemories = plugins.filter(p => (p.service_type === 'Memory' || p.category === 'Memory') && p.is_active);
  const memories = allMemories.filter(p => agentType === 'ai' ? true : !isLlmPlugin(p));

  const handleTypeChange = (type: AgentType) => {
    setAgentType(type);
    setEngine('');
  };

  return (
    <div className="bg-white/80 backdrop-blur-sm p-6 rounded-lg border border-slate-200 shadow-sm h-full overflow-auto">
      <h2 className="text-lg font-bold text-slate-700 flex items-center gap-2 mb-6">
        <Activity size={20} className="text-[#2e4de6]" />
        Agent Management
      </h2>

      <div className="grid grid-cols-1 md:grid-cols-2 gap-8">
        {/* List */}
        <div className="space-y-4">
          <h3 className="text-xs font-bold text-slate-400 uppercase tracking-widest mb-2">Active Agents</h3>
          <div className="space-y-3">
            {agents.map(a => (
              <div key={a.id} className="p-4 bg-white border border-slate-100 rounded-lg shadow-sm flex items-center gap-4 group hover:border-[#2e4de6]/30 transition-all">
                <div className="p-2 rounded-lg" style={{ backgroundColor: `${agentColor(a)}15`, color: agentColor(a) }}>
                  <AgentIcon agent={a} size={20} />
                </div>
                <div className="flex-1">
                  <div className="font-bold text-slate-700 text-sm">{a.name}</div>
                  <div className="text-[11px] text-slate-500">{a.description}</div>
                  <div className="flex gap-2 mt-2">
                     <span className="text-[9px] bg-slate-100 px-1.5 py-0.5 rounded text-slate-400 font-mono">BRAIN: {a.metadata.default_engine_id || 'DEFAULT'}</span>
                     <span className="text-[9px] bg-slate-100 px-1.5 py-0.5 rounded text-slate-400 font-mono">MEM: {a.metadata.preferred_memory || 'DEFAULT'}</span>
                  </div>
                </div>
                <div className={`inline-flex items-center gap-1 px-2 py-1 rounded text-[10px] font-bold ${statusBadgeClass(a.status)}`}>
                  {a.metadata?.has_power_password === 'true' && <Lock size={8} />}
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
            {/* Agent Type Selector */}
            <div>
              <label className="block text-xs font-bold text-slate-500 mb-2">Agent Type</label>
              <div className="grid grid-cols-2 gap-3">
                {([['ai', 'AI Agent', 'LLM-powered reasoning agent'], ['container', 'Container', 'Script / bridge-based process']] as const).map(([type, label, desc]) => {
                  const selected = agentType === type;
                  const color = agentTypeColor(type);
                  return (
                    <button
                      key={type}
                      type="button"
                      onClick={() => handleTypeChange(type)}
                      className={`flex items-center gap-3 p-3 rounded-xl border-2 transition-all text-left ${
                        selected ? 'bg-white shadow-md' : 'bg-white/50 border-slate-200 hover:border-slate-300'
                      }`}
                      style={selected ? { borderColor: color } : undefined}
                    >
                      <div
                        className="p-2 rounded-lg text-white shrink-0"
                        style={{ backgroundColor: selected ? color : '#94a3b8' }}
                      >
                        <AgentTypeIcon type={type} size={18} />
                      </div>
                      <div>
                        <div className="text-xs font-bold text-slate-700">{label}</div>
                        <div className="text-[9px] text-slate-400">{desc}</div>
                      </div>
                    </button>
                  );
                })}
              </div>
            </div>

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
                <label className="block text-xs font-bold text-slate-500 mb-1">
                  {agentType === 'ai' ? 'LLM Engine' : 'Bridge Engine'}
                </label>
                {engines.length > 0 ? (
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
                ) : (
                  <div className="w-full px-2 py-1.5 rounded border border-dashed border-slate-300 text-[10px] text-slate-400 font-mono text-center">
                    No {agentType === 'ai' ? 'LLM' : 'bridge'} engines available
                  </div>
                )}
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

            <div>
              <label className="block text-xs font-bold text-slate-500 mb-1">
                Power Password <span className="text-slate-300 font-normal">(optional)</span>
              </label>
              <div className="relative">
                <Lock size={14} className="absolute left-3 top-1/2 -translate-y-1/2 text-slate-300" />
                <input
                  type="password"
                  value={password}
                  onChange={e => setPassword(e.target.value)}
                  className="w-full pl-9 pr-3 py-2 rounded border border-slate-200 text-sm focus:outline-none focus:border-[#2e4de6]"
                  placeholder="Leave empty for no password"
                />
              </div>
              <p className="text-[9px] text-slate-400 mt-1">Require password to toggle power on/off</p>
            </div>

            <button
              onClick={handleCreate}
              disabled={!name || !engine || isCreating}
              className="w-full mt-4 text-white py-2.5 rounded text-sm font-bold shadow-sm hover:shadow-md transition-all disabled:opacity-50 disabled:cursor-not-allowed flex items-center justify-center gap-2"
              style={{ backgroundColor: agentTypeColor(agentType) }}
            >
              {isCreating ? <Activity size={16} className="animate-spin" /> : <Plus size={16} />}
              {agentType === 'ai' ? 'CREATE AI AGENT' : 'CREATE CONTAINER'}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}