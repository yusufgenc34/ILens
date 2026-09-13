import {describe, it, expect} from 'vitest'
import {execFileSync} from 'node:child_process'
import {compilationUnit, inferFramework, methodDownload, renderType, safeName, xml, type ExportMember, type ExportType} from '../src/lib/export'
import {crc32, createZipArchive} from '../src/lib/zip'
import type {Declaration, MethodAnalysis} from '../src/lib/types'
const method: MethodAnalysis = {token: 0x06000001, csharp: 'public long Limit() { return 9223372036854775807L; }', il: 'IL_0000: ldc.i8 9223372036854775807\nIL_0009: ret', quality: 'csharp_like', diagnostics: [], body: null, cfg: null, stack: null}
const member = {token: method.token, name: '../CON'} as Declaration

describe('source downloads', () => {
  it('preserves exact method contents and refuses IL mislabeled as C# or stale selection', () => {
    const file = methodDownload(method, member, 'C#')
    expect(file.name).toBe('_CON_06000001.cs'); expect(new TextDecoder().decode(file.buffer)).toContain('9223372036854775807L')
    expect(() => methodDownload({...method, quality: 'annotated_il'}, member, 'C#')).toThrow('unavailable')
    expect(() => methodDownload(method, {...member, token: 2}, 'C#')).toThrow('changed')
  })
  it('uses safe paths and exact allowlisted project templates', () => {
    expect(safeName('../../hello\\world:C$()', 1)).not.toMatch(/[./\\:$()]/)
    expect(xml('$(Run);@x% & <')).toBe('%24%28Run%29%3B%40x%25 &amp; &lt;')
    expect(inferFramework('v4.0.30319')).toBe(''); expect(inferFramework('.NETCoreApp,Version=v10.0')).toBe('net10.0')
  })
  it('owns accessors exactly once and marks missing bodies as partial', () => {
    const type: ExportType = {token: 0x02000001, parent: 0, name: 'Sample', namespace: 'Example', kind: 'class', declaration: 'public class Sample', members: [0x17000001, 0x06000001], diagnostics: []}
    const property = {token: 0x17000001, kind: 'property', declaration: 'public int Value', name: 'Value', il: '', metadata: {}, body: null, quality: 'declaration', diagnostics: [], accessors: [['get', 0x06000001]]} as ExportMember
    const getter = {token: 0x06000001, name: 'get_Value', kind: 'method', il: '', metadata: {}, declaration: 'public int get_Value()', body: '{\n    return 1;\n}', quality: 'csharp_like', diagnostics: [], accessors: []} as ExportMember
    const result = renderType(type, [type], new Map([[property.token, property], [getter.token, getter]]))
    expect(result.text).toContain('get\n'); expect(result.text).not.toContain('get_Value'); expect(result.partial).toBe(false)
    expect(compilationUnit(type, result.text, false)).toContain('namespace @Example')
    expect(renderType(type, [type], new Map([[property.token, property]])).partial).toBe(true)
  })
})
describe('browser ZIP writer', () => {
  it('produces archives accepted by an independent reader with neutral timestamps and correct CRC', () => {
    expect(crc32(new TextEncoder().encode('123456789'))).toBe(0xcbf43926)
    const zip = createZipArchive(); zip.add('src/Example.cs', 'hello λ\n'); zip.add('resources/data.bin', new Uint8Array([0, 255, 1]))
    const archive = zip.finish()
    const result = execFileSync('python3', ['-c', 'import sys,io,zipfile,json; z=zipfile.ZipFile(io.BytesIO(sys.stdin.buffer.read())); assert z.testzip() is None; assert all(i.date_time==(1980,1,1,0,0,0) and not i.extra and not i.comment for i in z.infolist()); print(json.dumps(z.read("src/Example.cs").decode()))'], {input: Buffer.from(archive), encoding: 'utf8'})
    expect(JSON.parse(result)).toBe('hello λ\n')
  })
  it('rejects traversal, duplicate case variants, reserved names and oversized entries', () => {
    for (const path of ['../bad', '/bad', 'C:/bad', 'src/CON.txt', 'src/../../bad', 'a\\b']) expect(() => createZipArchive().add(path, '')).toThrow()
    const zip = createZipArchive(); zip.add('a.cs', ''); expect(() => zip.add('A.cs', '')).toThrow('Duplicate')
    expect(() => createZipArchive().add('big.bin', new Uint8Array(4 * 1024 * 1024 + 1))).toThrow('4 MiB')
  })
})
