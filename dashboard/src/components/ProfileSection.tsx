import { Pencil } from 'lucide-react';
import { useTranslation } from 'react-i18next';

interface Props {
  name: string;
  description: string;
  onNameChange: (name: string) => void;
  onDescriptionChange: (description: string) => void;
}

export function ProfileSection({ name, description, onNameChange, onDescriptionChange }: Props) {
  const { t } = useTranslation('agents');
  return (
    <section>
      <div className="flex items-center gap-3 mb-3 border-b border-edge pb-2">
        <Pencil className="text-brand" size={16} />
        <h2 className="font-bold text-xs text-content-secondary uppercase tracking-widest">
          {t('plugin_workspace.profile')}
        </h2>
      </div>
      <div className="space-y-3">
        <div>
          <label className="block text-[10px] font-bold text-content-tertiary uppercase tracking-wider mb-1">
            {t('plugin_workspace.profile_name')}
          </label>
          <input
            type="text"
            value={name}
            onChange={(e) => onNameChange(e.target.value)}
            maxLength={200}
            className="w-full px-3 py-2 rounded-lg border border-edge text-xs focus:outline-none focus:border-brand bg-surface-primary font-mono"
          />
        </div>
        <div>
          <label className="block text-[10px] font-bold text-content-tertiary uppercase tracking-wider mb-1">
            {t('plugin_workspace.profile_description')}
          </label>
          <textarea
            value={description}
            onChange={(e) => onDescriptionChange(e.target.value)}
            maxLength={1000}
            className="w-full px-3 py-2 rounded-lg border border-edge text-xs focus:outline-none focus:border-brand bg-surface-primary font-mono h-20 resize-none"
          />
        </div>
      </div>
    </section>
  );
}
