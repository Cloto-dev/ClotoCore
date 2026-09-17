import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useSearchParams } from 'react-router-dom';
import { Modal } from '../components/Modal';
import { MarketplaceTab } from '../components/mcp/MarketplaceTab';
import { McpServerDetail } from '../components/mcp/McpServerDetail';
import { AlertCard } from '../components/ui/AlertCard';
import '../components/Workshop.css';
import { useApi } from '../hooks/useApi';
import { useAsyncAction } from '../hooks/useAsyncAction';
import { useMarketplace } from '../hooks/useMarketplace';
import { useMcpServers } from '../hooks/useMcpServers';
import { extractError } from '../lib/errors';
import {
  type Deviation,
  describe,
  deviation,
  groupServers,
  matchesQuery,
  nameOf,
  needsAttention,
  type ServerGroup,
  type ServerRow,
} from '../lib/mcpGroups';

type Tab = 'installed' | 'marketplace' | 'updates';

/**
 * The workshop's MCP page (docs/gui/samples/04-mcp-servers.html): one column,
 * a band per kind, a role beside every name, and a state only when it deviates.
 */
export function McpServersPage() {
  const api = useApi();
  const { t } = useTranslation('mcp');
  const { t: tc } = useTranslation('common');
  const { servers, isLoading, error: fetchError, refetch } = useMcpServers();
  // The catalog names each installed server's kind and description when the
  // store has none; the list is drawn without it while it loads or if it fails.
  const { servers: catalog } = useMarketplace();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  // ?server=<id> opens that server (search links here). The parameter is
  // taken and removed, so Back does not reopen it and a later link still works.
  const [searchParams, setSearchParams] = useSearchParams();
  const serverParam = searchParams.get('server');
  useEffect(() => {
    if (!serverParam) return;
    setSelectedId(serverParam);
    const next = new URLSearchParams(searchParams);
    next.delete('server');
    setSearchParams(next, { replace: true });
  }, [serverParam, searchParams, setSearchParams]);
  const [addModalOpen, setAddModalOpen] = useState(false);
  const [activeTab, setActiveTab] = useState<Tab>('installed');
  const [query, setQuery] = useState('');
  const marketplaceRefetchRef = useRef<(() => Promise<void>) | null>(null);

  // Add server form state
  const [newName, setNewName] = useState('');
  const [newCommand, setNewCommand] = useState('python3');
  const [newArgs, setNewArgs] = useState('');
  const [addError, setAddError] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const action = useAsyncAction();

  const isValidServerName = (name: string) => /^[a-z][a-z0-9._-]{0,62}[a-z0-9]$/.test(name);

  const selectedServer = servers.find((s) => s.id === selectedId);
  const groups = useMemo(() => groupServers(servers, catalog), [servers, catalog]);
  const rows = useMemo(() => groups.flatMap((g) => g.rows), [groups]);
  const updates = useMemo(() => rows.filter((r) => deviation(r.server, r.entry)?.kind === 'update'), [rows]);
  const running = servers.filter((s) => s.status === 'Connected').length;
  const failing = servers.filter((s) => s.status === 'Error').length;

  const handleDelete = useCallback(
    (id: string) =>
      action.run(async () => {
        await api.deleteMcpServer(id);
        if (selectedId === id) setSelectedId(null);
        refetch();
        return true; // bug-471: signal success so the caller can gate its checkmark
      }),
    [api, selectedId, refetch, action.run],
  );

  const handleStart = useCallback(
    (id: string) =>
      action.run(async () => {
        await api.startMcpServer(id);
        setTimeout(refetch, 500);
        return true; // bug-471: signal success so the caller can gate its checkmark
      }),
    [api, refetch, action.run],
  );

  const handleStop = useCallback(
    (id: string) =>
      action.run(async () => {
        await api.stopMcpServer(id);
        setTimeout(refetch, 500);
        return true; // bug-471: signal success so the caller can gate its checkmark
      }),
    [api, refetch, action.run],
  );

  const handleRestart = useCallback(
    (id: string) =>
      action.run(async () => {
        await api.restartMcpServer(id);
        setTimeout(refetch, 500);
        return true; // bug-471: signal success so the caller can gate its checkmark
      }),
    [api, refetch, action.run],
  );

  async function handleAdd() {
    if (!newName.trim()) return;
    setAdding(true);
    setAddError(null);
    try {
      const args = newArgs.trim() ? newArgs.split(/\s+/) : [];
      await api.createMcpServer({ name: newName.trim(), command: newCommand, args });
      setAddModalOpen(false);
      setNewName('');
      setNewArgs('');
      refetch();
    } catch (err) {
      setAddError(extractError(err, 'Failed to add server'));
    } finally {
      setAdding(false);
    }
  }

  if (selectedServer) {
    return (
      <McpServerDetail
        server={selectedServer}
        entry={rows.find((r) => r.server.id === selectedServer.id)?.entry}
        onBack={() => setSelectedId(null)}
        onRefresh={refetch}
        onDelete={handleDelete}
        onStart={handleStart}
        onStop={handleStop}
        onRestart={handleRestart}
      />
    );
  }

  const devText = (d: Deviation) => {
    if (!d) return null;
    switch (d.kind) {
      case 'failing':
        return d.message ? t('deviation.failing_with', { message: d.message }) : t('deviation.failing');
      case 'env_unresolved':
        return t('deviation.env_unresolved');
      case 'connecting':
        return t('deviation.connecting');
      case 'off':
        return t('deviation.off');
      case 'update':
        return t('deviation.update', { version: d.version });
    }
  };
  const devClass = (d: Deviation) =>
    !d ? '' : d.kind === 'failing' || d.kind === 'env_unresolved' ? 'dev' : d.kind === 'off' ? 'dev off' : 'dev warn';

  const drawRow = ({ server, entry }: ServerRow) => {
    const d = deviation(server, entry);
    const what = describe(server, entry);
    return (
      <button
        type="button"
        key={server.id}
        className="srv"
        onClick={() => setSelectedId(server.id)}
        aria-label={nameOf(server)}
      >
        <span className="nm">
          {nameOf(server)}
          <small>{server.id}</small>
        </span>
        <span className="what">
          {d && <span className={devClass(d)}>{devText(d)} </span>}
          {what}
        </span>
        <span className="cnt num">{t('tools_count', { count: server.tools.length })}</span>
        <span className="more" title={t('list.more')}>
          <svg viewBox="0 0 24 24" fill="currentColor" style={{ width: 14, height: 14 }}>
            <circle cx="5" cy="12" r="1.6" />
            <circle cx="12" cy="12" r="1.6" />
            <circle cx="19" cy="12" r="1.6" />
          </svg>
        </span>
      </button>
    );
  };

  const bandLabel = (group: ServerGroup) => t(`groups.${group}`);

  const visibleGroups = groups
    .map((g) => ({ ...g, rows: g.rows.filter((r) => matchesQuery(r, query)) }))
    .filter((g) => g.rows.length > 0);

  return (
    <div className="ws">
      <div className="ws-head">
        <h1>{t('title')}</h1>
        <span className="count">
          {t('list.summary', { count: servers.length, running })}
          {failing > 0 && (
            <>
              {t('list.sep')}
              <span className="bad">{t('list.failing', { count: failing })}</span>
            </>
          )}
        </span>
        <span className="spacer" />
        {activeTab === 'installed' && (
          <label className="find">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round">
              <circle cx="11" cy="11" r="7" />
              <path d="m20 20-3.5-3.5" />
            </svg>
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder={t('list.search')}
              aria-label={t('list.search')}
            />
          </label>
        )}
        <button type="button" className="btn pri" onClick={() => setAddModalOpen(true)} aria-label={t('add_server')}>
          {t('add_server')}
        </button>
      </div>

      <div className="tabs" role="tablist">
        <button
          type="button"
          role="tab"
          aria-selected={activeTab === 'installed'}
          className={activeTab === 'installed' ? 'on' : ''}
          onClick={() => setActiveTab('installed')}
        >
          {t('list.tab_installed')}
          <span className="n num">{servers.length}</span>
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={activeTab === 'marketplace'}
          className={activeTab === 'marketplace' ? 'on' : ''}
          onClick={() => setActiveTab('marketplace')}
        >
          {t('marketplace.tab_marketplace')}
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={activeTab === 'updates'}
          className={activeTab === 'updates' ? 'on' : ''}
          onClick={() => setActiveTab('updates')}
        >
          {t('list.tab_updates')}
          <span className="n num">{updates.length}</span>
        </button>
      </div>

      {action.error && <div className="problem">{action.error}</div>}
      {fetchError && <div className="problem">{t('backend_unreachable')}</div>}

      <div className="ws-body">
        {activeTab === 'installed' && (
          <div className="lst">
            {visibleGroups.map((g) => (
              <div key={g.group}>
                <div className={`band${g.group === 'attention' ? ' bad' : ''}`}>
                  <span className="lbl">{bandLabel(g.group)}</span>
                  <span className="n num">{g.rows.length}</span>
                </div>
                {g.rows.map(drawRow)}
              </div>
            ))}
            {servers.length === 0 && !isLoading && !fetchError && (
              <div className="empty">{t('no_servers_configured')}</div>
            )}
            {servers.length > 0 && visibleGroups.length === 0 && <div className="empty">{t('list.no_match')}</div>}
            {servers.length > 0 && <p className="note">{t('list.note')}</p>}
          </div>
        )}

        {activeTab === 'updates' && (
          <div className="lst">
            {updates.length === 0 ? (
              <div className="empty">{t('list.no_updates')}</div>
            ) : (
              <>
                <div className="band">
                  <span className="lbl">{t('list.tab_updates')}</span>
                  <span className="n num">{updates.length}</span>
                </div>
                {updates.map(drawRow)}
                <p className="note">{t('list.updates_note')}</p>
              </>
            )}
          </div>
        )}

        {activeTab === 'marketplace' && (
          <div className="p-5">
            <MarketplaceTab onRefetchRef={marketplaceRefetchRef} />
          </div>
        )}
      </div>

      {/* Add Server Modal */}
      {addModalOpen && (
        <Modal
          title={t('add_modal.title')}
          size="sm"
          onClose={() => {
            setAddModalOpen(false);
            setAddError(null);
          }}
        >
          <div className="px-5 py-4 space-y-3 ws">
            {addError && <AlertCard>{addError}</AlertCard>}

            <div>
              <label className="block text-xs text-content-tertiary mb-1">{t('add_modal.server_name')}</label>
              <input
                type="text"
                value={newName}
                onChange={(e) => setNewName(e.target.value)}
                placeholder="my-server"
                className="in mono"
              />
              <p className="hint">{t('add_modal.name_hint')}</p>
            </div>
            <div>
              <label className="block text-xs text-content-tertiary mb-1">{t('add_modal.command')}</label>
              <input
                type="text"
                value={newCommand}
                onChange={(e) => setNewCommand(e.target.value)}
                placeholder="python3"
                className="in mono"
              />
            </div>
            <div>
              <label className="block text-xs text-content-tertiary mb-1">{t('add_modal.arguments')}</label>
              <input
                type="text"
                value={newArgs}
                onChange={(e) => setNewArgs(e.target.value)}
                placeholder="scripts/my_server.py"
                className="in mono"
              />
            </div>

            <div className="flex justify-end gap-2 pt-1">
              <button
                type="button"
                className="btn"
                onClick={() => {
                  setAddModalOpen(false);
                  setAddError(null);
                }}
                aria-label={tc('cancel')}
              >
                {tc('cancel')}
              </button>
              <button
                type="button"
                className="btn pri"
                onClick={handleAdd}
                disabled={adding || !isValidServerName(newName.trim())}
                aria-label={t('add_modal.add')}
              >
                {adding ? t('add_modal.adding') : t('add_modal.add')}
              </button>
            </div>
          </div>
        </Modal>
      )}
    </div>
  );
}

// Re-exported for the detail page's header, which writes the same state words.
export { needsAttention };
