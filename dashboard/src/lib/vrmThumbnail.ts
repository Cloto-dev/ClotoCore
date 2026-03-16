/**
 * Extract thumbnail image from a VRM file (GLB/glTF-binary format).
 * Supports both VRM 0.x and VRM 1.0 thumbnail references.
 *
 * @returns A File containing the thumbnail image, or null if no thumbnail found.
 */
export async function extractVrmThumbnail(file: File): Promise<File | null> {
  const buffer = await file.arrayBuffer();
  const view = new DataView(buffer);

  // GLB Header: magic(4) + version(4) + length(4) = 12 bytes
  if (buffer.byteLength < 12) return null;
  const magic = view.getUint32(0, true);
  if (magic !== 0x46546c67) return null; // "glTF"

  // Parse chunks
  let jsonChunk: string | null = null;
  let binOffset = 0;
  let binLength = 0;
  let offset = 12;

  while (offset < buffer.byteLength) {
    if (offset + 8 > buffer.byteLength) break;
    const chunkLength = view.getUint32(offset, true);
    const chunkType = view.getUint32(offset + 4, true);

    if (chunkType === 0x4e4f534a) {
      // JSON chunk
      const decoder = new TextDecoder();
      jsonChunk = decoder.decode(new Uint8Array(buffer, offset + 8, chunkLength));
    } else if (chunkType === 0x004e4942) {
      // BIN chunk
      binOffset = offset + 8;
      binLength = chunkLength;
    }

    offset += 8 + chunkLength;
    // Chunks are 4-byte aligned
    if (offset % 4 !== 0) offset += 4 - (offset % 4);
  }

  if (!jsonChunk || binLength === 0) return null;

  const gltf = JSON.parse(jsonChunk);
  const images: Array<{ bufferView?: number; mimeType?: string }> = gltf.images ?? [];
  const bufferViews: Array<{ byteOffset?: number; byteLength?: number }> = gltf.bufferViews ?? [];
  const textures: Array<{ source?: number }> = gltf.textures ?? [];

  // Find thumbnail image index
  let imageIndex: number | null = null;

  // VRM 1.0: extensions.VRMC_vrm.meta.thumbnailImage → image index
  const vrmc = gltf.extensions?.VRMC_vrm;
  if (vrmc?.meta?.thumbnailImage != null) {
    imageIndex = vrmc.meta.thumbnailImage;
  }

  // VRM 0.x: extensions.VRM.meta.texture → texture index → textures[idx].source → image index
  if (imageIndex == null) {
    const vrm0 = gltf.extensions?.VRM;
    if (vrm0?.meta?.texture != null) {
      const texIdx = vrm0.meta.texture;
      if (textures[texIdx]?.source != null) {
        imageIndex = textures[texIdx].source!;
      }
    }
  }

  if (imageIndex == null || !images[imageIndex]) return null;

  const image = images[imageIndex];
  if (image.bufferView == null) return null;

  const bv = bufferViews[image.bufferView];
  if (!bv || bv.byteLength == null) return null;

  const imgOffset = binOffset + (bv.byteOffset ?? 0);
  if (imgOffset + bv.byteLength > buffer.byteLength) return null;

  const imgBytes = new Uint8Array(buffer, imgOffset, bv.byteLength);
  const mimeType = image.mimeType ?? 'image/png';
  const ext = mimeType === 'image/jpeg' ? 'jpg' : 'png';

  return new File([imgBytes], `vrm-thumbnail.${ext}`, { type: mimeType });
}
