import { AlertTriangle } from 'lucide-react';
import { Modal } from '../Modal';

interface ConfirmDialogProps {
  open: boolean;
  title: string;
  message: string;
  confirmLabel?: string;
  cancelLabel?: string;
  variant?: 'danger' | 'default';
  onConfirm: () => void;
  onCancel: () => void;
}

export function ConfirmDialog({
  open,
  title,
  message,
  confirmLabel = 'Confirm',
  cancelLabel = 'Cancel',
  variant = 'default',
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  if (!open) return null;

  const isDanger = variant === 'danger';

  return (
    <Modal title={title} icon={isDanger ? AlertTriangle : undefined} onClose={onCancel}>
      <div className="p-4 space-y-4">
        <p className="text-xs text-content-secondary">{message}</p>
        <div className="flex justify-end gap-2">
          <button
            onClick={onCancel}
            className="px-3 py-1.5 rounded text-xs font-mono text-content-tertiary hover:text-content-secondary hover:bg-surface-panel transition-colors"
          >
            {cancelLabel}
          </button>
          <button
            onClick={onConfirm}
            className={`px-3 py-1.5 rounded text-xs font-mono transition-colors ${
              isDanger ? 'bg-red-500/20 text-red-400 hover:bg-red-500/30' : 'bg-agent/20 text-agent hover:bg-agent/30'
            }`}
          >
            {confirmLabel}
          </button>
        </div>
      </div>
    </Modal>
  );
}
