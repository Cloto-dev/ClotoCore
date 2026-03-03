import { Pencil } from 'lucide-react';

interface Props {
  name: string;
  description: string;
  onNameChange: (name: string) => void;
  onDescriptionChange: (description: string) => void;
}

export function ProfileSection({ name, description, onNameChange, onDescriptionChange }: Props) {
  return (
    <section>
      <div className="flex items-center gap-3 mb-3 border-b border-edge pb-2">
        <Pencil className="text-brand" size={16} />
        <h2 className="font-bold text-xs text-content-secondary uppercase tracking-widest">Profile</h2>
      </div>
      <div className="space-y-3">
        <div>
          <label className="block text-[10px] font-bold text-content-tertiary uppercase tracking-wider mb-1">Name</label>
          <input
            type="text"
            value={name}
            onChange={e => onNameChange(e.target.value)}
            maxLength={200}
            className="w-full px-3 py-2 rounded-lg border border-edge text-xs focus:outline-none focus:border-brand bg-surface-primary font-mono"
          />
        </div>
        <div>
          <label className="block text-[10px] font-bold text-content-tertiary uppercase tracking-wider mb-1">Description</label>
          <textarea
            value={description}
            onChange={e => onDescriptionChange(e.target.value)}
            maxLength={1000}
            className="w-full px-3 py-2 rounded-lg border border-edge text-xs focus:outline-none focus:border-brand bg-surface-primary font-mono h-20 resize-none"
          />
        </div>
      </div>
    </section>
  );
}
