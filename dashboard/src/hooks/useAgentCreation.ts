import { useState } from 'react';
import { useApi } from './useApi';

export interface RoutingRuleEntry {
  match: string;
  engine: string;
  cfr?: boolean;
  escalate_to?: string;
  fallback?: string;
}

interface CreationForm {
  name: string;
  desc: string;
  engine: string;
  memory: string;
  password: string;
  routingRules: RoutingRuleEntry[];
  /** The face, and the 3D model: uploaded to the agent once it exists. */
  avatarFile: File | null;
  vrmFile: File | null;
}

/**
 * What a caller is told when the agent exists. `faceProblem` is set when the
 * agent was made but its picture or model could not be saved: that is not a
 * failed creation, and must not be shown as one — trying again would ask the
 * kernel for a second agent of the same name.
 */
export interface CreatedAgent {
  name: string;
  id: string | null;
  faceProblem: string | null;
}

const INITIAL_FORM: CreationForm = {
  name: '',
  desc: '',
  engine: '',
  memory: '',
  password: '',
  routingRules: [],
  avatarFile: null,
  vrmFile: null,
};

export function useAgentCreation(onCreated: (created: CreatedAgent) => void) {
  const api = useApi();
  const [form, setForm] = useState<CreationForm>(INITIAL_FORM);
  const [isCreating, setIsCreating] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);

  const updateField = <K extends keyof CreationForm>(key: K, value: CreationForm[K]) => {
    setCreateError(null);
    setForm((prev) => ({ ...prev, [key]: value }));
  };

  const handleCreate = async () => {
    setIsCreating(true);
    setCreateError(null);
    try {
      const metadata: Record<string, string> = {
        preferred_memory: form.memory,
        agent_type: 'ai',
      };
      if (form.routingRules.length > 0) {
        metadata.engine_routing = JSON.stringify(form.routingRules);
      }
      const { id } = await api.createAgent({
        name: form.name,
        description: form.desc,
        default_engine: form.engine,
        metadata,
        password: form.password || undefined,
      });
      const { name, avatarFile, vrmFile } = form;
      // From here on the agent exists. Whatever happens to its face is
      // reported beside the success, never in place of it.
      let faceProblem: string | null = null;
      if (avatarFile || vrmFile) {
        try {
          if (!id) throw new Error('the kernel did not say which agent it made');
          // The same order the settings page saves them in.
          if (avatarFile) await api.uploadAvatar(id, avatarFile);
          if (vrmFile) await api.uploadVrm(id, vrmFile);
        } catch (e) {
          faceProblem = e instanceof Error ? e.message : 'Unknown error';
          if (import.meta.env.DEV) console.error(e);
        }
      }
      setForm(INITIAL_FORM);
      onCreated({ name, id, faceProblem });
    } catch (e) {
      const msg = e instanceof Error ? e.message : 'Unknown error';
      setCreateError(msg);
      if (import.meta.env.DEV) console.error(e);
    } finally {
      setIsCreating(false);
    }
  };

  const addRoutingRule = () => {
    setForm((prev) => ({
      ...prev,
      routingRules: [...prev.routingRules, { match: 'default', engine: '', cfr: true }],
    }));
  };

  const updateRoutingRule = (index: number, field: keyof RoutingRuleEntry, value: string | boolean | undefined) => {
    setForm((prev) => {
      const rules = [...prev.routingRules];
      rules[index] = { ...rules[index], [field]: value };
      return { ...prev, routingRules: rules };
    });
  };

  const removeRoutingRule = (index: number) => {
    setForm((prev) => ({
      ...prev,
      routingRules: prev.routingRules.filter((_, i) => i !== index),
    }));
  };

  return {
    form,
    updateField,
    handleCreate,
    isCreating,
    createError,
    addRoutingRule,
    updateRoutingRule,
    removeRoutingRule,
  };
}
