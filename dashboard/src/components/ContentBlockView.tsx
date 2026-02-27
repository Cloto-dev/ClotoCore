import { ContentBlock } from '../types';
import { api } from '../services/api';
import { MarkdownRenderer } from './MarkdownRenderer';

/** Render a single ContentBlock */
export function ContentBlockView({ block }: { block: ContentBlock }) {
  switch (block.type) {
    case 'text':
      return <MarkdownRenderer content={block.text || ''} />;
    case 'image':
      return (
        <img
          src={block.attachment_id ? api.getAttachmentUrl(block.attachment_id) : block.url}
          alt={block.filename || 'image'}
          className="max-w-full rounded-lg mt-1"
          loading="lazy"
        />
      );
    case 'code':
      return (
        <pre className="bg-black/10 rounded-lg p-2 mt-1 overflow-x-auto text-[10px] font-mono">
          <code>{block.text}</code>
        </pre>
      );
    case 'tool_result':
      return (
        <div className="bg-black/10 rounded-lg p-2 mt-1 text-[10px] font-mono border-l-2 border-emerald-400">
          {block.text}
        </div>
      );
    case 'file':
      return (
        <a
          href={block.attachment_id ? api.getAttachmentUrl(block.attachment_id) : block.url}
          download={block.filename}
          className="inline-flex items-center gap-1 underline text-[10px] mt-1"
        >
          {block.filename || 'Download'}
        </a>
      );
    default:
      return <span>{block.text || ''}</span>;
  }
}

/** Render message content (supports both string and ContentBlock[]) */
export function MessageContent({ content }: { content: string | ContentBlock[] }) {
  if (typeof content === 'string') {
    return <span>{content}</span>;
  }
  return (
    <>
      {content.map((block, i) => (
        <ContentBlockView key={i} block={block} />
      ))}
    </>
  );
}
