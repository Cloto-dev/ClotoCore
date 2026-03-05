import { ReactNode } from 'react';

interface SystemAlertCardProps {
  icon: ReactNode;
  title: string;
  trailing?: ReactNode;
  footer?: ReactNode;
  children: ReactNode;
}

/**
 * Unified amber-styled alert card used for system notifications
 * (Engine Error, Command Approval, etc.).
 *
 * Layout: [icon]  [card with header + body + optional footer]
 */
export function SystemAlertCard({ icon, title, trailing, footer, children }: SystemAlertCardProps) {
  return (
    <div className="flex items-start gap-3">
      <div className="w-8 h-8 rounded-lg flex items-center justify-center shrink-0 shadow-sm bg-amber-500/20 text-amber-500">
        {icon}
      </div>
      <div className="p-4 rounded-xl border border-amber-500/20 bg-amber-500/5 text-sm max-w-[80%] space-y-3 animate-in fade-in slide-in-from-bottom-2 duration-300">
        <div className="flex items-center justify-between">
          <span className="text-[10px] font-bold text-amber-500 uppercase tracking-widest">{title}</span>
          {trailing}
        </div>
        {children}
        {footer && <div className="pt-1">{footer}</div>}
      </div>
    </div>
  );
}
