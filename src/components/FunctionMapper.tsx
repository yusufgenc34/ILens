'use client'
import {useEffect,useRef,useState} from 'react'
import {LoaderCircle,RefreshCw,X} from 'lucide-react'
import {cn,ui} from '../lib/ui'
import type {DecompilerClient} from '../lib/rpc'
import {diagnostic,type Diagnostic,type FunctionMap} from '../lib/types'
import RelationshipGraph from './RelationshipGraph'

// One thing only: the assembly's relationship map.
export default function FunctionMapper({client,assembly,filename,cached,onLoaded,onOpen,onClose}:{client:DecompilerClient;assembly:number;filename:string;cached:FunctionMap|null;onLoaded:(map:FunctionMap)=>void;onOpen:(token:number)=>void;onClose:()=>void}) {
  const dialog=useRef<HTMLDialogElement>(null)
  const [map,setMap]=useState<FunctionMap|null>(cached),[error,setError]=useState<Diagnostic|null>(null),[loading,setLoading]=useState(!cached)
  const load=async()=>{
    setLoading(true);setError(null)
    try{const result=await client.call('functionMap',{assembly});setMap(result);onLoaded(result)}
    catch(e){setError(diagnostic(e))}
    finally{setLoading(false)}
  }
  useEffect(()=>{
    dialog.current?.showModal()
    if(!cached)void load()
    return ()=>dialog.current?.close()
  // Loads once per opened assembly; Refresh reloads explicitly.
  },[assembly])
  return <dialog ref={dialog} className={cn(ui.dialog,'relationship-mapper w-[min(1180px,_calc(100vw_-_24px))] max-w-none [&[open]]:flex [&[open]]:h-[min(860px,_92dvh)] [&[open]]:flex-col [&[open]]:overflow-hidden')} aria-labelledby="relationship-mapper-title" onCancel={e=>{e.preventDefault();onClose()}}>
    <header className={cn(ui.dialogHeader,'shrink-0')}>
      <div className="min-w-0"><h2 id="relationship-mapper-title">Relationship Mapper</h2><p className="truncate">{filename}{map&&` · ${map.types.length.toLocaleString()} types · ${map.functions.length.toLocaleString()} functions · ${map.edges.toLocaleString()} calls`}</p></div>
      <div className="flex shrink-0 items-center gap-1">
        <button className={ui.iconButton} aria-label="Rebuild map" title="Rebuild map" disabled={loading} onClick={()=>void load()}><RefreshCw size={16}/></button>
        <button className={ui.iconButton} aria-label="Close relationship mapper" onClick={onClose}><X size={18}/></button>
      </div>
    </header>
    {loading&&<p role="status" className="flex items-center gap-2 px-6 py-4 text-[12px]"><LoaderCircle size={14} className="animate-spin motion-reduce:animate-none"/>Mapping types and calls from CIL…</p>}
    {error&&<div role="alert" className="mx-6 my-3 rounded-md border border-warning-border bg-warning-background p-3 text-[12px] text-warning-foreground"><strong>{error.message}</strong><p>{error.detail}</p></div>}
    {map&&<div className="min-h-0 flex-1"><RelationshipGraph map={map} onOpen={onOpen}/></div>}
  </dialog>
}
