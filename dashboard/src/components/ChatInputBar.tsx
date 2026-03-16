import { Mic, MicOff, Plus, Send, X } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { ContentBlock, McpServerInfo } from '../types';
import { EngineSelector } from './EngineSelector';

interface EditMode {
  messageId: string;
  initialContent: string;
  onCancel: () => void;
}

interface ChatInputBarProps {
  onSend: (blocks: ContentBlock[], rawText: string, engineOverride: string | null) => void;
  disabled: boolean;
  servers?: McpServerInfo[];
  editMode?: EditMode | null;
  agentId?: string;
}

interface PendingAttachment {
  file: File;
  preview: string;
  dataUrl: string;
}

export function ChatInputBar({ onSend, disabled, servers = [], editMode, agentId }: ChatInputBarProps) {
  const { t } = useTranslation('agents');
  const [input, setInput] = useState('');
  const [attachment, setAttachment] = useState<PendingAttachment | null>(null);
  const [isRecording, setIsRecording] = useState(false);
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
  const mediaRecorderRef = useRef<MediaRecorder | null>(null);
  const chunksRef = useRef<Blob[]>([]);
  const inputRef = useRef<HTMLInputElement>(null);

  // Prefill input when entering edit mode
  useEffect(() => {
    if (editMode) {
      setInput(editMode.initialContent);
      const timerId = setTimeout(() => inputRef.current?.focus(), 50);
      return () => clearTimeout(timerId);
    }
  }, [editMode?.messageId, editMode]);

  const handleSend = () => {
    if ((!input.trim() && !attachment) || disabled) return;

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
      if (!srv || srv.status !== 'Connected') engine = null;
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

  const toggleRecording = async () => {
    if (isRecording) {
      mediaRecorderRef.current?.stop();
      setIsRecording(false);
      return;
    }

    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      const recorder = new MediaRecorder(stream, { mimeType: 'audio/webm' });
      chunksRef.current = [];

      recorder.ondataavailable = (e) => {
        if (e.data.size > 0) chunksRef.current.push(e.data);
      };

      recorder.onstop = () => {
        stream.getTracks().forEach((t) => t.stop());
        if (chunksRef.current.length > 0) {
          // Insert a note that audio was recorded — agent will use STT
          const audioDuration = Math.round(chunksRef.current.reduce((acc, c) => acc + c.size, 0) / 16000);
          setInput((prev) => prev + (prev ? ' ' : '') + `[Audio recorded: ~${audioDuration}s — please transcribe]`);
        }
      };

      mediaRecorderRef.current = recorder;
      recorder.start();
      setIsRecording(true);
    } catch (err) {
      console.error('Microphone access denied:', err);
    }
  };

  return (
    <div className="p-4 bg-glass-strong border-t border-edge-subtle">
      {/* Attachment preview */}
      {attachment && (
        <div className="mb-2 flex items-center gap-2">
          <img
            src={attachment.preview}
            alt="preview"
            className="w-16 h-16 object-cover rounded-lg border border-edge"
          />
          <div className="flex-1 text-[10px] font-mono text-content-secondary truncate">{attachment.file.name}</div>
          <button
            onClick={() => setAttachment(null)}
            className="p-1 rounded hover:bg-glass text-content-tertiary hover:text-red-400 transition-colors"
          >
            <X size={14} />
          </button>
        </div>
      )}

      {/* Input row */}
      <div className="relative flex items-center gap-1">
        {/* Attach button */}
        <button
          onClick={handleFileSelect}
          disabled={disabled}
          className="p-2 rounded-lg text-content-tertiary hover:text-brand hover:bg-glass transition-colors disabled:opacity-30"
          title={t('chat_input.attach_image')}
        >
          <Plus size={16} />
        </button>

        {/* Mic button */}
        <button
          onClick={toggleRecording}
          disabled={disabled}
          className={`p-2 rounded-lg transition-colors disabled:opacity-30 ${
            isRecording
              ? 'text-red-500 bg-red-500/10 animate-pulse'
              : 'text-content-tertiary hover:text-brand hover:bg-glass'
          }`}
          title={isRecording ? t('chat_input.stop_recording') : t('chat_input.record_audio')}
        >
          {isRecording ? <MicOff size={16} /> : <Mic size={16} />}
        </button>

        {/* Engine selector */}
        <EngineSelector
          servers={servers}
          selectedEngine={selectedEngine}
          onSelect={setSelectedEngine}
          disabled={disabled}
        />

        {/* Text input */}
        <input
          ref={inputRef}
          type="text"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') handleSend();
            if (e.key === 'Escape' && editMode) editMode.onCancel();
          }}
          onPaste={handlePaste}
          disabled={disabled}
          placeholder={
            editMode
              ? t('chat_input.placeholder_edit')
              : disabled
                ? t('chat_input.placeholder_processing')
                : t('chat_input.placeholder')
          }
          className="flex-1 bg-surface-primary border rounded-xl py-3 px-4 pr-12 text-xs font-mono focus:outline-none transition-colors placeholder:text-content-tertiary disabled:opacity-50 shadow-inner border-edge focus:border-brand"
          style={editMode ? { borderColor: '#fbbf24' } : undefined}
        />

        {/* Cancel edit button */}
        {editMode && (
          <button
            onClick={editMode.onCancel}
            className="absolute right-12 p-1.5 text-content-tertiary hover:text-red-400 transition-colors"
            title={t('chat_input.cancel_edit')}
          >
            <X size={14} />
          </button>
        )}

        {/* Send button */}
        <button
          onClick={handleSend}
          disabled={disabled || (!input.trim() && !attachment)}
          className="absolute right-2 p-2 bg-brand text-white rounded-lg hover:scale-105 active:scale-95 transition-all disabled:opacity-30 disabled:grayscale disabled:scale-100 shadow-lg shadow-brand/20"
        >
          <Send size={16} />
        </button>
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
