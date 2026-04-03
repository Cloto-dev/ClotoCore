import { useCallback, useState } from 'react';
import type { AgentDialogue, ExternalAction } from '../types';

// ── Storage Keys ──

const ARTIFACTS_KEY = 'cloto-actions';
const OPEN_KEY = 'cloto-actions-open';
const DIALOGUES_KEY = 'cloto-dialogues';
const EXTERNAL_ACTIONS_KEY = 'cloto-external-actions';

// Legacy migration
const LEGACY_ARTIFACTS_KEY = 'cloto-artifacts';
const LEGACY_OPEN_KEY = 'cloto-artifacts-open';

// ── Types ──

export type ActionCategory = 'code' | 'dialogues' | 'external';

export interface Artifact {
  id: string;
  code: string;
  language: string;
  lineCount: number;
}

export interface DialogueTab {
  dialogue: AgentDialogue;
  unread: boolean;
}

export interface ExternalActionTab {
  action: ExternalAction;
  unread: boolean;
}

export interface UseActionsResult {
  // Panel state
  isOpen: boolean;
  openPanel: () => void;
  closePanel: () => void;

  // Category
  activeCategory: ActionCategory;
  setActiveCategory: (cat: ActionCategory) => void;
  hasDialogues: boolean;
  hasExternalActions: boolean;

  // Artifacts (code)
  artifacts: Artifact[];
  activeArtifactIndex: number;
  addArtifact: (artifact: Omit<Artifact, 'id'>) => void;
  setActiveArtifactIndex: (index: number) => void;
  clearArtifacts: () => void;
  /** Clear all actions (artifacts + dialogues) and hide the panel. */
  clearAll: () => void;

  // Dialogues
  dialogues: DialogueTab[];
  activeDialogueIndex: number;
  setActiveDialogueIndex: (index: number) => void;
  addOrUpdateDialogue: (dialogue: AgentDialogue) => void;
  markDialogueRead: (dialogueId: string) => void;

  // External Actions
  externalActions: ExternalActionTab[];
  addOrUpdateExternalAction: (action: ExternalAction) => void;

  // Counts
  totalCount: number;
  unreadDialogueCount: number;
  unreadExternalCount: number;
}

// ── Persistence Helpers ──

function loadArtifacts(): Artifact[] {
  try {
    // Migrate from legacy key
    const legacy = sessionStorage.getItem(LEGACY_ARTIFACTS_KEY);
    if (legacy && !sessionStorage.getItem(ARTIFACTS_KEY)) {
      sessionStorage.setItem(ARTIFACTS_KEY, legacy);
      sessionStorage.removeItem(LEGACY_ARTIFACTS_KEY);
    }
    sessionStorage.removeItem(LEGACY_OPEN_KEY);

    const raw = sessionStorage.getItem(ARTIFACTS_KEY);
    return raw ? JSON.parse(raw) : [];
  } catch {
    return [];
  }
}

function saveArtifacts(artifacts: Artifact[]) {
  try {
    if (artifacts.length === 0) {
      sessionStorage.removeItem(ARTIFACTS_KEY);
    } else {
      sessionStorage.setItem(ARTIFACTS_KEY, JSON.stringify(artifacts));
    }
  } catch {
    /* sessionStorage full — ignore */
  }
}

const MAX_DIALOGUES = 100;
const MAX_DIALOGUE_AGE_MS = 7 * 24 * 60 * 60 * 1000; // 7 days

/** Prune dialogues exceeding age or count limits. */
function pruneDialogues(dialogues: DialogueTab[]): DialogueTab[] {
  const cutoff = Date.now() - MAX_DIALOGUE_AGE_MS;
  const fresh = dialogues.filter((d) => d.dialogue.timestamp >= cutoff);
  return fresh.length > MAX_DIALOGUES ? fresh.slice(-MAX_DIALOGUES) : fresh;
}

