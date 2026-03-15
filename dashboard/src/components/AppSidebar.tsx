import { Brain, Clock, Cpu, PanelLeftClose, PanelLeftOpen, Server, Settings, Users } from 'lucide-react';
import type React from 'react';
import { useTranslation } from 'react-i18next';
import { useLocation, useNavigate } from 'react-router-dom';
import { useAgentContext } from '../contexts/AgentContext';
import { AgentIcon, statusDotColor } from '../lib/agentIdentity';

const NAV_LINKS: readonly { path: string; icon: typeof Server; labelKey: string; action?: 'settings' | 'agents' }[] = [
  { path: '/', icon: Users, labelKey: 'agent', action: 'agents' },
  { path: '/mcp-servers', icon: Server, labelKey: 'mcp' },
  { path: '/cron', icon: Clock, labelKey: 'cron' },
  { path: '/dashboard', icon: Brain, labelKey: 'memory' },
  { path: '#settings', icon: Settings, labelKey: 'settings', action: 'settings' },
] as const;

interface AppSidebarProps {
  onSettingsClick: () => void;
  collapsed: boolean;
  onToggleCollapse: () => void;
}

export const AppSidebar: React.FC<AppSidebarProps> = ({ onSettingsClick, collapsed, onToggleCollapse }) => {
  const navigate = useNavigate();
  const location = useLocation();
  const { t } = useTranslation('nav');
  const { agents, selectedAgentId, setSelectedAgentId, systemActive, setSystemActive } = useAgentContext();

  const handleSelectSystem = () => {
    if (systemActive && isAgentPageActive) {
      navigate('/');
    } else {
      navigate('/?system=true');
    }
  };

  const handleSelectAgent = (id: string) => {
    navigate('/?agent=' + encodeURIComponent(id));
  };

  const handleAddAgent = () => {
    navigate('/');
  };

  const handleNavClick = (path: string) => {
    setSelectedAgentId(null);
    setSystemActive(false);
    navigate(path);
  };

  const isNavActive = (path: string) => location.pathname === path;
  const isAgentPageActive = location.pathname === '/';

  return (
    <div
      className={`${collapsed ? 'w-14' : 'w-48'} h-full flex flex-col py-3 bg-surface-secondary/60 backdrop-blur-md border-r border-[var(--border-strong)] transition-[width] duration-200`}
    >
      {/* System / Kernel */}
      <button
        onClick={handleSelectSystem}
        title={collapsed ? t('system') : undefined}
        className={`relative mx-2 flex items-center ${collapsed ? 'justify-center px-0' : 'gap-2.5 px-3'} py-2 rounded-lg transition-all duration-200 ${
          systemActive && isAgentPageActive
            ? 'bg-surface-primary shadow-sm text-brand ring-1 ring-brand/20'
            : 'text-content-tertiary hover:text-content-secondary hover:bg-glass-strong'
        }`}
      >
        {!collapsed && systemActive && isAgentPageActive && (
          <div className="absolute left-0 top-1/2 -translate-y-1/2 w-1 h-5 bg-brand rounded-r-full" />
        )}
        <Cpu size={24} className="shrink-0" />
        {!collapsed && <span className="text-[11px] font-bold tracking-wide uppercase">{t('system')}</span>}
      </button>

      <div className={`${collapsed ? 'mx-2' : 'mx-3'} my-2 h-px bg-edge`} />

      {/* Agents List */}
      <div className="flex-1 flex flex-col gap-1 overflow-y-auto no-scrollbar px-2">
        {agents.map((agent) => {
          const isActive = selectedAgentId === agent.id && !systemActive && isAgentPageActive;
          return (
            <button
              key={agent.id}
              onClick={() => handleSelectAgent(agent.id)}
              title={collapsed ? agent.name : undefined}
              className={`relative flex items-center ${collapsed ? 'justify-center px-0' : 'gap-2.5 px-3'} py-2 rounded-lg transition-all duration-200 text-left w-full ${
                isActive
                  ? 'bg-surface-primary text-brand shadow-sm ring-1 ring-brand/20'
                  : 'text-content-tertiary hover:text-content-secondary hover:bg-glass-strong'
              }`}
            >
              {!collapsed && isActive && (
                <div className="absolute left-0 top-1/2 -translate-y-1/2 w-1 h-5 bg-brand rounded-r-full" />
              )}
              <div className="relative flex-shrink-0 w-7 h-7 overflow-hidden rounded-md flex items-center justify-center">
                <AgentIcon agent={agent} size={28} />
                <div
                  className={`absolute -bottom-0.5 -right-0.5 w-2.5 h-2.5 rounded-full border-2 border-surface-secondary ${statusDotColor(agent.status)}`}
                />
              </div>
              {!collapsed && (
                <div className="min-w-0 flex-1">
                  <div className="text-xs font-bold truncate">{agent.name}</div>
                </div>
              )}
            </button>
          );
        })}
      </div>

      <div className={`${collapsed ? 'mx-2' : 'mx-3'} my-2 h-px bg-edge`} />

      {/* Nav Links */}
      <div className="px-2 flex flex-col gap-0.5">
        {NAV_LINKS.map(({ path, icon: Icon, labelKey, action }) => {
          const label = t(labelKey);
          const isActive =
            action === 'agents' ? isAgentPageActive && !selectedAgentId && !systemActive : !action && isNavActive(path);
          return (
            <button
              key={path}
              onClick={() => {
                if (action === 'settings') onSettingsClick();
                else if (action === 'agents') handleAddAgent();
                else handleNavClick(path);
              }}
              title={collapsed ? label : undefined}
              className={`relative flex items-center ${collapsed ? 'justify-center px-0' : 'gap-2.5 px-3'} py-2 rounded-lg transition-all duration-200 text-xs font-bold uppercase tracking-wide ${
                isActive
                  ? 'bg-surface-primary text-brand shadow-sm ring-1 ring-brand/20'
                  : 'text-content-tertiary hover:text-content-secondary hover:bg-glass-strong'
              }`}
            >
              {!collapsed && isActive && (
                <div className="absolute left-0 top-1/2 -translate-y-1/2 w-1 h-4 bg-brand rounded-r-full" />
              )}
              <Icon size={24} className="shrink-0" />
              {!collapsed && label}
            </button>
          );
        })}
      </div>

      <div className={`${collapsed ? 'mx-2' : 'mx-3'} my-2 h-px bg-edge`} />

      {/* Collapse toggle */}
      <div className="px-2">
        <button
          onClick={onToggleCollapse}
          title={collapsed ? t('expand_sidebar') : t('collapse_sidebar')}
          className={`flex items-center ${collapsed ? 'justify-center px-0' : 'gap-2.5 px-3'} py-2 rounded-lg transition-all duration-200 text-content-tertiary hover:text-content-secondary hover:bg-glass-strong w-full`}
        >
          {collapsed ? (
            <PanelLeftOpen size={24} className="shrink-0" />
          ) : (
            <PanelLeftClose size={24} className="shrink-0" />
          )}
          {!collapsed && <span className="text-[10px] font-mono uppercase tracking-wide">{t('collapse')}</span>}
        </button>
      </div>
    </div>
  );
};
