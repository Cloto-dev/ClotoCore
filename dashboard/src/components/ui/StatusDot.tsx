export type StatusDotStatus = 'online' | 'connected' | 'degraded' | 'error' | 'offline';

interface StatusDotProps {
  status: StatusDotStatus;
  size?: 'sm' | 'md';
  pulse?: boolean;
  className?: string;
}

const colorMap: Record<StatusDotStatus, string> = {
  online: 'bg-emerald-500',
  connected: 'bg-emerald-500',
  degraded: 'bg-amber-500',
  error: 'bg-red-500',
  offline: 'bg-content-muted',
};

const defaultPulse: Record<StatusDotStatus, boolean> = {
  online: false,
  connected: false,
  degraded: true,
  error: true,
  offline: false,
};

export function StatusDot({ status, size = 'md', pulse, className }: StatusDotProps) {
  const sizeClass = size === 'sm' ? 'w-1.5 h-1.5' : 'w-2 h-2';
  const shouldPulse = pulse ?? defaultPulse[status];

  return (
    <span
      className={`${sizeClass} rounded-full inline-block ${colorMap[status]} ${shouldPulse ? 'animate-pulse' : ''} ${className ?? ''}`}
    />
  );
}