function loadDialogues(): DialogueTab[] {
  try {
    const raw = localStorage.getItem(DIALOGUES_KEY);
    if (!raw) return [];
    const parsed: DialogueTab[] = JSON.parse(raw);
    const pruned = pruneDialogues(parsed);
    // Persist pruned result if items were removed
    if (pruned.length !== parsed.length) saveDialogues(pruned);
    return pruned;
  } catch {
    return [];
  }
}

function saveDialogues(dialogues: DialogueTab[]) {
  try {
    if (dialogues.length === 0) {
      localStorage.removeItem(DIALOGUES_KEY);
    } else {
      localStorage.setItem(DIALOGUES_KEY, JSON.stringify(pruneDialogues(dialogues)));
    }
  } catch {
    /* localStorage full — ignore */
  }
}

// ── External Actions Persistence ──

function loadExternalActions(): ExternalActionTab[] {
  try {
    const raw = localStorage.getItem(EXTERNAL_ACTIONS_KEY);
    if (!raw) return [];
    const parsed: ExternalActionTab[] = JSON.parse(raw);
    const cutoff = Date.now() - MAX_DIALOGUE_AGE_MS;
    const fresh = parsed.filter((e) => e.action.timestamp >= cutoff);
    const pruned = fresh.length > MAX_DIALOGUES ? fresh.slice(-MAX_DIALOGUES) : fresh;
    if (pruned.length !== parsed.length) saveExternalActions(pruned);
    return pruned;
  } catch {
    return [];
  }
}

function saveExternalActions(actions: ExternalActionTab[]) {
  try {
    if (actions.length === 0) {
      localStorage.removeItem(EXTERNAL_ACTIONS_KEY);
    } else {
      localStorage.setItem(EXTERNAL_ACTIONS_KEY, JSON.stringify(actions));
    }
  } catch {
    /* localStorage full — ignore */
  }
}

// ── Hook ──

