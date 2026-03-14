import type { LucideIcon } from 'lucide-react';

interface SectionHeaderProps {
  icon: LucideIcon;
  title: string;
  trailing?: React.ReactNode;
  className?: string;
}

export function SectionHeader({ icon: Icon, title, trailing, className }: SectionHeaderProps) {
  return (
    <div className={`flex items-center gap-3 mb-3 border-b border-edge pb-2 ${className ?? ''}`}>
      <Icon className="text-brand" size={16} />
      <h2 className="font-bold text-xs text-content-secondary uppercase tracking-widest">{title}</h2>
      {trailing && <div className="ml-auto">{trailing}</div>}
    </div>
  );
}
