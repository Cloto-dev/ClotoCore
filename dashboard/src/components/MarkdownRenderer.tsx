import { useEffect, useRef, useCallback } from 'react';
import { renderMarkdown, renderMarkdownIncremental } from '../lib/markdown';

interface MarkdownRendererProps {
  content: string;
  incremental?: boolean;
  onCodeBlock?: (code: string, language: string, lineCount: number) => void;
  className?: string;
}

export function MarkdownRenderer({ content, incremental = false, onCodeBlock, className = '' }: MarkdownRendererProps) {
  const containerRef = useRef<HTMLDivElement>(null);

  const html = incremental
    ? renderMarkdownIncremental(content)
    : renderMarkdown(content);

  // Scan DOM for large code blocks and notify parent
  const scanCodeBlocks = useCallback(() => {
    if (!containerRef.current || !onCodeBlock) return;
    const codeBlocks = containerRef.current.querySelectorAll('pre.hljs-code-block');
    codeBlocks.forEach((block) => {
      const lines = parseInt(block.getAttribute('data-lines') || '0', 10);
      const lang = block.getAttribute('data-lang') || 'text';
      const raw = block.getAttribute('data-raw');
      const code = raw ? decodeURIComponent(raw) : (block.querySelector('code')?.textContent || '');
      if (lines >= 15) {
        onCodeBlock(code, lang, lines);
        // Replace with inline placeholder
        block.className = 'artifact-placeholder';
        block.removeAttribute('data-raw');
        block.innerHTML = `
          <span class="text-[9px] font-mono uppercase tracking-wider opacity-60">${lang} · ${lines} lines</span>
          <span class="text-[10px] font-mono opacity-80">View in panel →</span>
        `;
      }
    });
  }, [onCodeBlock]);

  useEffect(() => {
    scanCodeBlocks();
  }, [html, scanCodeBlocks]);

  return (
    <div
      ref={containerRef}
      className={`chat-markdown ${className}`}
      dangerouslySetInnerHTML={{ __html: html }}
    />
  );
}
