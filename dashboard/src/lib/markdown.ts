import hljs from 'highlight.js/lib/core';
import bash from 'highlight.js/lib/languages/bash';
import css from 'highlight.js/lib/languages/css';
// Tree-shake: register only needed languages
import javascript from 'highlight.js/lib/languages/javascript';
import json from 'highlight.js/lib/languages/json';
import python from 'highlight.js/lib/languages/python';
import rust from 'highlight.js/lib/languages/rust';
import sql from 'highlight.js/lib/languages/sql';
import typescript from 'highlight.js/lib/languages/typescript';
import xml from 'highlight.js/lib/languages/xml';
import yaml from 'highlight.js/lib/languages/yaml';
import { marked, type Renderer, type Tokens } from 'marked';

hljs.registerLanguage('javascript', javascript);
hljs.registerLanguage('js', javascript);
hljs.registerLanguage('typescript', typescript);
hljs.registerLanguage('ts', typescript);
hljs.registerLanguage('python', python);
hljs.registerLanguage('py', python);
hljs.registerLanguage('rust', rust);
hljs.registerLanguage('bash', bash);
hljs.registerLanguage('sh', bash);
hljs.registerLanguage('shell', bash);
hljs.registerLanguage('json', json);
hljs.registerLanguage('css', css);
hljs.registerLanguage('html', xml);
hljs.registerLanguage('xml', xml);
hljs.registerLanguage('sql', sql);
hljs.registerLanguage('yaml', yaml);
hljs.registerLanguage('yml', yaml);

export { hljs };

// Custom renderer: code blocks with data attributes for artifact extraction
const renderer: Partial<Renderer> = {
  code({ text, lang }: Tokens.Code) {
    const language = lang && hljs.getLanguage(lang) ? lang : null;
    const highlighted = language ? hljs.highlight(text, { language }).value : hljs.highlightAuto(text).value;
    const detectedLang = language || 'text';
    const lineCount = text.split('\n').length;
    const rawEncoded = encodeURIComponent(text);
    return `<div class="code-block-wrapper" data-raw="${rawEncoded}" data-ext="${detectedLang}"><div class="code-block-header"><span class="code-block-lang">${detectedLang}</span><div class="code-block-actions"><button class="code-block-copy" title="Copy" aria-label="Copy code"><svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg></button><button class="code-block-download" title="Download" aria-label="Download code"><svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg></button></div></div><pre class="hljs-code-block" data-lang="${detectedLang}" data-lines="${lineCount}" data-raw="${rawEncoded}"><code class="hljs language-${detectedLang}">${highlighted}</code></pre></div>`;
  },
};

marked.use({ renderer, gfm: true, breaks: true });

/** Parse a complete markdown string to HTML. */
export function renderMarkdown(text: string): string {
  return marked.parse(text, { async: false }) as string;
}

/**
 * Parse a partial markdown string during typewriter animation.
 * Auto-closes unclosed code fences to prevent broken HTML.
 */
export function renderMarkdownIncremental(text: string): string {
  const fenceCount = (text.match(/^```/gm) || []).length;
  let safeText = text;
  if (fenceCount % 2 !== 0) {
    safeText = text + '\n```';
  }
  return marked.parse(safeText, { async: false }) as string;
}
