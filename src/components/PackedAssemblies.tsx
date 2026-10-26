'use client'
import {useEffect,useRef,useState} from 'react'
import {LoaderCircle,PackageOpen,X} from 'lucide-react'
import {cn,ui} from '../lib/ui'
import type {DecompilerClient} from '../lib/rpc'
import {diagnostic,type Diagnostic,type EmbeddedPayloads,type Loaded} from '../lib/types'

const size=(bytes:number)=>bytes<1024?`${bytes} B`:bytes<1024*1024?`${(bytes/1024).toFixed(1)} KB`:`${(bytes/(1024*1024)).toFixed(1)} MB`
// Assemblies a packer or merge tool carried inside this one's resources. Reading
// them is a parse, never an unpack: nothing is decompressed, decrypted or run.
export default function PackedAssemblies({client,assembly,filename,onOpened,onClose}:{client:DecompilerClient;assembly:number;filename:string;onOpened:(loaded:Loaded,filename:string)=>Promise<void>;onClose:()=>void}) {
  const dialog=useRef<HTMLDialogElement>(null)
  const [found,setFound]=useState<EmbeddedPayloads|null>(null),[error,setError]=useState<Diagnostic|null>(null),[busy,setBusy]=useState<string|null>(null)
  useEffect(()=>{
    dialog.current?.showModal()
    void (async()=>{
      try{setFound(await client.call('embeddedPayloads',{assembly}))}
      catch(e){setError(diagnostic(e))}
    })()
    return ()=>dialog.current?.close()
  },[assembly])
  async function open(token:number,entry:number,name:string) {
    setBusy(name);setError(null)
    try{
      const {buffer}=await client.call('getResourceBytes',{assembly,token,entry})
      const loaded=await client.call('load',{buffer},{transfer:[buffer]})
      await onOpened(loaded,name);onClose()
    }catch(e){setError(diagnostic(e))}
    finally{setBusy(null)}
  }
  return <dialog ref={dialog} className={cn(ui.dialog,'w-[min(720px,_calc(100vw_-_24px))]')} aria-labelledby="packed-title" onCancel={e=>{e.preventDefault();onClose()}}>
    <header className={ui.dialogHeader}>
      <div className="min-w-0"><h2 id="packed-title">Packed assemblies</h2><p className="truncate">{filename}</p></div>
      <button className={ui.iconButton} aria-label="Close packed assemblies" onClick={onClose}><X size={18}/></button>
    </header>
    <div className={cn(ui.dialogBody,'gap-3 text-[12px]')}>
      <p className="text-muted-foreground">Assemblies stored inside this one&apos;s resources. ILens reads them as bytes and parses them like any file you open; nothing is decompressed, decrypted or run.</p>
      {!found&&!error&&<p role="status" className="flex items-center gap-2"><LoaderCircle size={14} className="animate-spin motion-reduce:animate-none"/>Reading resources…</p>}
      {error&&<div role="alert" className="rounded-md border border-warning-border bg-warning-background p-3 text-warning-foreground"><strong>{error.message}</strong><p>{error.detail}</p></div>}
      {found&&!found.assemblies.length&&<p>No assembly is stored in this file&apos;s resources.</p>}
      {found&&!!found.assemblies.length&&<ul className="grid gap-2">
        {found.assemblies.map(p=><li key={`${p.token}:${p.entry}`} className="flex items-center gap-3 rounded-md border border-solid border-border p-3">
          <div className="min-w-0 flex-1">
            <p className="truncate font-medium">{p.name}</p>
            <p className="truncate text-muted-foreground">{p.identity}</p>
            <p className="text-muted-foreground">{p.runtime} · {size(p.size)} · {p.types.toLocaleString()} types · {p.methods.toLocaleString()} methods</p>
          </div>
          <button className={ui.button} disabled={busy!==null} onClick={()=>void open(p.token,p.entry,p.name)}>
            {busy===p.name?<LoaderCircle size={14} className="animate-spin motion-reduce:animate-none"/>:<PackageOpen size={14}/>}Open in workspace
          </button>
        </li>)}
      </ul>}
      {found&&!!found.unrecognized.length&&<p className="text-muted-foreground">Starts like an executable but could not be parsed as one, so it is left alone — this is where a compressed or encrypted payload shows up: {found.unrecognized.join(', ')}.</p>}
      {found?.truncated&&<p className="text-muted-foreground">Scanning stopped at this file&apos;s resource limit; later resources were not examined.</p>}
    </div>
  </dialog>
}
