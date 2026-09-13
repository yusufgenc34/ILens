'use client'
import {cn, ui} from '../lib/ui'
import {useEffect, useRef, useState} from 'react'
import {Download, FileCode2, X} from 'lucide-react'
import type {DecompilerClient} from '../lib/rpc'
import {diagnostic, type Diagnostic, type Loaded} from '../lib/types'
import {downloadFile, frameworkChoices, inferFramework} from '../lib/export'
import {projectName} from '../lib/project-export'
import type {ExportProgress, ProjectExportReport} from '../lib/export-types'

export default function ProjectExportDialog({client, assemblies, primary, onClose}: {client: DecompilerClient; assemblies: (Loaded & {filename: string})[]; primary: number; onClose: () => void}) {
  const dialog = useRef<HTMLDialogElement>(null)
  const controller = useRef<AbortController | null>(null)
  const job = useRef<number | null>(null)
  const mounted = useRef(true)
  const [selection, setSelection] = useState([primary])
  const [frameworks, setFrameworks] = useState<Record<number, string>>(() => Object.fromEntries(assemblies.map(a => [a.id, inferFramework(a.info.target_framework.moniker)])))
  const [name, setName] = useState(projectName(assemblies.find(a => a.id === primary)?.info.identity.name ?? 'Exported'))
  const [solution, setSolution] = useState(true)
  const [dependencies, setDependencies] = useState(false)
  const [diagnostics, setDiagnostics] = useState(false)
  const [busy, setBusy] = useState(false)
  const [progress, setProgress] = useState<ExportProgress | null>(null)
  const [report, setReport] = useState<ProjectExportReport | null>(null)
  const [error, setError] = useState<Diagnostic | null>(null)
  const [status, setStatus] = useState('')
  useEffect(() => {
    mounted.current = true
    const previous = document.activeElement; dialog.current?.showModal()
    return () => {mounted.current = false; controller.current?.abort(); if (job.current !== null) void client.call('cancelExport', {job: job.current}).catch(() => {}); dialog.current?.close(); if (previous instanceof HTMLElement && previous.isConnected) previous.focus()}
  }, [client])
  function cancel() {controller.current?.abort(); if (job.current !== null) void client.call('cancelExport', {job: job.current}).catch(() => {}); job.current = null; setBusy(false); setProgress(null); setStatus('Export cancelled. No file was downloaded.')}
  async function start() {
    const abort = new AbortController(); controller.current = abort; setBusy(true); setReport(null); setError(null); setStatus('')
    try {
      let next = await client.call('beginProjectExport', {options: {assemblies: assemblies.filter(a => selection.includes(a.id)).map(a => ({id: a.id, framework: frameworks[a.id]})), solutionName: name, createSolution: solution, dependencies, diagnostics}}, {signal: abort.signal})
      job.current = next.job; setProgress(next)
      while (!next.done) {next = await client.call('stepExport', {job: next.job}, {signal: abort.signal}); if (abort.signal.aborted || !mounted.current) return; setProgress(next)}
      const result = await client.call('finishExport', {job: next.job}, {signal: abort.signal}); job.current = null
      if (abort.signal.aborted || !mounted.current) return
      downloadFile(result); setReport(result.report as ProjectExportReport); setProgress(null); setStatus(`Downloaded ${result.name}. Extract the archive and open ${solution ? `${projectName(name.replace(/\.sln$/i, ''))}.sln` : 'the project folder'} in your IDE.`)
    } catch (error) {if (mounted.current && !abort.signal.aborted) setError(diagnostic(error))}
    finally {if (job.current !== null) {void client.call('cancelExport', {job: job.current}).catch(() => {}); job.current = null} if (mounted.current && !abort.signal.aborted) setBusy(false)}
  }
  const selected = assemblies.filter(a => selection.includes(a.id))
  const invalid = !selected.length || selected.some(a => !frameworks[a.id]) || !name.trim()
  return <dialog ref={dialog} className={cn(ui.dialog, ui.exportDialog)} aria-labelledby="project-export-title" onCancel={event => {event.preventDefault(); onClose()}}>
    <header className={cn(ui.dialogHeader)}><div><h2 id="project-export-title"><FileCode2 size={18} />Export to Project</h2><p>Create a C# solution from the selected assemblies.</p></div><button className={cn(ui.iconButton)} aria-label="Close project export" onClick={onClose}><X size={18} /></button></header>
    <div className={cn(ui.dialogBody, ui.exportBody)}>
      <section className="project-selection [&_input]:accent-primary"><h3>Assemblies</h3><div className="project-selection-list border border-solid border-border rounded-[6px] max-h-57.5 overflow-auto">{assemblies.map(a => <div className="project-assembly-row flex items-center justify-between gap-3 p-3 [&_+_.project-assembly-row]:border-t [&_+_.project-assembly-row]:border-solid [&_+_.project-assembly-row]:border-t-border [&_label]:flex [&_label]:items-center [&_label]:gap-[9px] [&_label]:min-w-0 [&_label]:text-[12px] [&_label_>_span]:[overflow-wrap:anywhere] [&_small]:block [&_small]:text-muted-foreground [&_small]:text-[11px] [&_small]:mt-[5px] [&_select]:border [&_select]:border-solid [&_select]:border-input [&_select]:text-foreground [&_select]:bg-background [&_select]:rounded-[5px] [&_select]:p-2 [&_select]:[font:inherit] [&_select]:text-[12px] [&_select]:min-w-0 [&_select]:w-45 [&_select]:shrink-0" key={a.id}><label><input type="checkbox" disabled={busy} checked={selection.includes(a.id)} onChange={event => setSelection(previous => event.target.checked ? [...previous, a.id] : previous.filter(id => id !== a.id))} /><span>{a.info.identity.name}<small>{a.info.type_count} types · {a.info.method_count} methods</small></span></label><select aria-label={`Target framework for ${a.info.identity.name}`} disabled={busy || !selection.includes(a.id)} value={frameworks[a.id]} onChange={event => setFrameworks(previous => ({...previous, [a.id]: event.target.value}))}><option value="">Select target framework</option>{frameworkChoices.map(f => <option key={f}>{f}</option>)}</select></div>)}</div><p>Each checked assembly becomes a project. References between selected assemblies become project references.</p></section>
      <section className="project-output-options"><h3>Output</h3><label className="project-name [&_input]:border [&_input]:border-solid [&_input]:border-input [&_input]:text-foreground [&_input]:bg-background [&_input]:rounded-[5px] [&_input]:p-2 [&_input]:[font:inherit] [&_input]:text-[12px] [&_input]:min-w-0 flex items-center gap-3 text-[12px] [&_input]:flex-1 [&_>_span]:text-muted-foreground">{solution ? 'Solution name' : 'Archive name'}<input value={name} maxLength={100} disabled={busy} onChange={event => setName(event.target.value)} /><span>{solution ? '.sln' : '.zip'}</span></label><div className="export-options grid gap-3 my-4 mx-0 [&_>_label]:flex [&_>_label]:gap-2 [&_>_label]:items-center [&_>_label]:text-[12px] [&_input[type=checkbox]]:accent-primary"><label><input type="checkbox" checked={solution} disabled={busy} onChange={event => setSolution(event.target.checked)} />Create solution (.sln)</label><label><input type="checkbox" checked={dependencies} disabled={busy} onChange={event => setDependencies(event.target.checked)} />Copy other loaded dependencies into References</label><label><input type="checkbox" checked={diagnostics} disabled={busy} onChange={event => setDiagnostics(event.target.checked)} />Include IL and metadata for all members</label></div><div className="project-layout-preview bg-muted border border-solid border-border py-3 px-3.5 rounded-[6px] font-mono text-[11px] font-normal leading-[1.8] max-h-40 overflow-auto [overflow-wrap:anywhere] [&_span]:block [&_span]:ml-4 [&_span]:text-muted-foreground [&_span]:text-[10px]" aria-label="Export folder structure">{solution && <div>{projectName(name.replace(/\.sln$/i, ''))}.sln</div>}{selected.map(a => <div key={a.id}>{projectName(a.info.identity.name)}/<span>{projectName(a.info.identity.name)}.csproj · C# files · Properties · Resources</span></div>)}</div><p>The browser downloads a ZIP containing the project folders. Unsupported methods keep their IL and a diagnostic report. Resource bytes are preserved; XAML and designer reconstruction are not available yet.</p></section>
      {progress && <div role="status" className="export-progress grid gap-2.5 text-[12px] [&_progress]:w-full [&_progress]:h-2 [&_progress]:accent-primary"><span>{progress.phase}</span><progress value={progress.completed} max={progress.total} /></div>}
      {status && <p role="status">{status}</p>}{error && <div role="alert" className="export-warning border border-solid border-warning-border text-warning-foreground bg-warning-background p-3 rounded-[6px] text-[12px] [overflow-wrap:anywhere] [&_p]:text-inherit [&_p]:mb-0"><strong>{error.message}</strong><p>{error.detail}</p></div>}
      {report && <div className="export-report border border-solid border-border rounded-[6px] p-3 text-[12px] [&_summary]:cursor-pointer [&_li]:leading-[1.6] [&_li]:text-muted-foreground"><strong>{report.projects.length} project{report.projects.length === 1 ? '' : 's'} exported</strong><ul>{report.projects.map(p => <li key={p.project}>{p.project}{p.outcome === 'partial' ? ' — partial reconstruction; review its export report' : ''}</li>)}</ul><p>Source reconstruction may require corrections before the solution compiles.</p></div>}
    </div>
    <footer className={cn(ui.dialogFooter)}><span className="export-footnote self-center text-[11px] text-muted-foreground max-w-[60%] leading-[1.5]">C# · SDK-style projects</span><div className={cn(ui.actions)}>{busy ? <button key="cancel" className={cn(ui.button, ui.subtle)} onClick={cancel}>Cancel export</button> : <><button className={cn(ui.button, ui.subtle)} onClick={onClose}>Close</button><button className={cn(ui.button, ui.primary)} disabled={invalid} onClick={() => void start()}><Download size={14} />Export</button></>}</div></footer>
  </dialog>
}
