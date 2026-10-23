import {describe,expect,it,vi} from 'vitest'
import {deflateRawSync} from 'node:zlib'
import {frameworkPackage} from '../src/lib/framework-catalog'
import {packageEntries,packageEntry,readBounded} from '../src/workers/nuget-package'
import {createFrameworkResolver} from '../src/workers/framework-resolver'
import {createZipArchive,crc32} from '../src/lib/zip'
import {createWorkspaceTree,pathToToken} from '../src/lib/tree'
import type {AssemblyInfo,Identity,Loaded,Overview} from '../src/lib/types'
const identity:Identity={name:'System.Runtime',version:'10.0.0.0',culture:'neutral',public_key_token:'b03f5f7f11d50a3a'}
const info=(moniker='.NETCoreApp,Version=v10.0')=>({identity,target_framework:{moniker,status:'declared'}} as AssemblyInfo)
const signal=()=>new AbortController().signal
function stored(name='ref/net10.0/System.Runtime.dll') {const zip=createZipArchive();zip.add(name,'example bytes');return new Uint8Array(zip.finish())}
function deflated(payload:Uint8Array) {
  const name=new TextEncoder().encode('ref/net10.0/System.Runtime.dll'),compressed=deflateRawSync(payload),directory=30+name.length+compressed.length,end=directory+46+name.length
  const bytes=new Uint8Array(end+22),v=new DataView(bytes.buffer)
  const u16=(p:number,n:number)=>v.setUint16(p,n,true),u32=(p:number,n:number)=>v.setUint32(p,n,true)
  u32(0,0x04034b50);u16(4,20);u16(8,8);u32(14,crc32(payload));u32(18,compressed.length);u32(22,payload.length);u16(26,name.length);bytes.set(name,30);bytes.set(compressed,30+name.length)
  u32(directory,0x02014b50);u16(directory+6,20);u16(directory+10,8);u32(directory+16,crc32(payload));u32(directory+20,compressed.length);u32(directory+24,payload.length);u16(directory+28,name.length);bytes.set(name,directory+46)
  u32(end,0x06054b50);u16(end+8,1);u16(end+10,1);u32(end+12,end-directory);u32(end+16,directory)
  return bytes
}
describe('bounded reference package reader',()=>{
  it('extracts STORE and DEFLATE payloads and validates CRC',async()=>{
    for (const bytes of [stored(),deflated(new TextEncoder().encode('actual deflate bytes'))]) {
      const [entry]=packageEntries(bytes),result=await packageEntry(bytes,entry,signal())
      expect(crc32(result)).toBe(entry.crc)
      bytes[30+entry.name.length]^=1
      await expect(packageEntry(bytes,entry,signal())).rejects.toThrow()
    }
  })
  it('rejects truncation, encrypted entries, ZIP64, path traversal and directory overlap',async()=>{
    const bytes=stored(),[entry]=packageEntries(bytes)
    for(let end=0;end<bytes.length;end++) expect(()=>packageEntries(bytes.slice(0,end))).toThrow()
    await expect(packageEntry(bytes,{...entry,flags:1},signal())).rejects.toThrow('encrypted')
    await expect(packageEntry(bytes,{...entry,dataLimit:entry.offset+30},signal())).rejects.toThrow('Inconsistent')
    const zip64=bytes.slice();new DataView(zip64.buffer).setUint32(entry.dataLimit+24,0xffffffff,true)
    expect(()=>packageEntries(zip64)).toThrow('ZIP64')
    const unsafe=stored('aa/bb.dll');unsafe.set(new TextEncoder().encode('..'),30);const [e]=packageEntries(unsafe);unsafe.set(new TextEncoder().encode('..'),e.dataLimit+46)
    expect(()=>packageEntries(unsafe)).toThrow('unsafe')
  })
  it('limits inflated output even when an entry lies about its size',async()=>{
    const bytes=deflated(new Uint8Array(100000)),[entry]=packageEntries(bytes),v=new DataView(bytes.buffer)
    v.setUint32(22,1,true);v.setUint32(entry.dataLimit+24,1,true)
    await expect(packageEntry(bytes,packageEntries(bytes)[0],signal())).rejects.toThrow('limit')
    const abort=new AbortController();abort.abort()
    await expect(packageEntry(bytes,entry,abort.signal)).rejects.toThrow()
    await expect(readBounded(new Blob([new Uint8Array(20)]).stream(),10,signal())).rejects.toThrow('limit')
  })
})
describe('framework resolution policy',()=>{
  it('uses fixed framework packages and excludes arbitrary package/name/URL guesses',()=>{
    expect(frameworkPackage(info(),identity)).toEqual({id:'microsoft.netcore.app.ref',version:'10.0.0',directory:'ref/net10.0/'})
    expect(frameworkPackage(info('.NETCoreApp,Version=v3.1'),identity)?.directory).toBe('ref/netcoreapp3.1/')
    expect(frameworkPackage(info('.NETFramework,Version=v4.8'),{...identity,name:'mscorlib',public_key_token:'b77a5c561934e089'})?.id).toBe('microsoft.netframework.referenceassemblies.net48')
    expect(frameworkPackage(info('.NETStandard,Version=v2.0'),identity)?.directory).toBe('build/netstandard2.0/ref/')
    for(const invalid of [{...identity,name:'Company.Private'},{...identity,name:'https://example.com/dll'},{...identity,public_key_token:''},{...identity,culture:'de'}]) expect(frameworkPackage(info(),invalid)).toBeNull()
    expect(frameworkPackage(info('.NETCoreApp,Version=v999.0'),identity)).toBeNull()
  })
  it('loads exact identities, reuses bounded package cache and clears it on disposal',async()=>{
    const fetcher=vi.fn(async()=>new Response(stored())),resolve=createFrameworkResolver(fetcher as typeof fetch),load=vi.fn((_bytes:Uint8Array,_identity:Identity)=>({id:2,info:info()} as Loaded))
    const result=await resolve(info(),identity,signal(),load,vi.fn())
    expect(result.package.id).toBe('microsoft.netcore.app.ref');expect(load.mock.calls[0][1]).toEqual(identity)
    await resolve(info(),identity,signal(),load,vi.fn());expect(fetcher).toHaveBeenCalledTimes(1)
    expect(fetcher.mock.calls[0]).toEqual(['https://api.nuget.org/v3-flatcontainer/microsoft.netcore.app.ref/10.0.0/microsoft.netcore.app.ref.10.0.0.nupkg',expect.objectContaining({credentials:'omit',referrerPolicy:'no-referrer',redirect:'error'})])
    resolve.clear();await resolve(info(),identity,signal(),load,vi.fn());expect(fetcher).toHaveBeenCalledTimes(2)
    await expect(resolve(info(),{...identity,name:'Private'},signal(),load,vi.fn())).rejects.toThrow('No automatic');expect(fetcher).toHaveBeenCalledTimes(2)
    await expect(resolve(info(),identity,signal(),()=>null,vi.fn())).rejects.toThrow('does not match')
  })
  it('handles HTTP failures and aborts without loading an assembly',async()=>{
    const load=vi.fn(),progress=vi.fn()
    await expect(createFrameworkResolver(vi.fn(async()=>new Response(null,{status:503})))(info(),identity,signal(),load,progress)).rejects.toThrow('503')
    const controller=new AbortController(),fetcher=vi.fn((_url:RequestInfo | URL,options?:RequestInit)=>new Promise<Response>((_resolve,reject)=>options?.signal?.addEventListener('abort',()=>reject(options.signal?.reason))))
    const pending=createFrameworkResolver(fetcher)(info(),identity,controller.signal,load,progress);controller.abort()
    await expect(pending).rejects.toThrow();expect(load).not.toHaveBeenCalled()
  })
})
it('workspace tree qualifies identical tokens by assembly and keeps references inspectable',()=>{
  const overview={info:info(),references:[{token:0x23000001,identity,resolved_id:2}],resources:[]} as unknown as Overview
  const tree=createWorkspaceTree([1,2].map(id=>({id,overview,declarations:[{token:0x02000001,parent:0,kind:'class',name:'Sample',namespace:'Example',full_name:'Example.Sample',flags:1}]})))
  expect(tree).toHaveLength(2);expect(pathToToken(tree,0x02000001,2)?.[0]).toBe('a:2:overview')
  expect(tree[0].children[0].children.find(n=>n.kind==='references')!.children[0].reference?.resolved_id).toBe(2)
  expect(pathToToken(tree,0x02000001,3)).toBeNull()
})
