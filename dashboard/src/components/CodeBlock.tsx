import { useState, useCallback, useMemo } from 'react';
import DOMPurify from 'dompurify';
import { Copy, Check, Download } from 'lucide-react';
import { hljs } from '../lib/markdown';

const EXT_MAP: Record<string, string> = {
  typescript: 'ts', javascript: 'js', python: 'py', rust: 'rs',
  bash: 'sh', json: 'json', css: 'css', html: 'html', sql: 'sql',
  yaml: 'yml', xml: 'xml', ts: 'ts', js: 'js', py: 'py', sh: 'sh',
};

interface CodeBlockProps {
  code: string;
  language: string;
  showHeader?: boolean;
  maxHeight?: string;
  className?: string;
}

export function CodeBlock({ code, language, showHeader = true, maxHeight = 'none', className = '' }: CodeBlockProps) {
  const [copied, setCopied] = useState(false);

  const highlighted = useMemo(() => {
    if (hljs.getLanguage(language)) {
      return hljs.highlight(code, { language }).value;
    }
    return hljs.highlightAuto(code).value;
  }, [code, language]);

  const handleCopy = useCallback(async () => {
    await navigator.clipboard.writeText(code);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  }, [code]);

  const handleDownload = useCallback(() => {
    const ext = EXT_MAP[language] || 'txt';
    const blob = new Blob([code], { type: 'text/plain' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `snippet.${ext}`;
    a.click();
    URL.revokeObjectURL(url);
  }, [code, language]);

  return (
    <div className={`rounded-lg overflow-hidden my-2 ${className}`} style={{ backgroundColor: '#0d1117' }}>
      {showHeader && (
        <div className="flex items-center justify-between px-3 py-1.5" style={{ backgroundColor: '#161b22' }}>
          <span className="text-[9px] font-mono uppercase tracking-wider text-gray-400">
            {language}
          </span>
          <div className="flex items-center gap-1">
            <button
              onClick={handleCopy}
              className="p-1 rounded hover:bg-white/10 transition-colors text-gray-400 hover:text-gray-200"
              title={copied ? 'Copied!' : 'Copy'}
            >
              {copied ? <Check size={12} className="text-emerald-400" /> : <Copy size={12} />}
            </button>
            <button
              onClick={handleDownload}
              className="p-1 rounded hover:bg-white/10 transition-colors text-gray-400 hover:text-gray-200"
              title="Download"
            >
              <Download size={12} />
            </button>
          </div>
        </div>
      )}
      <div className="overflow-x-auto" style={{ maxHeight }}>
        <pre className="p-3 m-0 text-[11px] leading-relaxed font-mono">
          <code
            className={`hljs language-${language}`}
            dangerouslySetInnerHTML={{ __html: DOMPurify.sanitize(highlighted, { ALLOWED_TAGS: ['span'], ALLOWED_ATTR: ['class'] }) }}
          />
        </pre>
      </div>
    </div>
  );
}
