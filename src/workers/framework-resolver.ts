'use client'
import {frameworkPackage} from '../lib/framework-catalog'
import type {AssemblyInfo, Identity, Loaded} from '../lib/types'
import {packageEntries, packageEntry, readBounded, type PackageEntry} from './nuget-package'
interface Package {bytes:Uint8Array<ArrayBuffer>; entries:PackageEntry[]}
export function createFrameworkResolver(fetcher: typeof fetch = fetch) {
  const cache = new Map<string,Package>()
  const resolve = async (info: AssemblyInfo, identity: Identity, signal: AbortSignal, load: (bytes:Uint8Array<ArrayBuffer>, identity:Identity) => Loaded | null, progress:(message:string) => void) => {
    const source = frameworkPackage(info,identity)
    if (!source) throw new Error('No automatic package source is configured for this reference. Add its DLL or select a dependency folder.')
    const url = `https://api.nuget.org/v3-flatcontainer/${source.id}/${source.version}/${source.id}.${source.version}.nupkg`
    let pack = cache.get(url)
    if (!pack) {
      progress(`Downloading ${source.id} ${source.version} from NuGet…`)
      // Idle timeout (no bytes for 30 s) plus an overall cap, so large framework packs
      // (~30 MB) still complete on slower connections.
      const controller = new AbortController(), abort=()=>controller.abort(signal.reason)
      const stall=()=>setTimeout(()=>controller.abort(new Error('NuGet download stalled. Retry or add the DLL locally.')),30_000)
      let timer=stall()
      const cap=setTimeout(()=>controller.abort(new Error('NuGet download timed out after 5 minutes. Retry or add the DLL locally.')),300_000)
      signal.addEventListener('abort',abort,{once:true})
      try {
        signal.throwIfAborted()
        const response = await fetcher(url,{signal:controller.signal,credentials:'omit',referrerPolicy:'no-referrer',redirect:'error',cache:'no-store'})
        if (!response.ok || !response.body) throw new Error(`NuGet returned HTTP ${response.status}. Retry or add the dependency locally.`)
        if (Number(response.headers.get('content-length') ?? 0) > 32*1024*1024) throw new Error('NuGet package exceeds 32 MiB.')
        const total=Number(response.headers.get('content-length') ?? 0)
        let reported=0
        const bytes = await readBounded(response.body,32*1024*1024,controller.signal,size=>{
          clearTimeout(timer);timer=stall()
          if(size-reported>=1024*1024){reported=size;progress(`Downloading ${source.id} ${source.version}: ${(size/1048576).toFixed(1)}${total?` / ${(total/1048576).toFixed(1)}`:''} MB…`)}
        })
        pack = {bytes,entries:packageEntries(bytes)}
        // At most two packages / 48 MiB of compressed package data per worker.
        while (cache.size && (cache.size >= 2 || [...cache.values()].reduce((n,p)=>n+p.bytes.length,bytes.length)>48*1024*1024)) cache.delete(cache.keys().next().value!)
        cache.set(url,pack)
      } catch(error) {
        signal.throwIfAborted()
        const message=error instanceof Error ? error.message : 'The NuGet package could not be downloaded.'
        throw Object.assign(new Error(message),{code:'dependency_download',detail:message})
      } finally {clearTimeout(timer); clearTimeout(cap); signal.removeEventListener('abort',abort)}
    }
    const prefix=source.directory.toLowerCase(), name=`${identity.name.toLowerCase()}.dll`
    const candidates=pack.entries.filter(e=>e.name.toLowerCase()===`${prefix}${name}` || e.name.toLowerCase()===`${prefix}facades/${name}`)
    if (!candidates.length) throw new Error('The official framework package does not contain this DLL. Add the matching dependency locally.')
    for (const entry of candidates) {
      progress(`Validating ${identity.name} ${identity.version}…`)
      const bytes=await packageEntry(pack.bytes,entry,signal)
      signal.throwIfAborted()
      const loaded=load(bytes,identity)
      if (loaded) return {...loaded,package:source}
    }
    throw new Error('The downloaded DLL does not match the referenced name, version, culture and public key token. Add an exact matching DLL locally.')
  }
  return Object.assign(resolve,{clear:()=>cache.clear()})
}
