'use client'
import {cn, ui} from '../lib/ui'
import {useEffect, useRef, useState} from 'react'
import {Check, LoaderCircle, X} from 'lucide-react'
import type {DecompilerClient} from '../lib/rpc'
import {diagnostic, tokenHex, type Diagnostic} from '../lib/types'
import type {EditorSettings} from '../lib/editor-settings'
import {compileCSharp, type CompileResult} from '../lib/csharp-compiler'
import CodeEditor from './CodeEditor'
type Phase = 'loading' | 'ready' | 'compiling' | 'applying'
/** dnSpy's Edit Method (C#): edit the reconstructed body, compile it with Roslyn in the
 * browser, merge and verify the IL. The analyzed assembly is never run. */
export default function CSharpEditDialog({client, assembly, token, name, settings, theme, onClose, onApplied}: {client: DecompilerClient; assembly: number; token: number; name: string; settings: EditorSettings; theme: string; onClose: () => void; onApplied: (reloaded: boolean) => void}) {
  const dialog = useRef<HTMLDialogElement>(null)
  const [source, setSource] = useState<{method: string; before: string; after: string; line: number} | null>(null)
  const text = useRef('')
  const [phase, setPhase] = useState<Phase>('loading')
  const [status, setStatus] = useState('')
  const [diagnostics, setDiagnostics] = useState<CompileResult['Diagnostics']>([])
  const [error, setError] = useState<Diagnostic | null>(null)
  useEffect(() => {
    const previous = document.activeElement; dialog.current?.showModal()
    const abort = new AbortController()
    client.call('csharpSource', {assembly, token}, {signal: abort.signal})
      .then(result => {setSource(result); text.current = result.method; setPhase('ready')})
      .catch(e => {if (!abort.signal.aborted) {setError(diagnostic(e)); setPhase('ready')}})
    return () => {abort.abort(); dialog.current?.close(); if (previous instanceof HTMLElement && previous.isConnected) previous.focus()}
  }, [client, assembly, token])
  async function compile() {
    if (!source) return
    setPhase('compiling'); setError(null); setDiagnostics([])
    try {
      setStatus('Loading the C# compiler (downloaded once)…')
      const ids = await client.call('assemblyIds', {})
      const references = await Promise.all(ids.map(async id => ({name: `assembly${id}.dll`, buffer: (await client.call('assemblyBytes', {assembly: id})).buffer})))
      setStatus('Compiling…')
      const {result, buffer} = await compileCSharp(source.before + text.current + source.after, references)
      if (!result.Success || !buffer) {setDiagnostics(result.Diagnostics); setStatus(''); setPhase('ready'); return}
      setPhase('applying'); setStatus('Merging and verifying the IL…')
      const {edit_revision: revision} = await client.call('getEdits', {assembly})
      const applied = await client.call('csharpApply', {assembly, token, revision, compiled: buffer})
      setStatus(applied.reloaded ? 'Applied. New references were added, so the workspace now holds the modified assembly.' : 'Applied. The change is included in modified assembly export.')
      onApplied(applied.reloaded)
    } catch (e) {setError(diagnostic(e)); setStatus('')}
    setPhase('ready')
  }
  const busy = phase !== 'ready'
  // Diagnostics inside the method use its own line numbers; others are in the skeleton.
  const describe = (d: CompileResult['Diagnostics'][number]) => {
    const line = source ? d.Line - source.line + 1 : d.Line
    const inside = source && line >= 1 && line <= text.current.split('\n').length
    return `${inside ? `Line ${d.Line}, column ${d.Column}` : 'Type skeleton'}: ${d.Id} ${d.Message}`
  }
  const missingFramework = diagnostics.some(d => ['CS0518', 'CS0012', 'CS8021'].includes(d.Id))
  return <dialog ref={dialog} className={cn(ui.dialog, "csharp-edit-dialog [&[open]]:flex [&[open]]:flex-col w-[min(1080px,_calc(100vw_-_32px))] h-[min(760px,_calc(100vh_-_48px))]")} aria-labelledby="csharp-edit-title" onCancel={e => {e.preventDefault(); onClose()}}>
    <header className={cn(ui.dialogHeader)}><div><h2 id="csharp-edit-title">Edit Method (C#)</h2><p>{name} · {tokenHex(token)}</p></div><button className={cn(ui.iconButton)} aria-label="Close C# editor" onClick={onClose}><X size={18} /></button></header>
    <p className="px-6 text-[12px] text-muted-foreground leading-[1.6]">Change the method's body and compile. Roslyn runs in your browser; the result is merged into the assembly and checked by the IL write verifier. Code that needs new types (lambdas, local functions, iterators, async) or a different signature is not supported yet.</p>
    <div className="flex-1 min-h-0 mx-6 my-3 border border-solid border-border rounded-[6px] overflow-hidden">{source ? <CodeEditor initial={source.method} firstLine={source.line} onChange={value => {text.current = value}} settings={settings} theme={theme} label="C# method source" /> : <p className="p-4 text-[12px] text-muted-foreground" role="status">{phase === 'loading' ? 'Reconstructing the method…' : ''}</p>}</div>
    {(diagnostics.length > 0 || error || status) && <div className="mx-6 mb-2 max-h-40 overflow-auto text-[12px]" role="status">
      {status && <p className="text-muted-foreground">{status}</p>}
      {diagnostics.length > 0 && <ul className="m-0 p-0 list-none [&_li]:py-0.5 text-warning-foreground" aria-label="Compiler errors">{diagnostics.map((d, i) => <li key={i}>{describe(d)}</li>)}</ul>}
      {missingFramework && <p className="text-muted-foreground">The compiler needs the .NET reference assemblies: enable "Download .NET references automatically" in Settings or add a dependency folder, then compile again.</p>}
      {error && <div role="alert" className="text-warning-foreground"><strong>{error.message}</strong> {error.detail}</div>}
    </div>}
    <footer className={cn(ui.dialogFooter)}><button className={cn(ui.button, ui.subtle)} onClick={onClose}>Close</button><button className={cn(ui.button, ui.primary)} disabled={busy || !source} onClick={() => void compile()}>{busy && phase !== 'loading' ? <LoaderCircle className="animate-spin motion-reduce:animate-none" size={14} /> : <Check size={14} />}Compile and apply</button></footer>
  </dialog>
}
