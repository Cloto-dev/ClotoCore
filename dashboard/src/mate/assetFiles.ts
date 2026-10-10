import { AVATAR_MAX_BYTES } from '../lib/agentIdentity';

// HARDCODED(crates/core/src/handlers/agents.rs::VRM_MAX_BYTES): reject oversized files before transmitting them.
export const VRM_MAX_BYTES = 50 * 1024 * 1024;
export const MOTION_MAX_BYTES = VRM_MAX_BYTES;
export type AssetProblem = 'format' | 'external' | 'size';
export class AssetFileError extends Error {
  constructor(public readonly code: AssetProblem) {
    super(code);
  }
}

/** Local models and motions must be self-contained GLBs; loading never follows a remote resource URI. */
export function readGlbDocument(bytes: ArrayBuffer, kind: 'vrm' | 'motion') {
  if (bytes.byteLength < 20) throw new AssetFileError('format');
  const view = new DataView(bytes);
  if (
    view.getUint32(0, true) !== 0x46546c67 ||
    view.getUint32(4, true) !== 2 ||
    view.getUint32(8, true) !== bytes.byteLength ||
    view.getUint32(16, true) !== 0x4e4f534a
  )
    throw new AssetFileError('format');
  const length = view.getUint32(12, true);
  if (length > bytes.byteLength - 20) throw new AssetFileError('format');
  let doc: { extensions?: Record<string, unknown>; buffers?: { uri?: string }[]; images?: { uri?: string }[] };
  try {
    doc = JSON.parse(
      new TextDecoder()
        .decode(new Uint8Array(bytes, 20, length))
        .split(String.fromCharCode(0))
        .join(''),
    );
  } catch {
    throw new AssetFileError('format');
  }
  if (!(kind === 'vrm' ? doc.extensions?.VRM || doc.extensions?.VRMC_vrm : doc.extensions?.VRMC_vrm_animation))
    throw new AssetFileError('format');
  if (
    [...(doc.buffers ?? []), ...(doc.images ?? [])].some((r: { uri?: string }) => r.uri && !r.uri.startsWith('data:'))
  )
    throw new AssetFileError('external');
  return doc;
}

export async function validateModelFile(file: File, kind: 'vrm' | 'motion') {
  if (file.size > (kind === 'vrm' ? VRM_MAX_BYTES : MOTION_MAX_BYTES)) throw new AssetFileError('size');
  readGlbDocument(await file.arrayBuffer(), kind);
}

export async function validateIconFile(file: File) {
  if (file.size > AVATAR_MAX_BYTES) throw new AssetFileError('size');
  if (!['image/png', 'image/jpeg', 'image/webp'].includes(file.type)) throw new AssetFileError('format');
  const url = URL.createObjectURL(file);
  try {
    const img = new Image();
    img.src = url;
    await img.decode();
    if (!img.naturalWidth || !img.naturalHeight || Math.max(img.naturalWidth, img.naturalHeight) > 8192)
      throw new AssetFileError('format');
  } finally {
    URL.revokeObjectURL(url);
  }
}
