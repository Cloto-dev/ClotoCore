import { useEffect, useCallback } from 'react';
import { useTypewriter } from '../hooks/useTypewriter';
import { MarkdownRenderer } from './MarkdownRenderer';

interface TypewriterMessageProps {
  text: string;
  onComplete: () => void;
  onCodeBlock?: (code: string, language: string, lineCount: number) => void;
}

export function TypewriterMessage({ text, onComplete, onCodeBlock }: TypewriterMessageProps) {
  const { displayText, isAnimating, skip } = useTypewriter({
    text,
    speed: 5,
    onComplete,
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

  return (
    <div onClick={handleClick} className={isAnimating ? 'cursor-pointer' : ''}>
      <MarkdownRenderer
        content={displayText}
        incremental={isAnimating}
        onCodeBlock={isAnimating ? undefined : onCodeBlock}
      />
      {isAnimating && (
        <span className="inline-block w-[2px] h-[1em] bg-current animate-blink ml-0.5 align-text-bottom" />
      )}
    </div>
  );
}
