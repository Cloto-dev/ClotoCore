import { useState } from 'react';
import { api } from '../services/api';
import { AgentType } from '../lib/agentIdentity';
import { useApiKey } from '../contexts/ApiKeyContext';

export interface RoutingRuleEntry {
  match: string;
  engine: string;
}

interface CreationForm {
  name: string;
  desc: string;
  engine: string;
  memory: string;
  type: AgentType;
  password: string;
  routingRules: RoutingRuleEntry[];
}

const INITIAL_FORM: CreationForm = {
  name: '',
  desc: '',
  engine: '',
  memory: '',
  type: 'ai',
  password: '',
  routingRules: [],
};

export function useAgentCreation(onCreated: () => void) {
  const { apiKey } = useApiKey();
  const [form, setForm] = useState<CreationForm>(INITIAL_FORM);
  const [isCreating, setIsCreating] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);

  const updateField = <K extends keyof CreationForm>(key: K, value: CreationForm[K]) => {
    setCreateError(null);
    setForm(prev => ({ ...prev, [key]: value }));
  };

  const handleTypeChange = (type: AgentType) => {
    setCreateError(null);
    setForm(prev => ({ ...prev, type, engine: '', memory: '' }));
  };

  const handleCreate = async () => {
    setIsCreating(true);
    setCreateError(null);
    try {
      const metadata: Record<string, string> = {
        preferred_memory: form.memory,
        agent_type: form.type,
      };
      if (form.routingRules.length > 0) {
        metadata.engine_routing = JSON.stringify(form.routingRules);
      }
      await api.createAgent({
        name: form.name,
        description: form.desc,
        default_engine: form.engine,
        metadata,
        password: form.password || undefined
      }, apiKey);
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
    setForm(prev => ({
      ...prev,
      routingRules: [...prev.routingRules, { match: 'default', engine: '' }],
    }));
  };

  const updateRoutingRule = (index: number, field: keyof RoutingRuleEntry, value: string) => {
    setForm(prev => {
      const rules = [...prev.routingRules];
      rules[index] = { ...rules[index], [field]: value };
      return { ...prev, routingRules: rules };
    });
  };

  const removeRoutingRule = (index: number) => {
    setForm(prev => ({
      ...prev,
      routingRules: prev.routingRules.filter((_, i) => i !== index),
    }));
  };

  return {
    form, updateField, handleTypeChange, handleCreate, isCreating, createError,
    addRoutingRule, updateRoutingRule, removeRoutingRule,
  };
}
