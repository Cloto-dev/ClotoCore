import { type ReactNode, useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { agentColor } from '../lib/agentIdentity';
import type { AgentMetadata, ContentBlock, McpServerInfo } from '../types';
import { EngineSelector } from './EngineSelector';

interface EditMode {
  messageId: string;
  initialContent: string;
  onCancel: () => void;
}

interface ChatInputBarProps {
  onSend: (blocks: ContentBlock[], rawText: string, engineOverride: string | null) => void;
  /** A reply is being produced: the send button becomes stop. */
  generating?: boolean;
  onStop?: () => void;
  /** Nothing can be sent (the agent is off). */
  disabled?: boolean;
  servers?: McpServerInfo[];
  editMode?: EditMode | null;
  agentId?: string;
  agentName?: string;
  /** Everyone who could be talked to instead; choosing one switches the room. */
  agents?: AgentMetadata[];
  onSwitchAgent?: (agentId: string) => void;
  /** The context meter, drawn at the row's right. */
  meter?: ReactNode;
  /**
   * There is nobody to write to yet. The box stays empty and says nothing: it
   * is the thing a person reaches for, so reaching for it is the way in —
   * pressing it (or typing into it) accepts the invitation. Nothing can be
   * written or sent, and the send button wears no one's colour.
   */
  invitation?: { label: string; onAccept: () => void };
}

interface PendingAttachment {
  file: File;
  preview: string;
  dataUrl: string;
}

/** The mock's textarea grows with its text, to 240px, then scrolls. */
const MAX_TEXTAREA_PX = 240;

/**
 * The composer of docs/gui/samples/02-chat-conversation.html: a box lifted by
 * lightness alone, a textarea that grows, and one row of tools under it.
 */
export function ChatInputBar({
  onSend,
  generating = false,
  onStop,
  disabled = false,
  servers = [],
  editMode,
  agentId,
  agentName,
  agents = [],
  onSwitchAgent,
  meter,
  invitation,
}: ChatInputBarProps) {
  const { t } = useTranslation('agents');
  const [input, setInput] = useState('');
  const [attachment, setAttachment] = useState<PendingAttachment | null>(null);
  const storageKey = agentId ? `cloto-engine-${agentId}` : null;
  const [selectedEngine, setSelectedEngineRaw] = useState<string | null>(() => {
    if (!storageKey) return null;
    return localStorage.getItem(storageKey);
  });
  const setSelectedEngine = (id: string | null) => {
    setSelectedEngineRaw(id);
    if (storageKey) {
      if (id) localStorage.setItem(storageKey, id);
      else localStorage.removeItem(storageKey);
    }
  };
  const fileInputRef = useRef<HTMLInputElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const isComposingRef = useRef(false);
  const [agentMenuOpen, setAgentMenuOpen] = useState(false);
  const agentMenuRef = useRef<HTMLDivElement>(null);

  // Prefill input when entering edit mode. bug-461: depend ONLY on the stable
  // `editMode?.messageId`, not the whole `editMode` object. The parent builds
  // `editMode` as a fresh object literal on every render, so including it here
  // re-ran this effect on any unrelated parent re-render and clobbered the user's
  // in-progress edit back to the original content. Keying on messageId runs the
  // prefill exactly once per edit session, which is the intended behavior.
  useEffect(() => {
    if (editMode) {
      setInput(editMode.initialContent);
      const timerId = setTimeout(() => inputRef.current?.focus(), 50);
      return () => clearTimeout(timerId);
    }
  }, [editMode?.messageId]);

  // Grow with the text: one line at rest, up to the mock's 240px.
  useLayoutEffect(() => {
    const el = inputRef.current;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = `${Math.min(el.scrollHeight, MAX_TEXTAREA_PX)}px`;
  }, [input]);

  useEffect(() => {
    if (!agentMenuOpen) return;
    const onDown = (e: MouseEvent) => {
      if (!agentMenuRef.current?.contains(e.target as Node)) setAgentMenuOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setAgentMenuOpen(false);
    };
    document.addEventListener('mousedown', onDown);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('mousedown', onDown);
      document.removeEventListener('keydown', onKey);
    };
  }, [agentMenuOpen]);

  const canSend = !disabled && !generating && (input.trim().length > 0 || attachment !== null);

  const handleSend = () => {
    if (!canSend) return;

    const blocks: ContentBlock[] = [];

    if (attachment) {
      blocks.push({
        type: 'image',
        url: attachment.dataUrl,
        filename: attachment.file.name,
        mime_type: attachment.file.type,
      });
    }

    if (input.trim()) {
      blocks.push({ type: 'text', text: input.trim() });
    }

    // Fallback to Auto if selected engine is disconnected
    let engine = selectedEngine;
    if (engine) {
      const srv = servers.find((s) => s.id === engine);
      if (srv?.status !== 'Connected') engine = null;
    }

    onSend(blocks, input.trim(), engine);
    setInput('');
    setAttachment(null);
  };

  const handlePaste = useCallback((e: React.ClipboardEvent) => {
    const items = e.clipboardData?.items;
    if (!items) return;

    for (const item of items) {
      if (item.type.startsWith('image/')) {
        e.preventDefault();
        const file = item.getAsFile();
        if (!file) continue;

        const reader = new FileReader();
        reader.onload = () => {
          const dataUrl = reader.result as string;
          setAttachment({ file, preview: dataUrl, dataUrl });
        };
        reader.readAsDataURL(file);
        break;
      }
    }
  }, []);

  const handleFileSelect = useCallback(() => {
    fileInputRef.current?.click();
  }, []);

  const handleBrowserFileChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;

    const reader = new FileReader();
    reader.onload = () => {
      const dataUrl = reader.result as string;
      setAttachment({ file, preview: dataUrl, dataUrl });
    };
    reader.readAsDataURL(file);

    e.target.value = '';
  };

  const placeholder = editMode
    ? t('chat_input.placeholder_edit')
    : disabled
      ? t('chat_input.placeholder_offline')
      : t('chat_input.placeholder', { name: agentName ?? '' });

  return (
    <div className="write">
      <div className="col">
        {/* The textarea inside carries the role and the keyboard path; the click
            here only widens the pointer target to the whole box. */}
        <div className={`box${invitation ? ' inviting' : ''}`} onClick={invitation ? invitation.onAccept : undefined}>
          {attachment && (
            <div className="attach">
              <img src={attachment.preview} alt="" />
              <span className="t">{attachment.file.name}</span>
              <button type="button" onClick={() => setAttachment(null)} aria-label={t('chat_input.remove_attachment')}>
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round">
                  <path d="M6 6l12 12M18 6 6 18" />
                </svg>
              </button>
            </div>
          )}
          <textarea
            ref={inputRef}
            rows={1}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onCompositionStart={() => {
              isComposingRef.current = true;
            }}
            onCompositionEnd={() => {
              isComposingRef.current = false;
            }}
            onKeyDown={(e) => {
              if (invitation) {
                // Anything that would have written something accepts instead.
                if (e.key === 'Enter' || e.key === ' ' || (e.key.length === 1 && !e.metaKey && !e.ctrlKey)) {
                  e.preventDefault();
                  invitation.onAccept();
                }
                return;
              }
              // Enter sends; Shift+Enter breaks the line; an Enter that ends
              // IME composition is neither.
              if (
                e.key === 'Enter' &&
                !e.shiftKey &&
                !e.nativeEvent.isComposing &&
                !isComposingRef.current &&
                e.keyCode !== 229
              ) {
                e.preventDefault();
                handleSend();
              }
              if (e.key === 'Escape' && editMode) editMode.onCancel();
            }}
            onPaste={handlePaste}
            disabled={disabled && !invitation}
            readOnly={!!invitation}
            placeholder={invitation ? '' : placeholder}
            aria-label={invitation ? invitation.label : placeholder}
          />
          <div className="row">
            {/* Nothing can be attached to nobody: the box is empty. */}
            {!invitation && (
              <>
                <button
                  type="button"
                  className="ib"
                  onClick={handleFileSelect}
                  disabled={disabled}
                  title={t('chat_input.attach_image')}
                  aria-label={t('chat_input.attach_image')}
                >
                  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round">
                    <path d="M12 5v14M5 12h14" />
                  </svg>
                </button>
                <span className="sep" />
              </>
            )}
            {agentName && (
              <div className="menu-anchor" ref={agentMenuRef}>
                <button
                  type="button"
                  className="ctx"
                  onClick={() => setAgentMenuOpen((v) => !v)}
                  title={t('chat_input.switch_agent')}
                  aria-label={t('chat_input.switch_agent')}
                  aria-haspopup="menu"
                  aria-expanded={agentMenuOpen}
                >
                  <span className="dot" />
                  {agentName}
                </button>
                {agentMenuOpen && (
                  <div className="menu up" role="menu">
                    {agents.map((a) => (
                      <button
                        type="button"
                        key={a.id}
                        role="menuitem"
                        className={a.id === agentId ? 'on' : ''}
                        onClick={() => {
                          setAgentMenuOpen(false);
                          if (a.id !== agentId) onSwitchAgent?.(a.id);
                        }}
                      >
                        <span className="dot" style={{ background: agentColor(a) }} />
                        {a.name}
                      </button>
                    ))}
                  </div>
                )}
              </div>
            )}
            {/* An engine is somebody's; with nobody there is none to choose. */}
            {!invitation && (
              <EngineSelector
                servers={servers}
                selectedEngine={selectedEngine}
                onSelect={setSelectedEngine}
                disabled={disabled}
              />
            )}
            {editMode && (
              <button type="button" className="ctx" onClick={editMode.onCancel}>
                {t('chat_input.cancel_edit')}
              </button>
            )}
            <span className="spacer" />
            {meter}
            {generating ? (
              <button
                type="button"
                className="send stop"
                onClick={onStop}
                title={t('chat_input.stop')}
                aria-label={t('chat_input.stop')}
              >
                <svg viewBox="0 0 24 24" fill="currentColor">
                  <rect x="7" y="7" width="10" height="10" rx="1.5" />
                </svg>
              </button>
            ) : (
              <button
                type="button"
                className={`send${invitation ? ' nobody' : ''}`}
                onClick={handleSend}
                disabled={!canSend || !!invitation}
                title={t('chat_input.send')}
                aria-label={t('chat_input.send')}
              >
                <svg
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2.4"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <path d="M12 19V5M5 12l7-7 7 7" />
                </svg>
              </button>
            )}
          </div>
        </div>
        {/* Kept in the layout when it says nothing, so the box does not move
            as the faces turn. */}
        <div
          className="hint"
          style={invitation ? { visibility: 'hidden' } : undefined}
          aria-hidden={invitation ? true : undefined}
        >
          {t('chat_input.hint')}
        </div>
      </div>

      {/* Hidden file input for browser mode */}
      <input
        ref={fileInputRef}
        type="file"
        accept="image/png,image/jpeg,image/gif,image/webp"
        onChange={handleBrowserFileChange}
        className="hidden"
      />
    </div>
  );
}
