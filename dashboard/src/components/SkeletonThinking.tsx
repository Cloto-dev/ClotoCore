import { type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

interface SkeletonThinkingProps {
  agentColor: string;
  agentIcon: ReactNode;
}

export function SkeletonThinking({ agentColor, agentIcon }: SkeletonThinkingProps) {
  const { t } = useTranslation();
  return (
    <div className="flex items-start gap-3 message-enter">
      <div
        className="w-8 h-8 rounded-lg text-white flex items-center justify-center shrink-0 shadow-sm"
        style={{ backgroundColor: agentColor }}
      >
        {agentIcon}
      </div>
      <div className="pt-2 text-sm text-content-tertiary font-mono animate-pulse">
        {t('skeleton_thinking')}
      </div>
    </div>
  );
}
