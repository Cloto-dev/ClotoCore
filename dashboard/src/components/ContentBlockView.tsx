import { useState } from 'react';
import { createPortal } from 'react-dom';
import { Image, FileAudio, Download, X } from 'lucide-react';
import { ContentBlock } from '../types';
import { api } from '../services/api';
import { isTauri } from '../lib/tauri';
import { tryParseJsonObject } from '../lib/json';
import { MarkdownRenderer } from './MarkdownRenderer';

function ImageLightbox({ src, alt, onClose }: { src: string; alt: string; onClose: () => void }) {
  return createPortal(
    <div
      className="fixed inset-0 z-[9990] flex items-center justify-center bg-black/80 backdrop-blur-sm cursor-zoom-out"
      onClick={onClose}
    >
      <button onClick={onClose} className="absolute top-4 right-4 p-2 rounded-full bg-black/50 text-white hover:bg-black/70 transition-colors z-[9991]">
        <X size={20} />
      </button>
      <img
        src={src}
        alt={alt}
        className="max-w-[90vw] max-h-[90vh] object-contain rounded-lg shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      />
    </div>,
    document.body
  );
}

function ClickableImage({ src, alt, className }: { src: string; alt: string; className?: string }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <img
        src={src}
        alt={alt}
        className={`cursor-zoom-in ${className || ''}`}
        loading="lazy"
        onClick={() => setOpen(true)}
      />
      {open && <ImageLightbox src={src} alt={alt} onClose={() => setOpen(false)} />}
    </>
  );
}

function resolveLocalPath(filePath: string): string | null {
  if (!filePath) return null;
  if (isTauri) {
    // Tauri asset protocol for local files
    try {
      // Dynamic import would be ideal but for sync rendering we use the protocol directly
      return `asset://localhost/${filePath.replace(/\\/g, '/')}`;
    } catch {
      return null;
    }
  }
  return null;
}

const IMAGE_EXT = /\.(png|jpg|jpeg|gif|webp|bmp|svg)$/i;
const AUDIO_EXT = /\.(wav|mp3|ogg|flac|m4a)$/i;

/** Render a single ContentBlock */
export function ContentBlockView({ block }: { block: ContentBlock }) {
  switch (block.type) {
    case 'text':
      return <MarkdownRenderer content={block.text || ''} />;

    case 'image':
      return (
        <ClickableImage
          src={(block.attachment_id ? api.getAttachmentUrl(block.attachment_id) : block.url) || ''}
          alt={block.filename || 'image'}
          className="max-w-xs max-h-64 rounded-lg mt-1 object-cover"
        />
      );

    case 'code':
      return (
        <pre className="bg-black/10 rounded-lg p-2 mt-1 overflow-x-auto text-[10px] font-mono">
          <code>{block.text}</code>
        </pre>
      );

    case 'tool_result': {
      const parsed = tryParseJsonObject(block.text);

      // Image file path detection in tool results
      if (parsed?.path && typeof parsed.path === 'string' && IMAGE_EXT.test(parsed.path)) {
        const src = resolveLocalPath(parsed.path);
        const basename = (parsed.path as string).split(/[/\\]/).pop() || 'image';
        return (
          <div className="mt-1 space-y-1">
            {src ? (
              <ClickableImage src={src} alt={basename} className="max-w-xs max-h-64 rounded-lg border border-edge object-cover" />
            ) : (
              <div className="flex items-center gap-2 text-[10px] font-mono text-content-secondary bg-black/5 rounded-lg p-2">
                <Image size={12} className="text-brand" />
                <span>{basename}</span>
              </div>
            )}
            <div className="text-[9px] font-mono text-content-tertiary">{parsed.path as string}</div>
          </div>
        );
      }

      // Audio file path detection in tool results
      if (parsed?.path && typeof parsed.path === 'string' && AUDIO_EXT.test(parsed.path)) {
        const src = resolveLocalPath(parsed.path);
        const basename = (parsed.path as string).split(/[/\\]/).pop() || 'audio';
        return (
          <div className="mt-1 space-y-1">
            {src ? (
              <audio controls className="max-w-md" src={src} />
            ) : (
              <div className="flex items-center gap-2 text-[10px] font-mono text-content-secondary bg-black/5 rounded-lg p-2">
                <FileAudio size={12} className="text-emerald-400" />
                <span>{basename}</span>
              </div>
            )}
            <div className="text-[9px] font-mono text-content-tertiary">{parsed.path as string}</div>
          </div>
        );
      }

      // Successful tool result with status
      if (parsed?.status === 'ok') {
        const displayFields = Object.entries(parsed)
          .filter(([k]) => k !== 'status')
          .map(([k, v]) => `${k}: ${typeof v === 'string' ? v : JSON.stringify(v)}`)
          .join('\n');
        return (
          <div className="bg-black/10 rounded-lg p-2 mt-1 text-[10px] font-mono border-l-2 border-emerald-400 whitespace-pre-wrap">
            {displayFields}
          </div>
        );
      }

      // Error tool result
      if (parsed?.error) {
        return (
          <div className="bg-black/10 rounded-lg p-2 mt-1 text-[10px] font-mono border-l-2 border-red-400">
            {parsed.error as string}
            {typeof parsed.hint === 'string' && <div className="text-content-tertiary mt-1">{parsed.hint}</div>}
          </div>
        );
      }

      // Default text display
      return (
        <div className="bg-black/10 rounded-lg p-2 mt-1 text-[10px] font-mono border-l-2 border-emerald-400">
          {block.text}
        </div>
      );
    }

    case 'audio':
      return (
        <div className="mt-1">
          <audio
            controls
            className="max-w-md"
            src={block.attachment_id ? api.getAttachmentUrl(block.attachment_id) : block.url}
          />
          {block.filename && (
            <div className="text-[9px] font-mono text-content-tertiary mt-0.5">{block.filename}</div>
          )}
        </div>
      );

    case 'file':
      return (
        <a
          href={block.attachment_id ? api.getAttachmentUrl(block.attachment_id) : block.url}
          download={block.filename}
          className="inline-flex items-center gap-1 underline text-[10px] mt-1"
        >
          <Download size={10} />
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
