import { describe, expect, it, vi } from 'vitest';
import { AssetFileError, readGlbDocument, VRM_MAX_BYTES, validateModelFile } from './assetFiles';

function glb(doc: unknown) {
  const json = new TextEncoder().encode(JSON.stringify(doc));
  const length = Math.ceil(json.length / 4) * 4;
  const buffer = new ArrayBuffer(20 + length);
  const view = new DataView(buffer);
  [0x46546c67, 2, buffer.byteLength, length, 0x4e4f534a].forEach((n, i) => view.setUint32(i * 4, n, true));
  new Uint8Array(buffer, 20).fill(32);
  new Uint8Array(buffer, 20, json.length).set(json);
  return buffer;
}
describe('partner asset preflight', () => {
  it('accepts both VRM formats and distinguishes motions from models', () => {
    expect(readGlbDocument(glb({ extensions: { VRM: {} } }), 'vrm').extensions).toHaveProperty('VRM');
    expect(readGlbDocument(glb({ extensions: { VRMC_vrm: {} } }), 'vrm').extensions).toHaveProperty('VRMC_vrm');
    const motion = glb({ extensions: { VRMC_vrm_animation: {} } });
    expect(readGlbDocument(motion, 'motion').extensions).toHaveProperty('VRMC_vrm_animation');
    expect(() => readGlbDocument(motion, 'vrm')).toThrow(AssetFileError);
  });
  it('rejects external resources and truncated GLBs before the loader can fetch them', () => {
    expect(() =>
      readGlbDocument(glb({ extensions: { VRMC_vrm: {} }, images: [{ uri: 'https://example.com/image.png' }] }), 'vrm'),
    ).toThrow('external');
    const bytes = glb({ extensions: { VRMC_vrm: {} } });
    expect(() => readGlbDocument(bytes.slice(0, bytes.byteLength - 4), 'vrm')).toThrow('format');
  });
  it('rejects oversized files without reading or transmitting their bytes', async () => {
    const arrayBuffer = vi.fn();
    await expect(validateModelFile({ size: VRM_MAX_BYTES + 1, arrayBuffer } as unknown as File, 'vrm')).rejects.toThrow(
      'size',
    );
    expect(arrayBuffer).not.toHaveBeenCalled();
  });
});
