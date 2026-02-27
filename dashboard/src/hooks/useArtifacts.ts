import { useState, useCallback } from 'react';

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

export function useArtifacts(): UseArtifactsResult {
  const [artifacts, setArtifacts] = useState<Artifact[]>([]);
  const [isOpen, setIsOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState(0);

  const addArtifact = useCallback((artifact: Omit<Artifact, 'id'>) => {
    setArtifacts(prev => {
      // Deduplicate by code content
      if (prev.some(a => a.code === artifact.code)) return prev;
      const id = `artifact-${Date.now()}-${prev.length}`;
      const next = [...prev, { ...artifact, id }];
      setActiveIndex(next.length - 1);
      return next;
    });
    setIsOpen(true);
  }, []);

  const clearArtifacts = useCallback(() => {
    setArtifacts([]);
    setActiveIndex(0);
    setIsOpen(false);
  }, []);

  const closePanel = useCallback(() => setIsOpen(false), []);
  const openPanel = useCallback(() => setIsOpen(true), []);

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
