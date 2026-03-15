import type React from 'react';

interface AlertCardProps {
  variant?: 'error' | 'warning' | 'info';
  children: React.ReactNode;
  className?: string;
}

const variantClasses: Record<string, string> = {
  error: 'bg-red-500/10 border-red-500/20 text-red-400',
  warning: 'bg-amber-500/10 border-amber-500/20 text-amber-400',
  info: 'bg-brand/10 border-brand/20 text-brand',
};

export function AlertCard({ variant = 'error', children, className }: AlertCardProps) {
  return (
    <div className={`p-2 text-[10px] font-mono rounded-lg border ${variantClasses[variant]} ${className ?? ''}`}>
      {children}
    </div>
  );
}
