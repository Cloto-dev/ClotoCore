import { marked, type Renderer, type Tokens } from 'marked';
import hljs from 'highlight.js/lib/core';

// Tree-shake: register only needed languages
import javascript from 'highlight.js/lib/languages/javascript';
import typescript from 'highlight.js/lib/languages/typescript';
import python from 'highlight.js/lib/languages/python';
import rust from 'highlight.js/lib/languages/rust';
import bash from 'highlight.js/lib/languages/bash';
import json from 'highlight.js/lib/languages/json';
import css from 'highlight.js/lib/languages/css';
import xml from 'highlight.js/lib/languages/xml';
import sql from 'highlight.js/lib/languages/sql';
import yaml from 'highlight.js/lib/languages/yaml';

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
    const highlighted = language
      ? hljs.highlight(text, { language }).value
      : hljs.highlightAuto(text).value;
    const detectedLang = language || 'text';
    const lineCount = text.split('\n').length;
    return `<pre class="hljs-code-block" data-lang="${detectedLang}" data-lines="${lineCount}" data-raw="${encodeURIComponent(text)}"><code class="hljs language-${detectedLang}">${highlighted}</code></pre>`;
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
