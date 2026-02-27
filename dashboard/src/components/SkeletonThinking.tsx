import { type ReactNode } from 'react';

interface SkeletonThinkingProps {
  agentColor: string;
  agentIcon: ReactNode;
}

export function SkeletonThinking({ agentColor, agentIcon }: SkeletonThinkingProps) {
  return (
    <div className="flex items-start gap-3 message-enter">
      <div
        className="w-8 h-8 rounded-lg text-white flex items-center justify-center shrink-0 shadow-sm"
        style={{ backgroundColor: agentColor }}
      >
        {agentIcon}
      </div>
      <div className="max-w-[80%] p-4 rounded-2xl rounded-tl-none bg-surface-secondary space-y-2">
        <div className="shimmer-line h-3 rounded-full w-full" />
        <div className="shimmer-line h-3 rounded-full w-4/5" />
        <div className="shimmer-line h-3 rounded-full w-3/5" />
      </div>
    </div>
  );
}
