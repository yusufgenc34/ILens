// Bounded ZIP32 STORE writer, following PKWARE APPNOTE 4.3. No dependencies,
// compression threads, filesystem, ZIP64, encryption, comments or host metadata.
const encoder = new TextEncoder()
const table = new Uint32Array(256)
for (let n = 0; n < 256; n++) {let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; table[n] = c >>> 0}
export function crc32(bytes: Uint8Array): number {let crc = 0xffffffff; for (const b of bytes) crc = table[(crc ^ b) & 255] ^ (crc >>> 8); return (crc ^ 0xffffffff) >>> 0}
interface Entry {name: Uint8Array; data: Uint8Array; crc: number; offset: number}
class ZipArchive {
  private entries: Entry[] = []
  private names = new Set<string>()
  private length = 0
  private directoryLength = 0
  add(path: string, input: string | Uint8Array) {
    if (!/^[a-zA-Z0-9_./-]+$/.test(path) || path.startsWith('/') || path.split('/').some(p => !p || p === '.' || p === '..' || /^(con|prn|aux|nul|com[0-9]|lpt[0-9])(?:\.|$)/i.test(p))) throw new Error('Unsafe archive entry path.')
    const key = path.toLowerCase(); if (this.names.has(key)) throw new Error('Duplicate archive entry path.')
    const data = typeof input === 'string' ? encoder.encode(input) : input
    const name = encoder.encode(path)
    if (data.byteLength > 4 * 1024 * 1024 || name.length > 1024 || this.entries.length >= 25_000) throw new Error('Export entry exceeds the 4 MiB, path-length or entry-count limit.')
    const size = this.length + 30 + name.length + data.byteLength
    const central = this.directoryLength + 46 + name.length
    if (size + central + 22 > 64 * 1024 * 1024) throw new Error('The source archive exceeds 64 MiB. Export individual types instead.')
    this.entries.push({name, data, crc: crc32(data), offset: this.length}); this.length = size; this.directoryLength = central; this.names.add(key)
  }
  finish(): ArrayBuffer {
    const output = new Uint8Array(this.length + this.directoryLength + 22); const view = new DataView(output.buffer)
    const u16 = (p: number, v: number) => view.setUint16(p, v, true); const u32 = (p: number, v: number) => view.setUint32(p, v, true)
    let directory = this.length
    for (const e of this.entries) {
      const p = e.offset; u32(p, 0x04034b50); u16(p + 4, 20); u16(p + 6, 0x800); u16(p + 12, 33) // 1980-01-01, 00:00
      u32(p + 14, e.crc); u32(p + 18, e.data.length); u32(p + 22, e.data.length); u16(p + 26, e.name.length)
      output.set(e.name, p + 30); output.set(e.data, p + 30 + e.name.length)
      const c = directory; u32(c, 0x02014b50); u16(c + 4, 20); u16(c + 6, 20); u16(c + 8, 0x800); u16(c + 14, 33)
      u32(c + 16, e.crc); u32(c + 20, e.data.length); u32(c + 24, e.data.length); u16(c + 28, e.name.length); u32(c + 42, p)
      output.set(e.name, c + 46); directory += 46 + e.name.length
    }
    u32(directory, 0x06054b50); u16(directory + 8, this.entries.length); u16(directory + 10, this.entries.length); u32(directory + 12, this.directoryLength); u32(directory + 16, this.length)
    this.clear(); return output.buffer
  }
  clear() {this.entries = []; this.names.clear(); this.length = 0; this.directoryLength = 0}
}

export function createZipArchive() {return new ZipArchive()}
