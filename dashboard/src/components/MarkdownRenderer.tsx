import { useMemo, useRef, useEffect } from 'react';
import DOMPurify from 'dompurify';
import { renderMarkdown, renderMarkdownIncremental } from '../lib/markdown';

interface MarkdownRendererProps {
  content: string;
  incremental?: boolean;
  onCodeBlock?: (code: string, language: string, lineCount: number) => void;
  className?: string;
}

export function MarkdownRenderer({ content, incremental = false, onCodeBlock, className = '' }: MarkdownRendererProps) {
  const extractedCodesRef = useRef<Set<string>>(new Set());

  const html = useMemo(() => {
    const raw = incremental
      ? renderMarkdownIncremental(content)
      : renderMarkdown(content);

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
      }
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

  return (
    <div
      className={`chat-markdown ${className}`}
      dangerouslySetInnerHTML={{ __html: DOMPurify.sanitize(html) }}
    />
  );
}
