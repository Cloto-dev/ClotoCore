import type React from 'react';

interface AlertCardProps {
  variant?: 'error' | 'warning' | 'info';
  children: React.ReactNode;
  className?: string;
}

const variantClasses: Record<string, string> = {
  error: 'bg-red-500/10 border-red-500/20 text-red-400',
  warning: 'bg-amber-500/10 border-amber-500/20 text-amber-400',
  info: 'bg-agent/10 border-agent/20 text-agent',
};

export function AlertCard({ variant = 'error', children, className }: AlertCardProps) {
  return (
    <div className={`p-2 text-xs font-mono rounded-lg border ${variantClasses[variant]} ${className ?? ''}`}>
      {children}
    </div>
  );
}