export function useActions(): UseActionsResult {
  const [artifacts, setArtifacts] = useState<Artifact[]>(loadArtifacts);
  const [dialogues, setDialogues] = useState<DialogueTab[]>(loadDialogues);
  const [externalActions, setExternalActions] = useState<ExternalActionTab[]>(loadExternalActions);
  const [isOpen, setIsOpen] = useState(() => {
    const saved = loadArtifacts();
    return saved.length > 0 && sessionStorage.getItem(OPEN_KEY) !== 'closed';
  });
  const [activeCategory, setActiveCategory] = useState<ActionCategory>('code');
  const [activeArtifactIndex, setActiveArtifactIndex] = useState(0);
  const [activeDialogueIndex, setActiveDialogueIndex] = useState(0);

  // ── Artifacts ──

  const addArtifact = useCallback((artifact: Omit<Artifact, 'id'>) => {
    setArtifacts((prev) => {
      // Exact duplicate — skip
      if (prev.some((a) => a.code === artifact.code)) return prev;

      // First artifact in this session → auto-select Code category
      if (prev.length === 0) setActiveCategory('code');

      // Prefix match — streaming growth of the same code block
      const growthIndex = prev.findIndex(
        (a) =>
          a.language === artifact.language && (artifact.code.startsWith(a.code) || a.code.startsWith(artifact.code)),
      );
      if (growthIndex >= 0) {
        if (artifact.code.length <= prev[growthIndex].code.length) return prev;
        const next = [...prev];
        next[growthIndex] = { ...next[growthIndex], code: artifact.code, lineCount: artifact.lineCount };
        setActiveArtifactIndex(growthIndex);
        saveArtifacts(next);
        return next;
      }

      const id = `artifact-${Date.now()}-${prev.length}`;
      const next = [...prev, { ...artifact, id }];
      setActiveArtifactIndex(next.length - 1);
      saveArtifacts(next);
      return next;
    });
    setIsOpen(true);
    sessionStorage.removeItem(OPEN_KEY);
  }, []);

  const clearArtifacts = useCallback(() => {
    setArtifacts([]);
    setActiveArtifactIndex(0);
    saveArtifacts([]);
  }, []);

  /** Clear all actions (artifacts + dialogues + external) and hide the panel. */
  const clearAll = useCallback(() => {
    setArtifacts([]);
    setActiveArtifactIndex(0);
    saveArtifacts([]);
    setDialogues([]);
    setActiveDialogueIndex(0);
    saveDialogues([]);
    setExternalActions([]);
    saveExternalActions([]);
    setActiveCategory('code');
    setIsOpen(false);
    sessionStorage.setItem(OPEN_KEY, 'closed');
  }, []);

  // ── Dialogues ──

  const addOrUpdateDialogue = useCallback((dialogue: AgentDialogue) => {
    setDialogues((prev) => {
      const existingIndex = prev.findIndex((d) => d.dialogue.dialogue_id === dialogue.dialogue_id);
      if (existingIndex >= 0) {
        // Update existing dialogue in-place (e.g. pending → success)
        const next = [...prev];
        next[existingIndex] = { dialogue, unread: true };
        saveDialogues(next);
        return next;
      }
      // First dialogue in this session → auto-select Dialogues category
      if (prev.length === 0) setActiveCategory('dialogues');
      // New dialogue — append (browser-style: don't switch active tab)
      const next = [...prev, { dialogue, unread: true }];
      saveDialogues(next);
      return next;
    });
    setIsOpen(true);
    sessionStorage.removeItem(OPEN_KEY);
  }, []);

  const markDialogueRead = useCallback((dialogueId: string) => {
    setDialogues((prev) => {
      const idx = prev.findIndex((d) => d.dialogue.dialogue_id === dialogueId);
      if (idx < 0 || !prev[idx].unread) return prev;
      const next = [...prev];
      next[idx] = { ...next[idx], unread: false };
      saveDialogues(next);
      return next;
    });
  }, []);

  // ── External Actions ──

  const addOrUpdateExternalAction = useCallback((action: ExternalAction) => {
    setExternalActions((prev) => {
      const existingIndex = prev.findIndex((e) => e.action.action_id === action.action_id);
      if (existingIndex >= 0) {
        const next = [...prev];
        next[existingIndex] = { action, unread: true };
        saveExternalActions(next);
        return next;
      }
      if (prev.length === 0) setActiveCategory('external');
      const next = [...prev, { action, unread: true }];
      saveExternalActions(next);
      return next;
    });
    setIsOpen(true);
    sessionStorage.removeItem(OPEN_KEY);
  }, []);

  const handleDialogueTabChange = useCallback(
    (index: number) => {
      setActiveDialogueIndex(index);
      // Mark as read when user clicks the tab
      const tab = dialogues[index];
      if (tab?.unread) {
        markDialogueRead(tab.dialogue.dialogue_id);
      }
    },
    [dialogues, markDialogueRead],
  );

  // ── Panel ──

  const closePanel = useCallback(() => {
    setIsOpen(false);
    sessionStorage.setItem(OPEN_KEY, 'closed');
  }, []);

  const openPanel = useCallback(() => {
    setIsOpen(true);
    sessionStorage.removeItem(OPEN_KEY);
  }, []);

  // ── Derived ──

  const hasDialogues = dialogues.length > 0;
  const hasExternalActions = externalActions.length > 0;
  const unreadDialogueCount = dialogues.filter((d) => d.unread).length;
  const unreadExternalCount = externalActions.filter((e) => e.unread).length;
  const totalCount = artifacts.length + dialogues.length + externalActions.length;

  return {
    isOpen,
    openPanel,
    closePanel,
    activeCategory,
    setActiveCategory,
    hasDialogues,
    hasExternalActions,
    artifacts,
    activeArtifactIndex,
    addArtifact,
    setActiveArtifactIndex,
    clearArtifacts,
    clearAll,
    dialogues,
    activeDialogueIndex,
    setActiveDialogueIndex: handleDialogueTabChange,
    addOrUpdateDialogue,
    markDialogueRead,
    externalActions,
    addOrUpdateExternalAction,
    totalCount,
    unreadDialogueCount,
    unreadExternalCount,
  };
}
