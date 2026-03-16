import { ImageIcon } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Modal } from './Modal';

interface VrmThumbnailDialogProps {
  open: boolean;
  thumbnailUrl: string;
  onApply: () => void;
  onSkip: () => void;
}

const SESSION_KEY = 'cloto-vrm-thumbnail-skip';

export function VrmThumbnailDialog({ open, thumbnailUrl, onApply, onSkip }: VrmThumbnailDialogProps) {
  const { t } = useTranslation('agents');
  const [dontShowAgain, setDontShowAgain] = useState(false);

  if (!open) return null;

  const handleAction = (apply: boolean) => {
    if (dontShowAgain) {
      sessionStorage.setItem(SESSION_KEY, '1');
    }
    if (apply) onApply();
    else onSkip();
  };

  return (
    <Modal title={t('plugin_workspace.vrm_thumbnail_title')} icon={ImageIcon} onClose={() => handleAction(false)}>
      <div className="p-4 space-y-4">
        <p className="text-[13px] text-content-secondary">
          {t('plugin_workspace.vrm_thumbnail_message')}
        </p>

        <div className="flex justify-center">
          <img
            src={thumbnailUrl}
            alt="VRM thumbnail"
            className="w-24 h-24 rounded-lg border border-edge object-cover bg-surface-secondary"
          />
        </div>

        <label className="flex items-center gap-2 cursor-pointer select-none">
          <input
            type="checkbox"
            checked={dontShowAgain}
            onChange={(e) => setDontShowAgain(e.target.checked)}
            className="rounded border-edge"
          />
          <span className="text-[11px] font-mono text-content-tertiary">{t('plugin_workspace.vrm_thumbnail_dont_show')}</span>
        </label>

        <div className="flex justify-end gap-2">
          <button
            onClick={() => handleAction(false)}
            className="px-3 py-1.5 rounded text-[11px] font-mono uppercase tracking-widest text-content-tertiary hover:text-content-secondary hover:bg-glass transition-colors"
          >
            {t('plugin_workspace.vrm_thumbnail_skip')}
          </button>
          <button
            onClick={() => handleAction(true)}
            className="px-3 py-1.5 rounded text-[11px] font-mono uppercase tracking-widest bg-brand/20 text-brand hover:bg-brand/30 transition-colors"
          >
            {t('plugin_workspace.vrm_thumbnail_apply')}
          </button>
        </div>
      </div>
    </Modal>
  );
}
