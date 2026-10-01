'use client'
import type {DecompilerClient} from './rpc'
import type {Identity, Loaded} from './types'

export interface DependencyResolutionState {visited:Set<string>; attempted:number}
export interface DependencyFolder {files: number; candidates: Map<string, File[]>}
export function indexDependencyFolder(files: File[]): DependencyFolder {
  if (files.length > 4096) throw new Error('Choose a smaller dependency folder (at most 4,096 files).')
  const candidates = new Map<string, File[]>()
  for (const file of files) {
    if (!/\.(dll|exe)$/i.test(file.name)) continue
    const name = file.name.replace(/\.(dll|exe)$/i, '').toLowerCase()
    const group = candidates.get(name) ?? []
    if (group.length >= 32) throw new Error(`Too many versions of ${file.name}; choose a more specific folder.`)
    group.push(file); candidates.set(name, group)
  }
  return {files: [...candidates.values()].reduce((count, group) => count + group.length, 0), candidates}
}
const key = (identity: Identity) => JSON.stringify([identity.name.toLowerCase(), identity.version, identity.culture.toLowerCase(), identity.public_key_token.toLowerCase()])
export async function resolveDependencyFolder(client: DecompilerClient, assemblies: Loaded[], folder: DependencyFolder, signal: AbortSignal, onLoaded: (loaded: Loaded, filename: string) => Promise<void>, progress: (message: string) => void, state: DependencyResolutionState = {visited:new Set(),attempted:0}) {
  const pending = [...assemblies]
  const visited = state.visited
  const known = new Set(assemblies.map(a => a.id))
  let loadedCount = 0, unavailable = 0, rejected = 0
  const active = () => {if (signal.aborted) throw {code:'cancelled', message:'Dependency resolution cancelled.', detail:'Workspace closed.'}}
  for (let cursor = 0; cursor < pending.length; cursor++) {
    active()
    const references = await client.call('getReferences', {assembly:pending[cursor].id}, {signal})
    for (const reference of references) {
      active()
      const identity = key(reference.identity)
      if (reference.resolved_id !== null || visited.has(identity)) continue
      visited.add(identity)
      let matched = false
      for (const file of folder.candidates.get(reference.identity.name.toLowerCase()) ?? []) {
        active()
        if (++state.attempted > 128) throw new Error('Dependency resolution reached the 128-file inspection limit. Choose a more specific folder.')
        if (file.size > 64 * 1024 * 1024) {rejected++; continue}
        progress(`Resolving ${reference.identity.name} ${reference.identity.version}…`)
        const buffer = await file.arrayBuffer()
        active()
        let loaded: Loaded | null
        try {loaded = await client.call('loadDependency', {buffer, identity:reference.identity}, {signal, transfer:[buffer]})}
        catch (error) {
          active()
          if (error && typeof error === 'object' && 'code' in error && ['invalid_pe','not_managed','invalid_clr','corrupt_metadata','unsupported_metadata','unsupported_native'].includes(String(error.code))) {rejected++; continue}
          throw error
        }
        if (!loaded) continue
        matched = true
        if (!known.has(loaded.id)) {
          known.add(loaded.id); pending.push(loaded)
          await onLoaded(loaded, file.name); loadedCount++
        }
        break
      }
      if (!matched) unavailable++
    }
  }
  return {loaded:loadedCount, unavailable, rejected}
}
