import DOMPurify from 'dompurify';
import { useCallback, useEffect, useMemo, useRef } from 'react';
import { renderMarkdown, renderMarkdownIncremental } from '../lib/markdown';

const EXT_MAP: Record<string, string> = {
  typescript: 'ts', javascript: 'js', python: 'py', rust: 'rs',
  bash: 'sh', json: 'json', css: 'css', html: 'html', sql: 'sql',
  yaml: 'yml', xml: 'xml', ts: 'ts', js: 'js', py: 'py', sh: 'sh',
};

const COPY_FEEDBACK_MS = 2000;

// Allow code block wrapper elements through DOMPurify
const SANITIZE_CONFIG = {
  ADD_TAGS: ['button', 'svg', 'rect', 'path', 'polyline', 'line'],
  ADD_ATTR: ['data-raw', 'data-ext', 'data-lang', 'data-lines', 'viewBox', 'fill', 'stroke', 'stroke-width', 'd', 'x', 'y', 'width', 'height', 'rx', 'x1', 'y1', 'x2', 'y2', 'points'],
};

interface MarkdownRendererProps {
  content: string;
  incremental?: boolean;
  onCodeBlock?: (code: string, language: string, lineCount: number) => void;
  className?: string;
}

export function MarkdownRenderer({ content, incremental = false, onCodeBlock, className = '' }: MarkdownRendererProps) {
  const extractedCodesRef = useRef<Set<string>>(new Set());
  const containerRef = useRef<HTMLDivElement>(null);

  const html = useMemo(() => {
    const raw = incremental ? renderMarkdownIncremental(content) : renderMarkdown(content);

    if (!onCodeBlock) return raw;

    // Replace large code blocks with placeholders in the HTML string
    return raw.replace(
      /<pre class="hljs-code-block" data-lang="([^"]*)" data-lines="(\d+)" data-raw="([^"]*)">/g,
      (_match, lang, linesStr, rawEncoded) => {
        const lines = parseInt(linesStr, 10);
        if (lines >= 15) {
          const code = decodeURIComponent(rawEncoded);
          if (!extractedCodesRef.current.has(code)) {
            extractedCodesRef.current.add(code);
            onCodeBlock(code, lang, lines);
          }
          return `<div class="artifact-placeholder"><span class="text-[9px] font-mono uppercase tracking-wider opacity-60">${lang} · ${lines} lines</span><span class="text-[10px] font-mono opacity-80">View in panel →</span></div><pre style="display:none" data-lang="${lang}" data-lines="${linesStr}">`;
        }
        return `<pre class="hljs-code-block" data-lang="${lang}" data-lines="${linesStr}" data-raw="${rawEncoded}">`;
      },
    );
  }, [content, incremental, onCodeBlock]);

  // Reset extracted codes when content fully changes (new message)
  const prevContentRef = useRef(content);
  useEffect(() => {
    if (content !== prevContentRef.current && content.length < prevContentRef.current.length) {
      extractedCodesRef.current.clear();
    }
    prevContentRef.current = content;
  }, [content]);

  // Event delegation for code block copy/download buttons
  const handleClick = useCallback((e: React.MouseEvent) => {
    const target = e.target as HTMLElement;
    const copyBtn = target.closest('.code-block-copy') as HTMLElement | null;
    const downloadBtn = target.closest('.code-block-download') as HTMLElement | null;

    if (!copyBtn && !downloadBtn) return;

    const wrapper = (copyBtn || downloadBtn)!.closest('.code-block-wrapper') as HTMLElement | null;
    if (!wrapper) return;

    const rawEncoded = wrapper.getAttribute('data-raw');
    if (!rawEncoded) return;
    const code = decodeURIComponent(rawEncoded);

    if (copyBtn) {
      navigator.clipboard.writeText(code);
      copyBtn.classList.add('copied');
      setTimeout(() => copyBtn.classList.remove('copied'), COPY_FEEDBACK_MS);
    }

    if (downloadBtn) {
      const lang = wrapper.getAttribute('data-ext') || 'text';
      const ext = EXT_MAP[lang] || 'txt';
      const blob = new Blob([code], { type: 'text/plain' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `snippet.${ext}`;
      a.click();
      URL.revokeObjectURL(url);
    }
  }, []);

  return (
    <div
      ref={containerRef}
      className={`chat-markdown ${className}`}
      onClick={handleClick}
      dangerouslySetInnerHTML={{ __html: DOMPurify.sanitize(html, SANITIZE_CONFIG) }}
    />
  );
}
