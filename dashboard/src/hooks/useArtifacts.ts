import { useCallback, useState } from 'react';

const STORAGE_KEY = 'cloto-artifacts';
const OPEN_KEY = 'cloto-artifacts-open';

export interface Artifact {
  id: string;
  code: string;
  language: string;
  lineCount: number;
}

export interface UseArtifactsResult {
  artifacts: Artifact[];
  isOpen: boolean;
  activeIndex: number;
  addArtifact: (artifact: Omit<Artifact, 'id'>) => void;
  clearArtifacts: () => void;
  setActiveIndex: (index: number) => void;
  closePanel: () => void;
  openPanel: () => void;
}

function loadArtifacts(): Artifact[] {
  try {
    const raw = sessionStorage.getItem(STORAGE_KEY);
    return raw ? JSON.parse(raw) : [];
  } catch {
    return [];
  }
}

function saveArtifacts(artifacts: Artifact[]) {
  try {
    if (artifacts.length === 0) {
      sessionStorage.removeItem(STORAGE_KEY);
    } else {
      sessionStorage.setItem(STORAGE_KEY, JSON.stringify(artifacts));
    }
  } catch {
    // sessionStorage full or unavailable — ignore
  }
}

export function useArtifacts(): UseArtifactsResult {
  const [artifacts, setArtifacts] = useState<Artifact[]>(loadArtifacts);
  const [isOpen, setIsOpen] = useState(() => {
    const saved = loadArtifacts();
    return saved.length > 0 && sessionStorage.getItem(OPEN_KEY) !== 'closed';
  });
  const [activeIndex, setActiveIndex] = useState(0);

  const addArtifact = useCallback((artifact: Omit<Artifact, 'id'>) => {
    setArtifacts((prev) => {
      // Deduplicate by code content
      if (prev.some((a) => a.code === artifact.code)) return prev;
      const id = `artifact-${Date.now()}-${prev.length}`;
      const next = [...prev, { ...artifact, id }];
      setActiveIndex(next.length - 1);
      saveArtifacts(next);
      return next;
    });
    setIsOpen(true);
    sessionStorage.removeItem(OPEN_KEY);
  }, []);

  const clearArtifacts = useCallback(() => {
    setArtifacts([]);
    setActiveIndex(0);
    setIsOpen(false);
    saveArtifacts([]);
    sessionStorage.removeItem(OPEN_KEY);
  }, []);

  const closePanel = useCallback(() => {
    setIsOpen(false);
    sessionStorage.setItem(OPEN_KEY, 'closed');
  }, []);

  const openPanel = useCallback(() => {
    setIsOpen(true);
    sessionStorage.removeItem(OPEN_KEY);
  }, []);

  return {
    artifacts,
    isOpen,
    activeIndex,
    addArtifact,
    clearArtifacts,
    setActiveIndex,
    closePanel,
    openPanel,
  };
}
