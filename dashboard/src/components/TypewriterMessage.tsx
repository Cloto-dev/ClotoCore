import { useEffect, useCallback, useRef } from 'react';
import { useTypewriter } from '../hooks/useTypewriter';
import { MarkdownRenderer } from './MarkdownRenderer';

interface TypewriterMessageProps {
  text: string;
  onComplete: () => void;
  onCodeBlock?: (code: string, language: string, lineCount: number) => void;
}

export function TypewriterMessage({ text, onComplete, onCodeBlock }: TypewriterMessageProps) {
  const onCodeBlockRef = useRef(onCodeBlock);
  onCodeBlockRef.current = onCodeBlock;

  // Final scan before completion: extract code blocks from the full text
  const handleComplete = useCallback(() => {
    // Trigger a final scan with the full text by deferring onComplete
    // so MarkdownRenderer has one render cycle with onCodeBlock enabled
    requestAnimationFrame(() => onComplete());
  }, [onComplete]);

  const { displayText, isAnimating, skip } = useTypewriter({
    text,
    speed: 5,
    onComplete: handleComplete,
  });

  // Skip on Enter key
  useEffect(() => {
    if (!isAnimating) return;
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === 'Enter') skip();
    };
    window.addEventListener('keydown', handleKey);
    return () => window.removeEventListener('keydown', handleKey);
  }, [isAnimating, skip]);

  const handleClick = useCallback(() => {
    if (isAnimating) skip();
  }, [isAnimating, skip]);

  // Always pass onCodeBlock — MarkdownRenderer deduplicates via useArtifacts
  return (
    <div onClick={handleClick} className={isAnimating ? 'cursor-pointer' : ''}>
      <MarkdownRenderer
        content={displayText}
        incremental={isAnimating}
        onCodeBlock={onCodeBlock}
      />
      {isAnimating && (
        <span className="inline-block w-[2px] h-[1em] bg-current animate-blink ml-0.5 align-text-bottom" />
      )}
    </div>
  );
}
