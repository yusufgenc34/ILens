'use client'
import {useEffect,useRef,useState} from 'react'
import {Download,LoaderCircle,X} from 'lucide-react'
import {cn,ui} from '../lib/ui'
import type {DecompilerClient} from '../lib/rpc'
import {diagnostic,jsonText,tokenHex,type DeobfuscationOptions,type DeobfuscationProgress,type DeobfuscationReport,type Diagnostic,type EditsInfo,type Loaded} from '../lib/types'
import {downloadFile,safeName} from '../lib/export'

export default function DeobfuscateDialog({client,assembly,filename,autoStart=false,onOpened,onClose}:{client:DecompilerClient;assembly:number;filename:string;autoStart?:boolean;onOpened:(loaded:Loaded,filename:string)=>Promise<void>;onClose:()=>void}) {
  const dialog=useRef<HTMLDialogElement>(null), reportPanel=useRef<HTMLElement>(null), mounted=useRef(true), job=useRef<number|null>(null), controller=useRef<AbortController|null>(null), started=useRef(false)
  const [options,setOptions]=useState<DeobfuscationOptions>({constants:true,branches:true,strings:false,resource_strings:true,rename:true,proxies:true})
  const [progress,setProgress]=useState<DeobfuscationProgress|null>(null),[report,setReport]=useState<DeobfuscationReport|null>(null)
  const [error,setError]=useState<Diagnostic|null>(null),[edits,setEdits]=useState<EditsInfo|null>(null)
  const [busy,setBusy]=useState(false),[opening,setOpening]=useState(false),[status,setStatus]=useState('')
  const name=`${safeName(filename.replace(/\.(dll|exe)$/i,''))}.cleaned.${/\.exe$/i.test(filename)?'exe':'dll'}`
  const release=(id:number|null)=>{if(id!==null) void client.call('deobfuscateCancel',{job:id}).catch(()=>{})}
  useEffect(()=>{
    mounted.current=true;dialog.current?.showModal();const previous=document.activeElement
    void client.call('getEdits',{assembly}).then(value=>{if(mounted.current)setEdits(value)}).catch(e=>{if(mounted.current)setError(diagnostic(e))})
    return ()=>{mounted.current=false;controller.current?.abort();release(job.current);dialog.current?.close();if(previous instanceof HTMLElement&&previous.isConnected)previous.focus()}
  },[client,assembly])
  useEffect(()=>{if(report)reportPanel.current?.scrollIntoView({block:'start'})},[report])
  function cancel(){controller.current?.abort();release(job.current);job.current=null;setBusy(false);setProgress(null);setReport(null);setStatus('Cancelled. Pending changes were discarded; the original is unchanged.')}
  async function start(){
    const abort=new AbortController();controller.current=abort;setBusy(true);setError(null);setReport(null);setStatus('');release(job.current);job.current=null
    try{
      let next=await client.call('deobfuscateBegin',{assembly,options},{signal:abort.signal})
      if(abort.signal.aborted||!mounted.current)return
      job.current=next.job;setProgress(next)
      while(!next.done){next=await client.call('deobfuscateStep',{job:next.job},{signal:abort.signal});if(abort.signal.aborted||!mounted.current)return;setProgress(next)}
      setProgress({...next,phase:'Validating output'})
      const result=await client.call('deobfuscateFinish',{job:next.job},{signal:abort.signal})
      if(abort.signal.aborted||!mounted.current)return
      setReport(result);setProgress(null);setStatus(result.outcome==='no_changes'?'No supported changes found. Output is byte-for-byte identical.':`${result.changed_methods} methods changed, ${result.counts.renamed} names replaced. Review the report before using the output.`)
      // Fix it: open the cleaned copy directly; the original stays in the workspace.
      if(autoStart&&result.outcome!=='no_changes'){setBusy(false);await open()}
    }catch(e){if(mounted.current&&!abort.signal.aborted){release(job.current);job.current=null;setProgress(null);setError(diagnostic(e));setStatus('No output was published.')}}
    finally{if(mounted.current&&!abort.signal.aborted)setBusy(false)}
  }
  async function download(){
    if(job.current===null)return
    try{const result=await client.call('deobfuscateDownload',{job:job.current});if(mounted.current)downloadFile({name,mime:'application/octet-stream',buffer:result.buffer})}
    catch(e){if(mounted.current)setError(diagnostic(e))}
  }
  async function open(){
    if(job.current===null)return
    setOpening(true)
    try{const loaded=await client.call('deobfuscateOpen',{job:job.current});await onOpened(loaded,name);onClose()}
    catch(e){if(mounted.current)setError(diagnostic(e))}
    finally{if(mounted.current)setOpening(false)}
  }
  // Resource strings and renaming use the fixed-layout metadata writer; generic IL
  // transforms are dropped (with a notice) when only that writer can handle the file.
  const writerError=options.resource_strings||options.rename?edits?.resource_writer_error:edits?.writer_error
  const blocked=!!writerError||!!edits?.methods.length
  useEffect(()=>{if(autoStart&&edits&&!blocked&&!started.current){started.current=true;void start()}})
  const rows=report?.methods.filter(m=>m.status!=='unchanged'||m.reasons.length)??[]
  return <dialog ref={dialog} className={cn(ui.dialog,ui.exportDialog)} aria-labelledby="deobfuscate-title" onCancel={e=>{e.preventDefault();if(!opening)onClose()}}>
    <header className={ui.dialogHeader}><div><h2 id="deobfuscate-title">Deobfuscate assembly</h2><p>{filename}</p></div><button className={ui.iconButton} aria-label="Close deobfuscation" disabled={opening} onClick={onClose}><X size={18}/></button></header>
    <div className={cn(ui.dialogBody,ui.exportBody,'gap-2')}>
      <p className="text-[12px] text-muted-foreground">Static cleanup with a preserved original. Recognized resource-string protection is processed first; generic transforms are used when no compatible profile is found. Names and helper definitions are retained.</p>
      <fieldset disabled={busy||opening} className="my-2 grid gap-3 border-0 p-0 text-[12px] [&_label]:flex [&_label]:items-start [&_label]:gap-2 [&_input]:mt-1 [&_input]:accent-primary">
        <legend className="mb-3 font-medium">Transformations</legend>
        <label><input type="checkbox" checked={options.resource_strings} onChange={e=>setOptions({...options,resource_strings:e.target.checked})}/><span>Decode protected resource strings<span className="block text-muted-foreground">Supports one legacy Eazfuscator-compatible layout. Reads encrypted strings without running the assembly. Other protection variants remain unchanged.</span></span></label>
        <label><input type="checkbox" checked={options.rename} onChange={e=>setOptions({...options,rename:e.target.checked})}/><span>Rename obfuscated symbols<span className="block text-muted-foreground">Replaces invisible or invalid type, member and parameter names with readable synthetic names (Form0, method_3, string_0). Overrides keep matching names.</span></span></label>
        <label><input type="checkbox" checked={options.constants} onChange={e=>setOptions({...options,constants:e.target.checked})}/>Fold int32 constants</label>
        <label><input type="checkbox" checked={options.branches} onChange={e=>setOptions({...options,branches:e.target.checked})}/>Simplify constant branches and branch chains</label>
        <label><input type="checkbox" checked={options.proxies} onChange={e=>setOptions({...options,proxies:e.target.checked})}/><span>Unwrap proxy/indirect calls<span className="block text-muted-foreground">Redirects calls that only forward to another method straight to the real target. Forwarder definitions are kept; methods with exception regions are skipped in this pass.</span></span></label>
        <label><input type="checkbox" checked={options.strings} onChange={e=>setOptions({...options,strings:e.target.checked})}/><span>Evaluate pure string helpers <span className="block text-muted-foreground">Interprets a bounded CIL subset with literal arguments. No CLR execution, initializers or external calls outside the fixed string allowlist. Unsupported calls stay intact.</span></span></label>
      </fieldset>
      <p className="text-[11px] text-muted-foreground">Unsigned supported PE layouts, up to 16 MiB. Resource-string cleanup supports 16,384 methods and preserves exception regions and proven fixed-size field data. Generic transforms retain their 4,096 method / 256 change limits.</p>
      {!!edits?.methods.length&&<p role="alert" className="mt-3 text-warning-foreground">Save/reopen or discard pending method edits before deobfuscation.</p>}
      {writerError&&<p role="alert" className="mt-3 text-warning-foreground">{writerError.detail}</p>}
      <button className={cn(ui.button,ui.primary,'mt-4')} disabled={busy||opening||!edits||blocked||!Object.values(options).some(Boolean)} onClick={()=>void start()}>Run deobfuscation</button>
      {progress&&<div role="status" className="mt-4 grid gap-2 text-[12px]"><span>{progress.phase} · {progress.completed} / {progress.total}</span><progress className="h-2 w-full accent-primary" value={progress.completed} max={Math.max(progress.total,1)}/></div>}
      {status&&<p role="status" className="mt-4 text-[12px]">{status}</p>}
      {error&&<div role="alert" className="mt-3 rounded-md border border-warning-border bg-warning-background p-3 text-[12px] text-warning-foreground"><strong>{error.message}</strong><p>{error.detail}</p></div>}
      {report&&<section ref={reportPanel} aria-label="Deobfuscation report" className="mt-4 text-[12px]">
        <h3 className="font-medium">Report · {report.outcome.replaceAll('_',' ')}</h3>
        <p>{report.counts.constants} constants · {report.counts.branches} branches · {report.counts.strings} strings · {report.counts.proxies} proxy calls · {report.counts.renamed} names · {report.counts.unreachable_instructions} unreachable instructions removed · {report.skipped_methods} skipped methods · {report.unresolved_calls} unresolved calls</p>
        <ul className="my-3 list-disc space-y-1 pl-5 text-muted-foreground">{report.notices.map(n=><li key={n}>{n}</li>)}</ul>
        <details className="my-3 rounded-md border border-border p-3"><summary className="cursor-pointer">Method results ({rows.length})</summary><ul className="mt-2 max-h-56 space-y-3 overflow-auto">{rows.slice(0,200).map(m=><li key={m.token}><code>{tokenHex(m.token)}</code> · {m.status}{m.status==='changed'&&` · ${m.counts.strings} strings, ${m.counts.constants} constants, ${m.counts.branches} branches`}{m.reasons.map(r=><p key={r} className="text-muted-foreground">{r}</p>)}</li>)}</ul>{rows.length>200&&<p>Showing 200 entries. Download the report for all results.</p>}</details>
        <div className={ui.actions}><button className={cn(ui.button,ui.primary)} disabled={opening} onClick={()=>void download()}><Download size={14}/>Download {report.outcome==='no_changes'?'unchanged':'cleaned'} assembly</button><button className={cn(ui.button,ui.subtle)} disabled={opening} onClick={()=>void open()}>Open copy in workspace</button><button className={cn(ui.button,ui.subtle)} disabled={opening} onClick={()=>downloadFile({name:'deobfuscation-report.json',mime:'application/json',buffer:new TextEncoder().encode(jsonText(report)).buffer})}>Download report</button></div>
      </section>}
    </div>
    <footer className={ui.dialogFooter}><span className="self-center text-[11px] text-muted-foreground">Review transformed code before use.</span>{busy?<button className={cn(ui.button,ui.subtle)} onClick={cancel}><LoaderCircle size={14} className="animate-spin motion-reduce:animate-none"/>Cancel deobfuscation</button>:<button className={cn(ui.button,ui.subtle)} disabled={opening} onClick={onClose}>Done</button>}</footer>
  </dialog>
}
