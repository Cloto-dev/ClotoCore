import { useState, useEffect, useCallback } from 'react';
import { memo } from 'react';
import { Clock, Plus, Trash2, Play, Power, AlertTriangle } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { CronJob, AgentMetadata } from '../types';
import { useApi } from '../hooks/useApi';
import { useUserIdentity } from '../contexts/UserIdentityContext';
import { Modal } from './Modal';

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
  const api = useApi();
  const { identity } = useUserIdentity();
  const { t } = useTranslation('cron');
  const { t: tc } = useTranslation('common');
  const [jobs, setJobs] = useState<CronJob[]>([]);
  const [agents, setAgents] = useState<AgentMetadata[]>([]);
  const [showForm, setShowForm] = useState(false);
  const [notification, setNotification] = useState<{ type: 'success' | 'error'; message: string } | null>(null);
  const [confirmDialog, setConfirmDialog] = useState<{ message: string; onConfirm: () => void } | null>(null);
  const [form, setForm] = useState({
    agent_id: '',
    name: '',
    schedule_type: 'interval' as string,
    schedule_value: '3600',
    message: '',
    engine_id: '',
    hide_prompt: false,
    source_type: 'system' as 'user' | 'system',
  });

  const fetchJobs = useCallback(async () => {
    try {
      const data = await api.listCronJobs();
      setJobs(data.jobs);
    } catch (e) { console.error('Failed to fetch cron jobs', e); }
  }, [api]);

  const fetchAgents = useCallback(async () => {
    try {
      const data = await api.getAgents();
      setAgents(data);
    } catch (e) { console.error('Failed to fetch agents', e); }
  }, [api]);

  useEffect(() => { fetchJobs(); fetchAgents(); }, [fetchJobs, fetchAgents]);

  const handleCreate = async () => {
    if (!form.agent_id || !form.name || !form.message) return;
    try {
      await api.createCronJob({
        agent_id: form.agent_id,
        name: form.name,
        schedule_type: form.schedule_type,
        schedule_value: form.schedule_value,
        message: form.message,
        engine_id: form.engine_id || undefined,
        hide_prompt: form.hide_prompt || undefined,
        source_type: form.source_type,
        creator_user_id: form.source_type === 'user' ? identity.id : undefined,
        creator_user_name: form.source_type === 'user' ? identity.name : undefined,
      });
      setShowForm(false);
      setForm({ agent_id: '', name: '', schedule_type: 'interval', schedule_value: '3600', message: '', engine_id: '', hide_prompt: false, source_type: 'system' });
      fetchJobs();
    } catch (e: unknown) { setNotification({ type: 'error', message: e instanceof Error ? e.message : String(e) }); }
  };

  const handleToggle = async (job: CronJob) => {
    await api.toggleCronJob(job.id, !job.enabled);
    fetchJobs();
  };

  const handleDelete = (jobId: string) => {
    setConfirmDialog({
      message: t('delete_confirm'),
      onConfirm: async () => {
        setConfirmDialog(null);
        await api.deleteCronJob(jobId);
        fetchJobs();
      },
    });
  };

  const handleRunNow = async (jobId: string) => {
    await api.runCronJobNow(jobId);
    fetchJobs();
  };

  return (
    <div className="h-full relative font-sans text-content-primary overflow-x-hidden overflow-y-auto animate-in fade-in duration-500">
      <div className="relative z-10 p-6 md:p-12 space-y-6">
        {/* Inline header with New Job button */}
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Clock className="text-brand" size={16} />
            <h2 className="text-xs font-mono uppercase tracking-widest text-content-primary font-bold">{t('title')}</h2>
          </div>
          <button
            onClick={() => setShowForm(!showForm)}
            className="flex items-center gap-1.5 px-3 py-1 rounded bg-brand/10 text-brand hover:bg-brand/20 text-[10px] font-mono uppercase tracking-wider transition-colors"
          >
            <Plus size={12} /> {t('new_job')}
          </button>
        </div>
        {/* Create Form */}
        {showForm && (
          <div className="bg-glass-strong backdrop-blur-sm p-6 rounded-lg border border-edge space-y-4">
            <h3 className="text-xs font-bold text-content-secondary uppercase tracking-widest">{t('new_cron_job')}</h3>
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              <div>
                <label className="block text-[10px] font-mono text-content-tertiary uppercase mb-1">{t('agent')}</label>
                <select
                  value={form.agent_id}
                  onChange={e => setForm({ ...form, agent_id: e.target.value })}
                  className="w-full bg-surface-secondary border border-edge rounded px-3 py-2 text-xs font-mono text-content-primary"
                >
                  <option value="">{t('select_agent')}</option>
                  {agents.filter(a => a.enabled).map(a => (
                    <option key={a.id} value={a.id}>{a.name} ({a.id})</option>
                  ))}
                </select>
              </div>
              <div>
                <label className="block text-[10px] font-mono text-content-tertiary uppercase mb-1">{t('name')}</label>
                <input
                  value={form.name}
                  onChange={e => setForm({ ...form, name: e.target.value })}
                  placeholder={t('name_placeholder')}
                  className="w-full bg-surface-secondary border border-edge rounded px-3 py-2 text-xs font-mono text-content-primary"
                />
              </div>
              <div>
                <label className="block text-[10px] font-mono text-content-tertiary uppercase mb-1">{t('schedule_type')}</label>
                <select
                  value={form.schedule_type}
                  onChange={e => setForm({ ...form, schedule_type: e.target.value })}
                  className="w-full bg-surface-secondary border border-edge rounded px-3 py-2 text-xs font-mono text-content-primary"
                >
                  <option value="interval">{t('type_interval')}</option>
                  <option value="cron">{t('type_cron')}</option>
                  <option value="once">{t('type_once')}</option>
                </select>
              </div>
              <div>
                <label className="block text-[10px] font-mono text-content-tertiary uppercase mb-1">
                  {form.schedule_type === 'interval' ? t('label_interval') :
                   form.schedule_type === 'cron' ? t('label_cron') :
                   t('label_once')}
                </label>
                <input
                  value={form.schedule_value}
                  onChange={e => setForm({ ...form, schedule_value: e.target.value })}
                  placeholder={form.schedule_type === 'interval' ? '3600' : form.schedule_type === 'cron' ? '0 9 * * *' : '2026-03-01T09:00:00+09:00'}
                  className="w-full bg-surface-secondary border border-edge rounded px-3 py-2 text-xs font-mono text-content-primary"
                />
              </div>
              <div className="md:col-span-2">
                <label className="block text-[10px] font-mono text-content-tertiary uppercase mb-1">{t('message')}</label>
                <textarea
                  value={form.message}
                  onChange={e => setForm({ ...form, message: e.target.value })}
                  placeholder={t('message_placeholder')}
                  rows={3}
                  className="w-full bg-surface-secondary border border-edge rounded px-3 py-2 text-xs font-mono text-content-primary resize-none"
                />
              </div>
              <div>
                <label className="block text-[10px] font-mono text-content-tertiary uppercase mb-1">{t('source_type')}</label>
                <select
                  value={form.source_type}
                  onChange={e => setForm({ ...form, source_type: e.target.value as 'user' | 'system' })}
                  className="w-full bg-surface-secondary border border-edge rounded px-3 py-2 text-xs font-mono text-content-primary"
                >
                  <option value="system">{t('source_system')}</option>
                  <option value="user">{t('source_user')}</option>
                </select>
              </div>
              <div className="md:col-span-2">
                <label className="flex items-center gap-2 cursor-pointer select-none">
                  <input
                    type="checkbox"
                    checked={form.hide_prompt}
                    onChange={e => setForm({ ...form, hide_prompt: e.target.checked })}
                    className="rounded border-edge bg-surface-secondary"
                  />
                  <span className="text-[10px] font-mono text-content-tertiary uppercase">
                    {t('hide_prompt')}
                  </span>
                </label>
              </div>
            </div>
            <div className="flex gap-3 pt-2">
              <button
                onClick={handleCreate}
                disabled={!form.agent_id || !form.name || !form.message}
                className="px-4 py-2 bg-brand text-white rounded text-xs font-mono uppercase tracking-wider hover:bg-brand/80 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
              >
                {t('create_job')}
              </button>
              <button
                onClick={() => setShowForm(false)}
                className="px-4 py-2 bg-surface-secondary border border-edge text-content-secondary rounded text-xs font-mono uppercase tracking-wider hover:bg-surface-secondary/80 transition-colors"
              >
                {tc('cancel')}
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
                    <span className="text-[10px] font-mono text-content-tertiary px-1.5 py-0.5 bg-surface-secondary rounded">{job.schedule_type}</span>
                    {job.hide_prompt && (
                      <span className="text-[10px] font-mono text-brand px-1.5 py-0.5 bg-brand/10 rounded">agent</span>
                    )}
                    {(job.cron_generation ?? 0) > 0 && (
                      <span className="text-[10px] font-mono text-amber-400 px-1.5 py-0.5 bg-amber-500/10 rounded">gen:{job.cron_generation}</span>
                    )}
                  </div>
                  <div className="text-[10px] font-mono text-content-tertiary space-y-0.5">
                    <div>{t('agent_label')} <span className="text-content-secondary">{job.agent_id}</span></div>
                    <div>{t('schedule_label')} <span className="text-content-secondary">{formatSchedule(job.schedule_type, job.schedule_value)}</span></div>
                    <div>{t('next_label')} <span className="text-content-secondary">{job.next_run_at < Number.MAX_SAFE_INTEGER ? formatTimestamp(job.next_run_at) : '—'}</span></div>
                    {job.last_run_at && (
                      <div>{t('last_label')} <span className="text-content-secondary">{formatTimestamp(job.last_run_at)}</span>
                        {job.last_status && (
                          <span className={`ml-2 px-1 py-0.5 rounded text-[9px] ${job.last_status === 'success' ? 'bg-green-500/20 text-green-400' : 'bg-red-500/20 text-red-400'}`}>
                            {job.last_status}
                          </span>
                        )}
                      </div>
                    )}
                  </div>
                  <div className="mt-1 text-[10px] font-mono text-content-tertiary truncate" title={job.message}>
                    {t('prompt_label')} {job.message}
                  </div>
                  <div className="mt-0.5 text-[10px] font-mono text-content-tertiary">
                    {t('source_label')} <span className={`px-1 py-0.5 rounded ${
                      job.source_type === 'user'
                        ? 'bg-green-500/10 text-green-400'
                        : 'bg-blue-500/10 text-blue-400'
                    }`}>
                      {job.source_type === 'user'
                        ? `${t('source_user')}: ${job.creator_user_name || job.creator_user_id || '?'}`
                        : t('source_system')}
                    </span>
                  </div>
                </div>
                <div className="flex items-center gap-2 shrink-0">
                  <button onClick={() => handleRunNow(job.id)} title={t('run_now')} className="p-1.5 rounded hover:bg-brand/10 text-content-tertiary hover:text-brand transition-colors">
                    <Play size={14} />
                  </button>
                  <button onClick={() => handleToggle(job)} title={job.enabled ? t('disable') : t('enable')} className="p-1.5 rounded hover:bg-brand/10 text-content-tertiary hover:text-brand transition-colors">
                    <Power size={14} className={job.enabled ? 'text-green-500' : 'text-gray-500'} />
                  </button>
                  <button onClick={() => handleDelete(job.id)} title={tc('delete')} className="p-1.5 rounded hover:bg-red-500/10 text-content-tertiary hover:text-red-400 transition-colors">
                    <Trash2 size={14} />
                  </button>
                </div>
              </div>
            </div>
          )) : (
            <div className="py-12 text-center text-content-tertiary bg-glass rounded-lg border border-edge border-dashed font-mono text-xs">
              {t('no_jobs')}
            </div>
          )}
        </div>
      </div>
      {/* Notification banner */}
      {notification && (
        <div className={`fixed bottom-6 right-6 z-50 flex items-center gap-2 px-4 py-3 rounded-lg shadow-lg text-xs font-mono border ${
          notification.type === 'error'
            ? 'bg-red-500/10 border-red-500/30 text-red-400'
            : 'bg-green-500/10 border-green-500/30 text-green-400'
        } animate-in fade-in slide-in-from-bottom-2 duration-300`}>
          {notification.type === 'error' && <AlertTriangle size={14} />}
          <span>{notification.message}</span>
          <button onClick={() => setNotification(null)} className="ml-2 opacity-60 hover:opacity-100">&times;</button>
        </div>
      )}

      {/* Confirm dialog */}
      {confirmDialog && (
        <Modal title={tc('confirm')} onClose={() => setConfirmDialog(null)}>
          <p className="text-sm text-content-secondary mb-4">{confirmDialog.message}</p>
          <div className="flex justify-end gap-2">
            <button
              onClick={() => setConfirmDialog(null)}
              className="px-3 py-1.5 text-xs font-bold rounded bg-surface-secondary border border-edge text-content-secondary hover:bg-surface-secondary/80"
            >
              {tc('cancel')}
            </button>
            <button
              onClick={confirmDialog.onConfirm}
              className="px-3 py-1.5 text-xs font-bold rounded bg-red-500 text-white hover:bg-red-600"
            >
              {tc('confirm')}
            </button>
          </div>
        </Modal>
      )}
    </div>
  );
});
