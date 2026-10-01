'use client'
import type {DependencyFolder} from './dependency-folder'
// Browser File objects only: never probe the filesystem or build remote URLs
// from executable names. Ambiguous folders must not choose a random DLL.
export function companionAssembly(exe:File, supplied:File[], folder:DependencyFolder | null):File | null {
  if(!/\.exe$/i.test(exe.name)) return null
  const name=exe.name.replace(/\.exe$/i,'.dll').toLowerCase()
  const selected=supplied.filter(file=>file.name.toLowerCase()===name)
  const candidates=selected.length ? selected : (folder?.candidates.get(name.slice(0,-4)) ?? []).filter(file=>file.name.toLowerCase()===name)
  const directory=exe.webkitRelativePath?.replace(/[^/]*$/,'')
  const adjacent=directory ? candidates.filter(file=>file.webkitRelativePath?.replace(/[^/]*$/,'')===directory) : []
  const matches=[...new Set(adjacent.length ? adjacent : candidates)]
  if(matches.length>1) throw {code:'not_managed',message:`More than one ${name} was found. Open the matching DLL explicitly.`,detail:'The executable contains no CLR metadata; a companion assembly cannot be chosen unambiguously.'}
  return matches[0] ?? null
}
