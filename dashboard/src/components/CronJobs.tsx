import { useState, useEffect, useCallback } from 'react';
import { memo } from 'react';
import { Clock, Plus, Trash2, Play, Power } from 'lucide-react';
import { ViewHeader } from './ViewHeader';
import { CronJob, AgentMetadata } from '../types';
import { api } from '../services/api';
import { useApiKey } from '../contexts/ApiKeyContext';

function formatSchedule(type: string, value: string): string {
  if (type === 'interval') {
    const secs = parseInt(value, 10);
    if (secs >= 3600) return `Every ${Math.floor(secs / 3600)}h${secs % 3600 ? ` ${Math.floor((secs % 3600) / 60)}m` : ''}`;
    if (secs >= 60) return `Every ${Math.floor(secs / 60)}m`;
    return `Every ${secs}s`;
  }
  if (type === 'once') return `Once at ${new Date(value).toLocaleString()}`;
  return value; // cron expression
}

function formatTimestamp(ms?: number | null): string {
  if (!ms) return '—';
  return new Date(ms).toLocaleString();
}

export const CronJobs = memo(function CronJobs() {
  const { apiKey } = useApiKey();
  const [jobs, setJobs] = useState<CronJob[]>([]);
  const [agents, setAgents] = useState<AgentMetadata[]>([]);
  const [showForm, setShowForm] = useState(false);
  const [form, setForm] = useState({
    agent_id: '',
    name: '',
    schedule_type: 'interval' as string,
    schedule_value: '3600',
    message: '',
    engine_id: '',
  });

  const fetchJobs = useCallback(async () => {
    if (!apiKey) return;
    try {
      const data = await api.listCronJobs(apiKey);
      setJobs(data.jobs);
    } catch (e) { console.error('Failed to fetch cron jobs', e); }
  }, [apiKey]);

  const fetchAgents = useCallback(async () => {
    try {
      const data = await api.getAgents();
      setAgents(data);
    } catch (e) { console.error('Failed to fetch agents', e); }
  }, []);

  useEffect(() => { fetchJobs(); fetchAgents(); }, [fetchJobs, fetchAgents]);

  const handleCreate = async () => {
    if (!apiKey || !form.agent_id || !form.name || !form.message) return;
    try {
      await api.createCronJob({
        agent_id: form.agent_id,
        name: form.name,
        schedule_type: form.schedule_type,
        schedule_value: form.schedule_value,
        message: form.message,
        engine_id: form.engine_id || undefined,
      }, apiKey);
      setShowForm(false);
      setForm({ agent_id: '', name: '', schedule_type: 'interval', schedule_value: '3600', message: '', engine_id: '' });
      fetchJobs();
    } catch (e: any) { alert(e.message); }
  };

  const handleToggle = async (job: CronJob) => {
    if (!apiKey) return;
    await api.toggleCronJob(job.id, !job.enabled, apiKey);
    fetchJobs();
  };

  const handleDelete = async (jobId: string) => {
    if (!apiKey || !confirm('Delete this cron job?')) return;
    await api.deleteCronJob(jobId, apiKey);
    fetchJobs();
  };

  const handleRunNow = async (jobId: string) => {
    if (!apiKey) return;
    await api.runCronJobNow(jobId, apiKey);
    fetchJobs();
  };

  return (
    <div className="bg-surface-base min-h-screen relative font-sans text-content-primary overflow-x-hidden animate-in fade-in duration-500">
      <div
        className="fixed left-0 right-0 bottom-0 z-0 opacity-30 pointer-events-none"
        style={{
          top: '41px',
          backgroundImage: `linear-gradient(to right, var(--canvas-grid) 1px, transparent 1px), linear-gradient(to bottom, var(--canvas-grid) 1px, transparent 1px)`,
          backgroundSize: '40px 40px',
          maskImage: 'linear-gradient(to bottom, black 40%, transparent 100%)',
          WebkitMaskImage: 'linear-gradient(to bottom, black 40%, transparent 100%)'
        }}
      />

      <ViewHeader
        icon={Clock}
        title="Cron Jobs"
        onBack="/"
        right={
          <button
            onClick={() => setShowForm(!showForm)}
            className="flex items-center gap-1.5 px-3 py-1 rounded bg-brand/10 text-brand hover:bg-brand/20 text-[10px] font-mono uppercase tracking-wider transition-colors"
          >
            <Plus size={12} /> New Job
          </button>
        }
      />

      <div className="relative z-10 p-6 md:p-12 space-y-6">
        {/* Create Form */}
        {showForm && (
          <div className="bg-glass-strong backdrop-blur-sm p-6 rounded-lg border border-edge space-y-4">
            <h3 className="text-xs font-bold text-content-secondary uppercase tracking-widest">New Cron Job</h3>
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              <div>
                <label className="block text-[10px] font-mono text-content-tertiary uppercase mb-1">Agent</label>
                <select
                  value={form.agent_id}
                  onChange={e => setForm({ ...form, agent_id: e.target.value })}
                  className="w-full bg-surface-secondary border border-edge rounded px-3 py-2 text-xs font-mono text-content-primary"
                >
                  <option value="">Select agent...</option>
                  {agents.filter(a => a.enabled).map(a => (
                    <option key={a.id} value={a.id}>{a.name} ({a.id})</option>
                  ))}
                </select>
              </div>
              <div>
                <label className="block text-[10px] font-mono text-content-tertiary uppercase mb-1">Name</label>
                <input
                  value={form.name}
                  onChange={e => setForm({ ...form, name: e.target.value })}
                  placeholder="e.g. Morning briefing"
                  className="w-full bg-surface-secondary border border-edge rounded px-3 py-2 text-xs font-mono text-content-primary"
                />
              </div>
              <div>
                <label className="block text-[10px] font-mono text-content-tertiary uppercase mb-1">Schedule Type</label>
                <select
                  value={form.schedule_type}
                  onChange={e => setForm({ ...form, schedule_type: e.target.value })}
                  className="w-full bg-surface-secondary border border-edge rounded px-3 py-2 text-xs font-mono text-content-primary"
                >
                  <option value="interval">Interval (seconds)</option>
                  <option value="cron">Cron Expression</option>
                  <option value="once">One-shot (ISO 8601)</option>
                </select>
              </div>
              <div>
                <label className="block text-[10px] font-mono text-content-tertiary uppercase mb-1">
                  {form.schedule_type === 'interval' ? 'Interval (seconds, min 60)' :
                   form.schedule_type === 'cron' ? 'Cron Expression (e.g. 0 9 * * *)' :
                   'Run At (ISO 8601)'}
                </label>
                <input
                  value={form.schedule_value}
                  onChange={e => setForm({ ...form, schedule_value: e.target.value })}
                  placeholder={form.schedule_type === 'interval' ? '3600' : form.schedule_type === 'cron' ? '0 9 * * *' : '2026-03-01T09:00:00+09:00'}
                  className="w-full bg-surface-secondary border border-edge rounded px-3 py-2 text-xs font-mono text-content-primary"
                />
              </div>
              <div className="md:col-span-2">
                <label className="block text-[10px] font-mono text-content-tertiary uppercase mb-1">Message (prompt sent to agent)</label>
                <textarea
                  value={form.message}
                  onChange={e => setForm({ ...form, message: e.target.value })}
                  placeholder="e.g. Summarize today's tasks and send a briefing."
                  rows={3}
                  className="w-full bg-surface-secondary border border-edge rounded px-3 py-2 text-xs font-mono text-content-primary resize-none"
                />
              </div>
            </div>
            <div className="flex gap-3 pt-2">
              <button
                onClick={handleCreate}
                disabled={!form.agent_id || !form.name || !form.message}
                className="px-4 py-2 bg-brand text-white rounded text-xs font-mono uppercase tracking-wider hover:bg-brand/80 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
              >
                Create Job
              </button>
              <button
                onClick={() => setShowForm(false)}
                className="px-4 py-2 bg-surface-secondary border border-edge text-content-secondary rounded text-xs font-mono uppercase tracking-wider hover:bg-surface-secondary/80 transition-colors"
              >
                Cancel
              </button>
            </div>
          </div>
        )}

        {/* Job List */}
        <div className="space-y-3">
          {jobs.length > 0 ? jobs.map(job => (
            <div
              key={job.id}
              className={`bg-glass-strong backdrop-blur-sm p-4 rounded-lg border transition-all duration-300 ${
                job.enabled ? 'border-edge hover:border-brand' : 'border-edge-subtle opacity-60'
              }`}
            >
              <div className="flex items-center justify-between gap-4">
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2 mb-1">
                    <span className={`w-2 h-2 rounded-full ${job.enabled ? 'bg-green-500' : 'bg-gray-500'}`} />
                    <span className="text-sm font-medium text-content-primary truncate">{job.name}</span>
                    <span className="text-[10px] font-mono text-content-muted px-1.5 py-0.5 bg-surface-secondary rounded">{job.schedule_type}</span>
                  </div>
                  <div className="text-[10px] font-mono text-content-tertiary space-y-0.5">
                    <div>Agent: <span className="text-content-secondary">{job.agent_id}</span></div>
                    <div>Schedule: <span className="text-content-secondary">{formatSchedule(job.schedule_type, job.schedule_value)}</span></div>
                    <div>Next: <span className="text-content-secondary">{job.next_run_at < Number.MAX_SAFE_INTEGER ? formatTimestamp(job.next_run_at) : '—'}</span></div>
                    {job.last_run_at && (
                      <div>Last: <span className="text-content-secondary">{formatTimestamp(job.last_run_at)}</span>
                        {job.last_status && (
                          <span className={`ml-2 px-1 py-0.5 rounded text-[9px] ${job.last_status === 'success' ? 'bg-green-500/20 text-green-400' : 'bg-red-500/20 text-red-400'}`}>
                            {job.last_status}
                          </span>
                        )}
                      </div>
                    )}
                  </div>
                  <div className="mt-1 text-[10px] font-mono text-content-muted truncate" title={job.message}>
                    Prompt: {job.message}
                  </div>
                </div>
                <div className="flex items-center gap-2 shrink-0">
                  <button onClick={() => handleRunNow(job.id)} title="Run now" className="p-1.5 rounded hover:bg-brand/10 text-content-tertiary hover:text-brand transition-colors">
                    <Play size={14} />
                  </button>
                  <button onClick={() => handleToggle(job)} title={job.enabled ? 'Disable' : 'Enable'} className="p-1.5 rounded hover:bg-brand/10 text-content-tertiary hover:text-brand transition-colors">
                    <Power size={14} className={job.enabled ? 'text-green-500' : 'text-gray-500'} />
                  </button>
                  <button onClick={() => handleDelete(job.id)} title="Delete" className="p-1.5 rounded hover:bg-red-500/10 text-content-tertiary hover:text-red-400 transition-colors">
                    <Trash2 size={14} />
                  </button>
                </div>
              </div>
            </div>
          )) : (
            <div className="py-12 text-center text-content-tertiary bg-glass rounded-lg border border-edge border-dashed font-mono text-xs">
              No cron jobs configured. Create one to enable autonomous agent execution.
            </div>
          )}
        </div>
      </div>
    </div>
  );
});
