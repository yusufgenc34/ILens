'use client'
// Read-only, bounded ZIP32 reader for selected NuGet DLL entries. No extraction
// to a filesystem, package installation, build scripts or managed execution.
import {crc32} from '../lib/zip'
export interface PackageEntry {name:string; dataLimit:number; offset:number; compressed:number; size:number; method:number; crc:number; flags:number}
const maxPackage = 32 * 1024 * 1024
const maxEntry = 8 * 1024 * 1024
export async function readBounded(stream: ReadableStream<Uint8Array>, limit: number, signal: AbortSignal): Promise<Uint8Array<ArrayBuffer>> {
  const reader = stream.getReader(), chunks: Uint8Array[] = []; let size = 0
  try {
    for (;;) {
      signal.throwIfAborted()
      const {done,value} = await reader.read()
      signal.throwIfAborted()
      if (done) break
      size += value.length
      if (size > limit) throw new Error('Dependency package exceeds the download or extraction limit.')
      chunks.push(value)
    }
  } catch (error) {await reader.cancel().catch(() => {}); throw error}
  finally {reader.releaseLock()}
  const output = new Uint8Array(size); let offset = 0
  for (const chunk of chunks) {output.set(chunk,offset); offset += chunk.length}
  return output
}
export function packageEntries(bytes: Uint8Array): PackageEntry[] {
  if (bytes.length > maxPackage) throw new Error('NuGet package exceeds 32 MiB.')
  const data = new DataView(bytes.buffer,bytes.byteOffset,bytes.byteLength)
  const span = (p:number,n:number) => {if (!Number.isSafeInteger(p) || !Number.isSafeInteger(n) || p < 0 || n < 0 || p+n > bytes.length) throw new Error('Truncated NuGet ZIP.'); return p}
  const u16 = (p:number) => data.getUint16(span(p,2),true), u32 = (p:number) => data.getUint32(span(p,4),true)
  let end = -1
  for (let p = bytes.length-22; p >= Math.max(0,bytes.length-65557); p--) if (u32(p) === 0x06054b50 && p+22+u16(p+20) === bytes.length) {end=p; break}
  if (end < 0 || u16(end+4) || u16(end+6) || u16(end+8) !== u16(end+10)) throw new Error('Unsupported NuGet ZIP directory.')
  const count = u16(end+10), directory = u32(end+16), directorySize = u32(end+12)
  if (count > 10000 || directory+directorySize !== end) throw new Error('Invalid or excessive NuGet ZIP directory.')
  span(directory,directorySize)
  const entries: PackageEntry[] = [], names = new Set<string>(), decoder = new TextDecoder('utf-8',{fatal:true})
  let cursor = directory
  for (let i=0; i<count; i++) {
    span(cursor,46)
    if (u32(cursor) !== 0x02014b50) throw new Error('Invalid NuGet ZIP record.')
    const length = u16(cursor+28), extra = u16(cursor+30), comment = u16(cursor+32), next = cursor+46+length+extra+comment
    if (length > 1024 || next > end || u16(cursor+34)) throw new Error('Unsupported NuGet ZIP entry.')
    span(cursor+46,length)
    const name = decoder.decode(bytes.subarray(cursor+46,cursor+46+length))
    const key = name.toLowerCase()
    if (names.has(key) || name.startsWith('/') || name.includes('\\') || name.includes(':') || [...name].some(c=>c.charCodeAt(0)<32) || name.split('/').some(p => p === '..' || p === '.')) throw new Error('Ambiguous or unsafe NuGet entry name.')
    names.add(key)
    const entry = {name,dataLimit:directory,offset:u32(cursor+42),compressed:u32(cursor+20),size:u32(cursor+24),method:u16(cursor+10),crc:u32(cursor+16),flags:u16(cursor+8)}
    if ([entry.offset,entry.compressed,entry.size].includes(0xffffffff)) throw new Error('ZIP64 is not supported.')
    if (entry.offset >= directory) throw new Error('Invalid ZIP local offset.')
    entries.push(entry); cursor=next
  }
  if (cursor !== end) throw new Error('Invalid NuGet ZIP directory length.')
  return entries
}
export async function packageEntry(bytes: Uint8Array<ArrayBuffer>, entry: PackageEntry, signal: AbortSignal): Promise<Uint8Array<ArrayBuffer>> {
  signal.throwIfAborted()
  if (entry.size > maxEntry || entry.compressed > maxPackage || ![0,8].includes(entry.method) || entry.flags & ~0x80e) throw new Error('Unsupported, encrypted or excessive dependency entry.')
  const data = new DataView(bytes.buffer,bytes.byteOffset,bytes.byteLength), p = entry.offset
  if (p < 0 || p+30 > entry.dataLimit || entry.dataLimit > bytes.length || data.getUint32(p,true) !== 0x04034b50) throw new Error('Invalid ZIP local header.')
  const flags=data.getUint16(p+6,true), method=data.getUint16(p+8,true), length=data.getUint16(p+26,true), extra=data.getUint16(p+28,true), start=p+30+length+extra
  if (start+entry.compressed > entry.dataLimit || flags !== entry.flags || method !== entry.method || new TextDecoder('utf-8',{fatal:true}).decode(bytes.subarray(p+30,p+30+length)) !== entry.name) throw new Error('Inconsistent ZIP headers.')
  if (!(flags & 8) && (data.getUint32(p+14,true) !== entry.crc || data.getUint32(p+18,true) !== entry.compressed || data.getUint32(p+22,true) !== entry.size)) throw new Error('Inconsistent ZIP entry sizes or checksum.')
  const input = bytes.slice(start,start+entry.compressed)
  const output = method === 0 ? input : await readBounded(new Blob([input]).stream().pipeThrough(new DecompressionStream('deflate-raw')),Math.min(maxEntry,entry.size),signal)
  if (output.length !== entry.size || crc32(output) !== entry.crc) throw new Error('Dependency entry failed ZIP size/CRC validation.')
  return output
}
