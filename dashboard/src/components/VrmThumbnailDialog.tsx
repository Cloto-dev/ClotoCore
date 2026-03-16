import { ImageIcon } from 'lucide-react';
import { useState } from 'react';
import { Modal } from './Modal';

interface VrmThumbnailDialogProps {
  open: boolean;
  thumbnailUrl: string;
  onApply: () => void;
  onSkip: () => void;
}

const SESSION_KEY = 'cloto-vrm-thumbnail-skip';

export function VrmThumbnailDialog({ open, thumbnailUrl, onApply, onSkip }: VrmThumbnailDialogProps) {
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
    <Modal title="VRM Thumbnail Detected" icon={ImageIcon} onClose={() => handleAction(false)}>
      <div className="p-4 space-y-4">
        <p className="text-[13px] text-content-secondary">
          VRM file contains a thumbnail image. Apply it as the agent avatar?
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
          <span className="text-[11px] font-mono text-content-tertiary">Don't show again this session</span>
        </label>

        <div className="flex justify-end gap-2">
          <button
            onClick={() => handleAction(false)}
            className="px-3 py-1.5 rounded text-[11px] font-mono uppercase tracking-widest text-content-tertiary hover:text-content-secondary hover:bg-glass transition-colors"
          >
            Skip
          </button>
          <button
            onClick={() => handleAction(true)}
            className="px-3 py-1.5 rounded text-[11px] font-mono uppercase tracking-widest bg-brand/20 text-brand hover:bg-brand/30 transition-colors"
          >
            Apply
          </button>
        </div>
      </div>
    </Modal>
  );
}
