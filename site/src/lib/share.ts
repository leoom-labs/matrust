export const MAX_CODE_BYTES = 65536;
const encoder = new TextEncoder();
const decoder = new TextDecoder('utf-8', { fatal: true });
function base64url(bytes: Uint8Array): string {
  let binary = '';
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}
function unbase64url(text: string): Uint8Array {
  if (!/^[A-Za-z0-9_-]*$/.test(text)) throw new Error('Invalid share link.');
  return Uint8Array.from(atob(text.replace(/-/g, '+').replace(/_/g, '/')), char => char.charCodeAt(0));
}
async function boundedBytes(stream: ReadableStream<Uint8Array>): Promise<Uint8Array> {
  const reader = stream.getReader();
  const chunks: Uint8Array[] = [];
  let length = 0;
  try {
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      length += value.length;
      if (length > MAX_CODE_BYTES) throw new Error('Shared code exceeds the 64 KB limit.');
      chunks.push(value);
    }
  } catch (error) { await reader.cancel(); throw error; }
  const bytes = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.length; }
  return bytes;
}
export async function encodeCode(code: string): Promise<string> {
  const bytes = encoder.encode(code);
  if (bytes.length > MAX_CODE_BYTES) throw new Error('Code exceeds the 64 KB sharing limit.');
  if (typeof CompressionStream !== 'undefined') {
    const compressed = await boundedBytes(new Blob([bytes]).stream().pipeThrough(new CompressionStream('deflate')));
    if (compressed.length < bytes.length) return `v1z.${base64url(compressed)}`;
  }
  return `v1.${base64url(bytes)}`;
}
export async function decodeCode(value: string): Promise<string> {
  if (value.length > MAX_CODE_BYTES * 2) throw new Error('Share link is too large.');
  const [version, payload, extra] = value.split('.');
  if (extra !== undefined || !payload && payload !== '') throw new Error('Invalid share link.');
  const bytes = unbase64url(payload);
  if (version === 'v1') {
    if (bytes.length > MAX_CODE_BYTES) throw new Error('Shared code exceeds the 64 KB limit.');
    return decoder.decode(bytes);
  }
  if (version === 'v1z') {
    if (typeof DecompressionStream === 'undefined') throw new Error('This browser cannot open compressed links. Open it in a newer browser.');
    return decoder.decode(await boundedBytes(new Blob([bytes as Uint8Array<ArrayBuffer>]).stream().pipeThrough(new DecompressionStream('deflate'))));
  }
  throw new Error('Unsupported share-link version.');
}
// Build-time links use the universally supported uncompressed version.
export function plainCodeHash(code: string): string { return `#code=v1.${base64url(encoder.encode(code))}`; }
