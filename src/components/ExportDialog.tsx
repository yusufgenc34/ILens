'use client'
import {cn, ui} from '../lib/ui'
import {useEffect, useRef, useState} from 'react'
import {Download, LoaderCircle, X} from 'lucide-react'
import type {DecompilerClient} from '../lib/rpc'
import {diagnostic, jsonText, type AssemblyInfo, type Declaration, type Diagnostic, type EditsInfo, type MethodAnalysis} from '../lib/types'
import {downloadFile, methodDownload, safeName, type ExportProgress, type ExportReport} from '../lib/export'
export default function ExportDialog({client, assembly, info, filename, member, method, mode, onClose}: {mode: 'source' | 'binary'; client: DecompilerClient; assembly: number; info: AssemblyInfo; filename: string; member?: Declaration; method: MethodAnalysis | null; onClose: () => void}) {
  const dialog = useRef<HTMLDialogElement>(null)
  const controller = useRef<AbortController | null>(null)
  const mounted = useRef(true)
  const job = useRef<number | null>(null)
  const [progress, setProgress] = useState<ExportProgress | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<Diagnostic | null>(null)
  const [report, setReport] = useState<ExportReport | null>(null)
  const [status, setStatus] = useState('')
  const [dependencies, setDependencies] = useState(false)
  const [edits, setEdits] = useState<EditsInfo | null>(null)
  useEffect(() => {
    mounted.current = true; dialog.current?.showModal()
    const previous = document.activeElement
    void client.call('getEdits', {assembly}).then(value => {if (mounted.current) setEdits(value)}).catch(error => {if (mounted.current) setError(diagnostic(error))})
    return () => {mounted.current = false; controller.current?.abort(); if (job.current !== null) void client.call('cancelExport', {job: job.current}).catch(() => {}); dialog.current?.close(); if (previous instanceof HTMLElement && previous.isConnected) previous.focus()}
  }, [client, assembly])
  function cancel() {controller.current?.abort(); if (job.current !== null) void client.call('cancelExport', {job: job.current}).catch(() => {}); job.current = null; setBusy(false); setProgress(null); setStatus('Export cancelled. No file was downloaded.')}
  async function start(scope: 'type' | 'assembly', format: 'zip' | 'source' = 'zip') {
    const abort = new AbortController(); controller.current = abort; setBusy(true); setError(null); setReport(null); setStatus('')
    try {
      let next = await client.call('beginExport', {assembly, options: {scope, token: scope === 'type' ? member?.token ?? 0 : 0, dependencies: format === 'zip' && dependencies, format}}, {signal: abort.signal})
      job.current = next.job; setProgress(next)
      while (!next.done) {next = await client.call('stepExport', {job: next.job}, {signal: abort.signal}); if (abort.signal.aborted || !mounted.current) return; setProgress(next)}
      const result = await client.call('finishExport', {job: next.job}, {signal: abort.signal})
      job.current = null
      if (abort.signal.aborted || !mounted.current) return
      setProgress(null); setReport(result.report); downloadFile(result); setStatus(format === 'source' ? 'Type source downloaded.' : result.report.outcome === 'partial' ? 'Partial reconstruction downloaded. Review export-report.json inside the archive.' : 'Source archive downloaded. Reconstructed code still requires review.')
    } catch (error) {if (mounted.current && !abort.signal.aborted) {setError(diagnostic(error)); setStatus('Export failed. No file was downloaded.')}}
    finally {if (job.current !== null) {void client.call('cancelExport', {job: job.current}).catch(() => {}); job.current = null} if (mounted.current && !abort.signal.aborted) setBusy(false)}
  }
  function selected(language: 'C#' | 'IL') {
    if (!method || !member) return
    try {downloadFile(methodDownload(method, member, language)); setStatus(`${language} method downloaded.`)} catch (error) {setError(diagnostic(error))}
  }
  async function binary() {
    if (!edits) return
    const abort = new AbortController(); controller.current = abort; setBusy(true); setError(null); setStatus('Validating the modified PE and unchanged members…')
    try {
      const result = await client.call('exportModifiedAssembly', {assembly, revision: edits.revision}, {signal: abort.signal})
      if (abort.signal.aborted || !mounted.current) return
      downloadFile({name: `${safeName(filename.replace(/\.(dll|exe)$/i, ''))}.modified.${/\.exe$/i.test(filename) ? 'exe' : 'dll'}`, mime: 'application/octet-stream', buffer: result.buffer}); setStatus('Validated modified assembly downloaded.')
    } catch (error) {if (mounted.current && !abort.signal.aborted) setError(diagnostic(error))}
    finally {if (mounted.current && !abort.signal.aborted) setBusy(false)}
  }
  const selectedMethod = method && member && method.token === member.token
  return <dialog ref={dialog} className={cn(ui.dialog, ui.exportDialog)} aria-labelledby="export-title" onCancel={e => {e.preventDefault(); onClose()}}>
    <header className={cn(ui.dialogHeader)}><div><h2 id="export-title"><Download size={18} />{mode === 'binary' ? 'Save Module' : 'Save Code'}</h2><p>{info.identity.name}</p></div><button className={cn(ui.iconButton)} aria-label="Close export" onClick={onClose}><X size={18} /></button></header>
    <div className={cn(ui.dialogBody, ui.exportBody)}>
      {mode === 'source' && <><section><h3>Selected member</h3><p>{member?.full_name ?? 'Select a method or type in the explorer.'}</p><div className={cn(ui.actions)}><button className={cn(ui.button, ui.subtle)} disabled={busy || !selectedMethod || method.quality === 'annotated_il'} onClick={() => selected('C#')}>Download C#</button><button className={cn(ui.button, ui.subtle)} disabled={busy || !selectedMethod} onClick={() => selected('IL')}>Download IL</button><button className={cn(ui.button, ui.subtle)} disabled={busy || ((member?.token ?? 0) >>> 24) !== 2} onClick={() => void start('type', 'source')}>Download type C#</button><button className={cn(ui.button, ui.subtle)} disabled={busy || ((member?.token ?? 0) >>> 24) !== 2} onClick={() => void start('type')}>Export type sources</button></div>{selectedMethod && method.quality === 'annotated_il' && <p>C# reconstruction is unavailable for this method. Its original IL can be downloaded.</p>}</section>
      <section><h3>Assembly sources</h3><p>Download reconstructed types, independent IL, metadata, resources and a completeness report as a ZIP. Source downloads use the original assembly; modified DLL/EXE output includes applied IL edits.</p>
        <div className="export-options grid gap-3 my-4 mx-0 [&_>_label]:flex [&_>_label]:gap-2 [&_>_label]:items-center [&_>_label]:text-[12px] [&_input[type=checkbox]]:accent-primary"><label><input type="checkbox" checked={dependencies} disabled={busy} onChange={e => setDependencies(e.target.checked)} />Include loaded dependency DLLs</label></div>
        <button className={cn(ui.button, ui.primary)} disabled={busy} onClick={() => void start('assembly')}><Download size={14} />Export assembly sources</button>
      </section>
      </>}
      {mode === 'binary' && <section><h3>Modified assembly</h3>{edits?.methods.length ? <><p>{edits.methods.length} validated method change{edits.methods.length === 1 ? '' : 's'}. Metadata tokens and original assembly identity are retained. Stale debug records and their payloads are removed.</p><ul className="changed-methods max-h-25 overflow-auto pl-5 text-muted-foreground font-mono text-[11px] [overflow-wrap:anywhere]">{edits.methods.map(m => <li key={m.token}>{m.name}</li>)}</ul></> : <p>Apply a validated change in the IL editor to enable DLL/EXE output.</p>}{edits?.writer_error && <p className="export-warning border border-solid border-warning-border text-warning-foreground bg-warning-background p-3 rounded-[6px] text-[12px] [overflow-wrap:anywhere] [&_p]:text-inherit [&_p]:mb-0">{edits.writer_error.detail}</p>}<button className={cn(ui.button, ui.subtle)} disabled={busy || !edits?.methods.length || !!edits.writer_error} onClick={() => void binary()}><Download size={14} />Download modified assembly</button></section>}
      {progress && <div className="export-progress grid gap-2.5 text-[12px] [&_progress]:w-full [&_progress]:h-2 [&_progress]:accent-primary" role="status"><span>{progress.phase} · {progress.completed} / {progress.total}</span><progress value={progress.completed} max={Math.max(progress.total, 1)} /></div>}
      {status && <p role="status">{status}</p>}{error && <div className="export-warning border border-solid border-warning-border text-warning-foreground bg-warning-background p-3 rounded-[6px] text-[12px] [overflow-wrap:anywhere] [&_p]:text-inherit [&_p]:mb-0" role="alert"><strong>{error.message}</strong><p>{error.detail}</p></div>}
      {report && <details className="export-report border border-solid border-border rounded-[6px] p-3 text-[12px] [&_summary]:cursor-pointer [&_li]:leading-[1.6] [&_li]:text-muted-foreground"><summary>Export report · {report.outcome}</summary><p>{report.types.length} source files · {report.members.length} member outcomes · {report.resources.length} resources</p><ul>{report.notices.map((n, i) => <li key={i}>{n}</li>)}</ul><button className={cn(ui.button, ui.subtle)} onClick={() => downloadFile({name: 'export-report.json', mime: 'application/json', buffer: new TextEncoder().encode(jsonText(report)).buffer})}>Download report</button></details>}
    </div>
    <footer className={cn(ui.dialogFooter)}><span className="export-footnote self-center text-[11px] text-muted-foreground max-w-[60%] leading-[1.5]">{mode === 'binary' ? 'The original input file is never overwritten.' : 'Source downloads contain reconstructed code.'}</span>{busy ? <button key="cancel" className={cn(ui.button, ui.subtle)} onClick={cancel}><LoaderCircle size={14} className="spin animate-spin motion-reduce:animate-none" />Cancel export</button> : <button key="done" className={cn(ui.button, ui.primary)} onClick={onClose}>Done</button>}</footer>
  </dialog>
}
