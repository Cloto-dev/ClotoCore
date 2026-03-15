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
}

const INITIAL_FORM: CreationForm = {
  name: '',
  desc: '',
  engine: '',
  memory: '',
  password: '',
  routingRules: [],
};

export function useAgentCreation(onCreated: () => void) {
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
      await api.createAgent({
        name: form.name,
        description: form.desc,
        default_engine: form.engine,
        metadata,
        password: form.password || undefined,
      });
      setForm(INITIAL_FORM);
      onCreated();
    } catch (e) {
      const msg = e instanceof Error ? e.message : 'Unknown error';
      setCreateError(msg);
      console.error(e);
    } finally {
      setIsCreating(false);
    }
  };

  const addRoutingRule = () => {
    setForm((prev) => ({
      ...prev,
      routingRules: [...prev.routingRules, { match: 'default', engine: '' }],
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
