'use client'
import {frameworkPackage} from './framework-catalog'
import {resolveDependencyFolder,type DependencyFolder,type DependencyResolutionState} from './dependency-folder'
import {diagnostic,type AssemblyInfo,type Diagnostic,type Identity,type Loaded} from './types'
import type {DecompilerClient} from './rpc'
const identityKey=(identity:Identity)=>JSON.stringify([identity.name.toLowerCase(),identity.version,identity.culture.toLowerCase(),identity.public_key_token.toLowerCase()])

// Orchestrates coarse worker calls only. Assembly validation and package extraction
// remain inside the worker; this function never receives downloaded binary bytes.
export async function resolveAutomaticDependencies(client:DecompilerClient, seeds:Loaded[], folder:DependencyFolder | null, signal:AbortSignal, online:()=>boolean, onLoaded:(loaded:Loaded,filename:string)=>Promise<void>, progress:(message:string)=>void) {
  const pending=seeds.map(loaded=>({loaded,framework:loaded.info})),known=new Set(seeds.map(a=>a.id))
  const visited=new Set<string>(), failedPackages=new Set<string>(), failures:Diagnostic[]=[]
  const localState:DependencyResolutionState={visited:new Set(),attempted:0}
  let downloaded=0, attempted=0, rejected=0, limited=false
  const retain=async(loaded:Loaded,filename:string,framework:AssemblyInfo=loaded.info)=>{
    signal.throwIfAborted()
    if(known.has(loaded.id)) return
    await onLoaded(loaded,filename)
    known.add(loaded.id);pending.push({loaded,framework})
  }
  const local=async(assemblies:Loaded[])=>{
    if(!folder) return
    const result=await resolveDependencyFolder(client,assemblies,folder,signal,(loaded,filename)=>retain(loaded,filename),progress,localState)
    rejected+=result.rejected
  }
  // All supplied files and selected-folder candidates take precedence over downloads.
  await local(seeds)
  for(let cursor=0;cursor<pending.length;cursor++) {
    signal.throwIfAborted()
    if(!online()) break
    const {loaded:owner,framework}=pending[cursor]
    const references=await client.call('getReferences',{assembly:owner.id},{signal})
    for(const reference of references) {
      signal.throwIfAborted()
      if(!online()) break
      if(reference.resolved_id!==null) continue
      const source=frameworkPackage(framework,reference.identity)
      if(!source) continue // Private/unknown references are kept unresolved, never sent to NuGet.
      const packageKey=`${source.id}/${source.version}`,key=`${packageKey}/${identityKey(reference.identity)}`
      if(visited.has(key) || failedPackages.has(packageKey)) continue
      if(attempted>=64) {limited=true;break}
      visited.add(key);attempted++
      progress(`Resolving dependency ${attempted}: ${reference.identity.name}…`)
      let result:Loaded
      try {result=await client.call('resolveFrameworkReference',{assembly:owner.id,token:reference.token},{signal})}
      catch(error) {
        signal.throwIfAborted()
        const failure=diagnostic(error)
        if(failure.code==='cancelled' || failure.code==='size_limit') throw error
        if(failure.code==='dependency_download') failedPackages.add(packageKey)
        if(failures.length<8) failures.push(failure)
        continue // Optional failures must not discard the opened EXE or other dependencies.
      }
      await retain(result,`${result.info.identity.name}.dll`,framework)
      downloaded++
      // Newly found references also prefer the selected folder, with one shared budget.
      await local([result])
    }
    if(limited) break
  }
  return {downloaded,rejected,limited,failures}
}
