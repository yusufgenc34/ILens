'use client'
import {cn, ui} from '../lib/ui'
import { useCallback, useEffect, useMemo, useRef, useState, type ChangeEvent, type DragEvent, type CSSProperties } from 'react'
import { ArrowRight, Binary, BookOpen, Box, Braces, Check, ChevronRight, Code2, Copy, Pencil, FileCode2, HardDrive, Layers, Link as LinkIcon, LoaderCircle, Plus, Search, Upload, WrapText, X } from 'lucide-react'
import { DecompilerClient } from '../lib/rpc'
import { diagnostic, jsonText, tokenHex, type Declaration, type Diagnostic, type Loaded, type MethodAnalysis, type Overview, type SearchHit, type XrefPage } from '../lib/types'
import { createTree } from '../lib/tree'
import Explorer from './Explorer'
import CodeViewer from './CodeViewer'
import PaneHandle from './PaneHandle'
import SettingsDialog from './SettingsDialog'
import ExportDialog from './ExportDialog'
import ProjectExportDialog from './ProjectExportDialog'
import AppMenuBar from './AppMenuBar'
import ILEditorDialog from './ILEditorDialog'
import AssemblyWarnings, {obfuscationLabels} from './AssemblyWarnings'
import {defaultEditorSettings, editorSettingsKey, parseEditorSettings} from '../lib/editor-settings'
interface WorkspaceAssembly extends Loaded { filename: string; declarations: Declaration[]; overview: Overview; elapsed: number }
type View = 'C#' | 'IL' | 'Metadata' | 'Hex'
const views: View[] = ['C#', 'IL', 'Metadata', 'Hex']
const bytes = (size: number) => size < 1024 * 1024 ? `${(size / 1024).toFixed(1)} KB` : `${(size / (1024 * 1024)).toFixed(1)} MB`
export default function DecompilerApp() {
  const [ready, setReady] = useState(false)
  const [assemblies, setAssemblies] = useState<WorkspaceAssembly[]>([])
  const [primary, setPrimary] = useState<number | null>(null)
  const [selected, setSelected] = useState<number | null>(null)
  const [view, setView] = useState<View>('C#')
  const [method, setMethod] = useState<MethodAnalysis | null>(null)
  const [methodAssembly, setMethodAssembly] = useState<number | null>(null)
  const [metadata, setMetadata] = useState<Record<string, unknown> | null>(null)
  const [declaration, setDeclaration] = useState('')
  const [status, setStatus] = useState('Ready to open an assembly')
  const [busy, setBusy] = useState(false)
  const [loadingFiles, setLoadingFiles] = useState(false)
  const [errors, setErrors] = useState<Diagnostic[]>([])
  const [theme, setTheme] = useState('dark')
  const [editorSettings, setEditorSettings] = useState(defaultEditorSettings)
  const [settingsOpen, setSettingsOpen] = useState(false)
  const [exportOpen, setExportOpen] = useState<'source' | 'binary' | null>(null)
  const [projectOpen, setProjectOpen] = useState(false)
  const [editOpen, setEditOpen] = useState(false)
  const wrap = editorSettings.wrap
  const [copied, setCopied] = useState(false)
  const [dragging, setDragging] = useState(false)
  const [leftWidth, setLeftWidth] = useState(285)
  const [rightWidth, setRightWidth] = useState(275)
  const [searchOpen, setSearchOpen] = useState(false)
  const [query, setQuery] = useState('')
  const [results, setResults] = useState<SearchHit[]>([])
  const [searching, setSearching] = useState(false)
  const [helpOpen, setHelpOpen] = useState(false)
  const [xrefs, setXrefs] = useState<XrefPage['hits'] | null>(null)
  const [xrefBusy, setXrefBusy] = useState(false)
  const [xrefStatus, setXrefStatus] = useState('')
  const client = useRef<DecompilerClient | null>(null)
  const filePicker = useRef<HTMLInputElement>(null)
  const dependencyPicker = useRef<HTMLInputElement>(null)
  const searchInput = useRef<HTMLInputElement>(null)
  const selectionRequest = useRef<AbortController | null>(null)
  const xrefRequest = useRef<AbortController | null>(null)
  const selectionVersion = useRef(0)
  const generation = useRef(0)
  const loadLock = useRef(false)
  const active = assemblies.find(a => a.id === primary) ?? null
  const activeDeclaration = active?.declarations.find(d => d.token === selected)
  const tree = useMemo(() => active ? createTree(active.declarations, active.overview) : [], [active])
  const report = useCallback((error: unknown) => {const d = diagnostic(error); if (d.code !== 'cancelled') setErrors(previous => [...previous.slice(-4), d])}, [])
  const reset = useCallback((message = 'Workspace closed. Assembly memory released.') => {
    generation.current++; selectionVersion.current++; selectionRequest.current?.abort(); xrefRequest.current?.abort()
    client.current?.terminate(); client.current = null; loadLock.current = false
    setExportOpen(null); setProjectOpen(false); setEditOpen(false); setAssemblies([]); setPrimary(null); setSelected(null); setMethod(null); setMetadata(null); setDeclaration(''); setBusy(false); setLoadingFiles(false); setXrefs(null); setXrefBusy(false); setStatus(message)
  }, [])
  function getClient() {
    if (!client.current) {
      const worker = new Worker(new URL('../workers/decompiler.worker.ts', import.meta.url), {type: 'module', name: 'ilens-static-analysis'})
      client.current = new DecompilerClient(worker)
      client.current.onProgress = message => setStatus(message)
      client.current.onFatal = error => { reset(error.message); report(error) }
    }
    return client.current
  }
  useEffect(() => {
    setReady(true)
    try {setTheme((localStorage.getItem('ilens:theme') ?? localStorage.getItem('monoasm:theme')) === 'light' ? 'light' : 'dark'); setEditorSettings(parseEditorSettings(localStorage.getItem(editorSettingsKey) ?? localStorage.getItem('monoasm:editor-settings')))} catch { /* Storage is optional. */ }
    return () => {client.current?.terminate()}
  }, [])
  useEffect(() => {if (!ready) return; try {localStorage.setItem('ilens:theme', theme); localStorage.setItem(editorSettingsKey, JSON.stringify(editorSettings))} catch { /* Storage is optional. */ }}, [ready, theme, editorSettings])
  useEffect(() => {
    const keyboard = (e: KeyboardEvent) => {
      if ((e.target as Element | null)?.closest('dialog[open]')) return
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'o') {e.preventDefault(); filePicker.current?.click()}
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {e.preventDefault(); setSearchOpen(open => !open)}
      if (e.key === 'Escape') {setSearchOpen(false); setHelpOpen(false)}
    }
    window.addEventListener('keydown', keyboard); return () => window.removeEventListener('keydown', keyboard)
  }, [])
  useEffect(() => {if (searchOpen) searchInput.current?.focus()}, [searchOpen])
  useEffect(() => {
    if (!searchOpen || !primary || !query.trim()) {setResults([]); setSearching(false); return}
    const controller = new AbortController()
    setSearching(true)
    const timer = setTimeout(() => {void getClient().call('search', {assembly: primary, query: query.slice(0, 256)}, {signal: controller.signal}).then(setResults).catch(report).finally(() => {if (!controller.signal.aborted) setSearching(false)})}, 250)
    return () => {clearTimeout(timer); controller.abort()}
  }, [query, primary, searchOpen, report])
  async function openFiles(files: File[], dependencies = false) {
    if (loadLock.current || !files.length) return
    loadLock.current = true; setLoadingFiles(true); setErrors([])
    const version = generation.current
    let firstId: number | null = null
    const loaded: WorkspaceAssembly[] = []
    try {
      for (const file of files.slice(0, 16)) {
        if (version !== generation.current) return
        if (!/\.(dll|exe)$/i.test(file.name)) {report({code: 'invalid_pe', message: `${file.name} is not a DLL or EXE.`, detail: 'Choose a managed PE assembly.'}); continue}
        if (file.size > 64 * 1024 * 1024) {report({code: 'size_limit', message: `${file.name} exceeds 64 MiB.`, detail: 'The file was rejected before reading it into memory.'}); continue}
        const started = performance.now()
        setStatus(`Reading ${file.name}…`)
        try {
          const core = getClient()
          const buffer = await file.arrayBuffer()
          if (version !== generation.current) return
          const result = await core.call('load', {buffer}, {transfer: [buffer]})
          try {
            const declarations = await core.call('getTree', {assembly: result.id})
            const overview = await core.call('getInfo', {assembly: result.id})
            loaded.push({...result, filename: file.name, declarations, overview, elapsed: performance.now() - started})
            firstId ??= result.id
          } catch (error) {await core.call('close', {assembly: result.id}); throw error}
        } catch (error) {report(error)}
      }
      if (version !== generation.current) return
      // Refresh resolution statuses after all dependency identities have been indexed.
      const next = [...assemblies, ...loaded]
      for (const assembly of next) assembly.overview = {...assembly.overview, references: await getClient().call('getReferences', {assembly: assembly.id})}
      setAssemblies(next)
      if (firstId !== null && (!dependencies || !primary)) {setPrimary(firstId); setSelected(0); setView('C#')}
      setStatus(loaded.length ? `${loaded.length} ${dependencies ? 'dependency assemblies' : 'assemblies'} loaded` : 'No new assemblies loaded')
    } catch (error) {report(error)}
    finally {if (version === generation.current) {loadLock.current = false; setLoadingFiles(false)}}
  }
  function onFiles(event: ChangeEvent<HTMLInputElement>, dependencies: boolean) {const files = Array.from(event.target.files ?? []); event.target.value = ''; void openFiles(files, dependencies)}
  function drop(event: DragEvent) {event.preventDefault(); setDragging(false); void openFiles(Array.from(event.dataTransfer.files), !!active)}
  function select(token: number) {setSelected(token); setSearchOpen(false); if (token === -1 || token === -2 || token === 0) setView('Metadata')}
  useEffect(() => {
    selectionRequest.current?.abort(); xrefRequest.current?.abort(); setXrefs(null); setXrefBusy(false); setXrefStatus(''); setMethod(null); setMetadata(null); setDeclaration('')
    const version = ++selectionVersion.current
    if (!active || selected === null) return
    if (selected <= 0) {setBusy(false); return}
    const controller = new AbortController(); selectionRequest.current = controller
    setBusy(true)
    const core = getClient()
    const run = async () => {
      try {
        const type = await core.call('getType', {assembly: active.id, token: selected}, {signal: controller.signal})
        if (version !== selectionVersion.current) return
        setMetadata(type.metadata); setDeclaration(type.declaration)
        if (selected >>> 24 === 6) {
          const analysis = await core.call('decompileMethod', {assembly: active.id, token: selected}, {signal: controller.signal})
          if (version !== selectionVersion.current) return
          setMethod(analysis); setMethodAssembly(active.id)
        }
        if (version === selectionVersion.current) setStatus('Analysis complete')
      } catch (error) {if (version === selectionVersion.current) report(error)}
      finally {if (version === selectionVersion.current) setBusy(false)}
    }
    void run(); return () => controller.abort()
  }, [primary, selected, active?.declarations, active?.overview.references, report])
  async function findXrefs() {
    if (!primary || !selected || selected < 0) return
    const controller = new AbortController(); xrefRequest.current?.abort(); xrefRequest.current = controller
    setXrefBusy(true); setXrefs([])
    try {
      let cursor = 0; let hits: XrefPage['hits'] = []; let skipped = 0
      while (!controller.signal.aborted) {
        const page = await getClient().call('getXrefs', {assembly: primary, token: selected, cursor}, {signal: controller.signal})
        if (controller.signal.aborted) break
        hits = [...hits, ...page.hits]; skipped += page.skipped.length
        setXrefs(hits.slice(0, 500)); setXrefStatus(`${page.next.toLocaleString()} / ${page.total.toLocaleString()} methods scanned${skipped ? ` · ${skipped}+ invalid bodies skipped` : ''}`)
        if (page.done || hits.length >= 500) break
        cursor = page.next
      }
    } catch (error) {report(error)}
    finally {if (!controller.signal.aborted) setXrefBusy(false)}
  }
  const displayMetadata = selected === -1 ? active?.overview.references : selected === -2 ? active?.overview.resources : selected === 0 ? active?.overview : metadata
  const code = useMemo(() => {
    if (!active) return ''
    if (view === 'Metadata') return jsonText(displayMetadata ?? {})
    if (view === 'Hex') {
      if (method?.body) return `// File offset 0x${method.body.file_offset.toString(16).toUpperCase()} · ${method.body.code_size} bytes of CIL\n\n${method.body.instructions.map(i => `IL_${i.offset.toString(16).padStart(4, '0').toUpperCase()}   ${i.bytes.padEnd(35)}  // ${i.name}`).join('\n')}`
      return `// Raw metadata table row\n${String(metadata?.raw ?? 'Select a method or metadata member to inspect raw bytes.')}`
    }
    if (view === 'IL') return method?.il ?? '// Select a method to inspect its original CIL instructions.'
    if (method) return method.csharp
    if (declaration) return declaration
    return `// ${active.info.identity.name}\n// Target: ${active.info.target_framework.display_name} · ${active.info.machine}\n// CLR metadata: ${active.info.runtime}\n// ${active.info.type_count} types · ${active.info.method_count} methods\n\n// Select a type or method in the Assembly Explorer.\n// C# is reconstructed from metadata and CIL, not recovered original source.\n// No managed code is executed.`
  }, [active, view, displayMetadata, method, metadata, declaration])
  const outgoing = useMemo(() => {
    const unique = new Map<number, string>()
    for (const instruction of method?.body?.instructions ?? []) if (instruction.operand.kind === 'token' && typeof instruction.operand.value === 'number') unique.set(instruction.operand.value, instruction.resolved ?? tokenHex(instruction.operand.value))
    return [...unique].slice(0, 100)
  }, [method])
  async function copy() {try {await navigator.clipboard.writeText(code); setCopied(true); setTimeout(() => setCopied(false), 1600)} catch (error) {report(error)}}
  async function openSample() {try {const response = await fetch(`${import.meta.env.BASE_URL}samples/ILens.Patterns.dll`); if (!response.ok) throw new Error('The sample assembly could not be loaded.'); const blob = await response.blob(); await openFiles([new File([blob], 'ILens.Patterns.dll')])} catch (error) {report(error)}}
  const resolvedDefinition = metadata?.resolved_definition as {assembly: number; token: number} | null | undefined
  const selectedLabel = activeDeclaration?.name ?? (selected === -1 ? 'Assembly references' : selected === -2 ? 'Manifest resources' : selected === 0 ? 'Assembly overview' : metadata?.name ? String(metadata.name) : 'Select a member')
  return <div data-ready={ready} className={cn(`application bg-background text-foreground font-sans text-[13px] font-normal flex flex-col h-dvh min-w-165 leading-[1.5] ${theme}`)} onDragOver={e => {e.preventDefault(); if (e.dataTransfer.types.includes('Files')) setDragging(true)}} onDragLeave={e => {if (!e.currentTarget.contains(e.relatedTarget as Node)) setDragging(false)}} onDrop={drop}>
    <input ref={filePicker} type="file" disabled={!ready || loadingFiles} accept=".dll,.exe" multiple hidden aria-label="Open assembly files" onChange={e => onFiles(e, false)} />
    <input ref={dependencyPicker} type="file" disabled={!ready || loadingFiles} accept=".dll,.exe" multiple hidden aria-label="Add dependency files" onChange={e => onFiles(e, true)} />
    <AppMenuBar context={active?.filename ?? ''} menus={[
      {label: 'File', actions: [
        {label: 'Open Assembly…', shortcut: 'Ctrl/⌘ O', disabled: !ready || loadingFiles, run: () => filePicker.current?.click()},
        {label: 'Add Dependencies…', disabled: !ready || loadingFiles, run: () => dependencyPicker.current?.click()}, null,
        {label: 'Export to Project…', disabled: !active || loadingFiles, run: () => setProjectOpen(true)},
        {label: 'Save Code…', disabled: !active || !activeDeclaration || busy, run: () => setExportOpen('source')},
        {label: 'Save Module…', disabled: !active || loadingFiles, run: () => setExportOpen('binary')}, null,
        {label: 'Close Workspace', disabled: !active, run: () => reset()},
      ]},
      {label: 'Edit', actions: [
        {label: 'Copy Code', disabled: !active || busy, run: () => {void copy()}}, null,
        {label: 'Search Assembly…', shortcut: 'Ctrl/⌘ K', disabled: !active, run: () => setSearchOpen(true)},
        {label: 'Settings…', disabled: !ready, run: () => setSettingsOpen(true)},
      ]},
      {label: 'View', actions: [
        ...views.map(item => ({label: item === 'Hex' ? 'Hex / Raw Metadata' : item, checked: view === item, disabled: !active, run: () => setView(item)})), null,
        {label: 'Word Wrap', checked: wrap, run: () => setEditorSettings(previous => ({...previous, wrap: !previous.wrap}))},
        {label: 'Light Appearance', checked: theme === 'light', run: () => setTheme(theme === 'dark' ? 'light' : 'dark')},
      ]},
      {label: 'Help', actions: [{label: 'About ILens', run: () => setHelpOpen(true)}]},
    ]} />
    {errors.length > 0 && <div className="errors py-0 px-4 max-h-45 overflow-auto bg-destructive-bg text-destructive border-b border-solid border-b-border" role="alert">{errors.map((error, i) => <div className="error-item flex justify-between gap-[15px] py-2.5 px-0 text-[12px] [&_strong]:font-[550] [&_details]:text-[10px] [&_details]:mt-[3px] [&_code]:[overflow-wrap:anywhere]" key={`${error.code}-${i}`}><div><strong>{error.message}</strong><details><summary>Technical details</summary><code>{error.code}: {error.detail}</code></details></div><button className={cn(ui.iconButton)} aria-label="Dismiss error" onClick={() => setErrors(previous => previous.filter((_, index) => index !== i))}><X size={15} /></button></div>)}</div>}
    {active && <AssemblyWarnings info={active.info} onNavigate={token => {select(token); setView('Metadata')}} />}
    <div className="workspace flex-1 grid grid-cols-[var(--left-width)_1px_minmax(200px,1fr)_1px_var(--right-width)] min-h-0 max-[1000px]:grid-cols-[240px_1px_minmax(200px,1fr)] max-[1000px]:[&>.pane-handle:nth-of-type(2)]:hidden" style={{'--left-width': `${leftWidth}px`, '--right-width': `${rightWidth}px`} as CSSProperties}>
      <aside className="explorer-pane flex flex-col min-w-0 min-h-0 bg-surface">
        <div className="pane-heading h-10.5 min-h-10.5 flex items-center justify-between py-0 px-[17px] border-b border-solid border-b-border font-sans text-[12px] font-[500] leading-normal tracking-[0px] text-foreground"><span>Assembly Explorer</span><span className="count-pill tracking-[0px] bg-hover min-w-[19px] text-center text-muted-foreground text-[10px] rounded-[4px] py-px px-[5px]">{assemblies.length}</span></div>
        {active ? <>
          <div className="assembly-selector flex items-center gap-[7px] py-2 px-2.5 border-b border-solid border-b-border text-foreground [&_select]:min-w-0 [&_select]:flex-1 [&_select]:bg-surface [&_select]:text-text [&_select]:border-0 [&_select]:py-1 [&_select]:px-0 [&_select]:text-[12px]"><PackageSelect assemblies={assemblies} primary={primary} onSelect={id => {setPrimary(id); setSelected(0); setView('Metadata')}} /><button className={cn(ui.iconButton)} aria-label="Close workspace" title="Close workspace and release memory" onClick={() => reset()}><X size={14} /></button></div>
          <Explorer tree={tree} selected={selected} onSelect={select} onExport={token => {select(token); setExportOpen('source')}} />
          <div className="explorer-foot flex gap-2 items-center py-[11px] px-3.5 border-t border-solid border-t-border text-[9px] text-faint"><HardDrive size={13} /><span>{bytes(assemblies.reduce((sum, a) => sum + a.info.size, 0))} in workspace</span></div>
        </> : <div className="empty-explorer flex items-center flex-col text-faint py-10.5 px-7 text-center [&_p]:mt-4 [&_p]:mr-0 [&_p]:mb-1.5 [&_p]:ml-0 [&_p]:text-[13px] [&_p]:font-medium [&_p]:text-foreground [&>span]:max-w-47.5 [&>span]:text-[12px] [&>span]:leading-[1.7] [&_.text-button]:mt-[21px]"><Layers size={25} /><p>No assemblies loaded</p><span>Open a DLL or EXE to explore its structure.</span><button className="inline-flex gap-1.5 items-center border-0 bg-transparent p-0 text-[12px] text-foreground hover:underline hover:underline-offset-1" onClick={() => filePicker.current?.click()}>Open an assembly <ArrowRight size={13} /></button></div>}
      </aside>
      <PaneHandle name="assembly explorer" width={leftWidth} onChange={setLeftWidth} />
      <main className="center-pane flex flex-col min-w-0 min-h-0 bg-editor relative">
        {!active ? <div className="welcome m-auto w-full py-8 px-[clamp(25px,5vw,80px)] overflow-auto [scrollbar-width:thin] max-w-200 [@media(min-width:1600px)]:max-w-240 [@media(min-width:1600px)]:pl-25 [@media(min-width:1600px)]:pr-25 [@media(min-width:1600px)]:[&_h1]:text-[36px] max-[1200px]:p-7.5 max-[1200px]:[&_h1]:text-[32px] max-[1000px]:max-w-162.5 [@media(max-height:760px)]:pt-5.5 [@media(max-height:760px)]:pb-5.5 [@media(max-height:760px)]:[&_h1]:mt-[15px] [@media(max-height:760px)]:[&_h1]:text-[30px]">
          <div className="welcome-eyebrow flex items-center gap-2.5 font-sans text-[12px] font-[500] leading-normal tracking-[0px] text-muted-foreground">Assembly workspace</div>
          <h1>Look inside<br /><span>your .NET assemblies.</span></h1>
          <p className="welcome-intro text-muted-foreground mt-0 mr-0 mb-[33px] ml-0 text-[14px] leading-[1.7] [@media(max-height:760px)]:mb-5">A workspace for exploring metadata, inspecting IL,<br className="wide-only" /> and reconstructing readable C#.</p>
          <button className={cn(`drop-zone w-full text-text flex flex-col items-center [transition:border-color_.15s,background_.15s] border border-dashed border-input rounded-[8px] bg-background pt-8 pr-5 pb-5.5 pl-5 hover:border-ring hover:bg-accent [&_strong]:text-[14px] [&_strong]:font-medium [&>span:not(.upload-icon)]:text-muted-foreground [&>span:not(.upload-icon)]:mt-[5px] [&>span:not(.upload-icon)]:text-[13px] [&_em]:not-italic [&_em]:text-foreground [&_em]:underline [&_em]:underline-offset-1 [@media(max-height:760px)]:pt-5 [@media(max-height:760px)]:pb-3.5 ${loadingFiles ? 'loading' : ''}`)} onClick={() => filePicker.current?.click()} disabled={!ready || loadingFiles}>
            <span className="upload-icon flex mb-4.5 text-muted-foreground bg-muted border border-solid border-border rounded-[8px] p-3 [@media(max-height:760px)]:p-2.5 [@media(max-height:760px)]:mb-3">{loadingFiles ? <LoaderCircle className="spin animate-spin motion-reduce:animate-none" size={27} /> : <Upload size={27} />}</span>
            <strong>{loadingFiles ? 'Opening your assembly…' : 'Drop a managed .NET DLL or EXE here'}</strong>
            <span>or <em>browse files</em> to get started</span>
            <div className="drop-footer flex items-center gap-1.5 text-faint flex-wrap justify-center text-[11px] mt-6 [&>span]:py-px [&>span]:px-[5px] [&>span]:border [&>span]:border-solid [&>span]:border-border [&>span]:rounded-[3px] [&>span]:font-mono [&>span]:text-[10px] [&>span]:font-normal [&>span]:leading-normal [&>i]:inline-block [&>i]:h-[11px] [&>i]:w-px [&>i]:bg-border [&>i]:my-0 [&>i]:mx-[5px] [@media(max-height:760px)]:mt-4.5"><span>.dll</span><span>.exe</span><i />Multiple files supported · up to 64 MB each</div>
          </button>
          <button className="sample-button flex items-center justify-center gap-[9px] border-0 bg-transparent mt-4 mr-auto mb-0 ml-auto text-muted-foreground text-[12px] rounded-md py-2 px-3 hover:text-foreground hover:bg-accent [&_svg:last-child]:ml-2.5" onClick={() => void openSample()} disabled={!ready || loadingFiles}><FileCode2 size={15} />Explore a compiled sample<ArrowRight size={14} /></button>
          <div className="feature-row grid grid-cols-[repeat(2,1fr)] gap-[15px] border-t border-solid border-t-border mt-8 pt-6 [&>div]:flex [&>div]:flex-col [&>div]:gap-[7px] [&_svg]:text-faint [&_svg]:mb-1 [&_strong]:font-medium [&_strong]:text-[12px] [&_span]:text-faint [&_span]:text-[11px] max-[1200px]:grid-cols-[1fr] max-[1200px]:gap-3 max-[1200px]:mt-[25px] max-[1200px]:[&>div]:grid max-[1200px]:[&>div]:grid-cols-[20px_1fr] max-[1200px]:[&>div]:gap-y-0.5 max-[1200px]:[&>div]:gap-x-2.5 max-[1200px]:[&_span]:[grid-column:2] max-[1200px]:[&_svg]:[grid-row:span_2] max-[1000px]:grid-cols-[1fr_1fr] [@media(max-height:760px)]:mt-[25px] [@media(max-height:760px)]:pt-[17px]"><div><Braces size={17} /><strong>Understand the structure</strong><span>Types, members, and references</span></div><div><Binary size={17} /><strong>Inspect every instruction</strong><span>Decoded CIL, with resolved tokens</span></div></div>
        </div> : <>
          <div className="document-tab h-[39px] min-h-[39px] border-b border-solid border-b-border flex items-center gap-[9px] py-0 px-[17px] text-text bg-surface text-[12px] [&_svg]:text-foreground [&>span:first-of-type]:max-w-[50%] [&>span:first-of-type]:text-ellipsis [&>span:first-of-type]:overflow-hidden [&>span:first-of-type]:whitespace-nowrap"><FileCode2 size={14} /><span>{selectedLabel}</span><span className="document-language text-faint ml-auto overflow-hidden whitespace-nowrap text-ellipsis text-[11px]">{active.info.identity.name}</span></div>
          <div className="code-toolbar flex justify-between border-b border-solid border-b-border py-0 px-3 h-11 min-h-11 items-center"><div className="view-tabs flex items-stretch bg-muted rounded-md h-8 p-[3px] gap-0.5 [&_button]:flex [&_button]:items-center [&_button]:gap-[5px] [&_button]:bg-transparent [&_button]:text-muted-foreground [&_button]:border [&_button]:border-solid [&_button]:border-transparent [&_button]:rounded-[4px] [&_button]:text-[12px] [&_button]:py-0 [&_button]:px-2.5 [&_button.active]:text-foreground [&_button.active]:bg-background [&_button.active]:border-b-transparent [&_button.active]:shadow-[0_1px_2px_#00000014]" role="tablist" aria-label="Code view">{views.map(v => <button role="tab" aria-selected={view === v} className={cn(view === v ? "active" : "")} key={v} onClick={() => setView(v)}>{v === 'C#' && <Braces size={13} />}{v === 'IL' && <Binary size={13} />}{v}</button>)}</div><span className="code-tools ml-auto flex items-center gap-1">
            {view === 'IL' && selected !== null && selected >>> 24 === 6 && <button className={cn(ui.button, ui.subtle, "h-7 gap-1.5 px-2 text-[12px]")} disabled={busy || loadingFiles || !method?.body || methodAssembly !== active.id || method.token !== selected} onClick={() => setEditOpen(true)}><Pencil size={13} />Edit IL</button>}
            <button className={cn(`${ui.iconButton} ${wrap ? 'enabled' : ''}`)} aria-label="Toggle word wrap" aria-pressed={wrap} title="Word wrap" onClick={() => setEditorSettings(previous => ({...previous, wrap: !previous.wrap}))}><WrapText size={15} /></button><button className={cn(ui.iconButton)} aria-label="Copy code" title="Copy code" onClick={() => void copy()}>{copied ? <Check size={15} /> : <Copy size={15} />}</button></span></div>
          <div className="breadcrumbs flex gap-[7px] items-center h-[33px] min-h-[33px] py-0 px-4 border-b border-solid border-b-border text-faint text-[11px] [&>span]:whitespace-nowrap [&>span]:overflow-hidden [&>span]:text-ellipsis [&_code]:ml-auto [&_code]:whitespace-nowrap [&_code]:text-muted-foreground [&_code]:font-mono [&_code]:text-[9px] [&_code]:font-normal [&_code]:leading-normal"><Box size={12} /><span>{active.info.identity.name}</span><ChevronRight size={11} /><span>{activeDeclaration?.full_name ?? selectedLabel}</span>{selected && selected > 0 ? <code>{tokenHex(selected)}</code> : null}</div>
          {method?.quality === 'annotated_il' && view === 'C#' && <div className="limitation-banner flex items-start gap-2 py-[9px] px-3.5 bg-muted border-b border-solid border-b-border text-foreground text-[12px] [&>svg]:shrink-0 [&>svg]:mt-0.5 [&_button]:shrink-0 [&_button]:text-foreground [&_button]:bg-transparent [&_button]:border-0 [&_button]:ml-auto [&_button]:text-[11px]"><BookOpen size={14} /><span>{method.diagnostics[0]?.detail ?? 'This method is shown as annotated IL.'}</span><button onClick={() => setView('IL')}>View IL</button></div>}
          <div className="viewer-container relative flex-1 min-h-0"><CodeViewer text={code} language={view === 'C#' && method?.quality === 'annotated_il' ? 'IL' : view} settings={editorSettings} theme={theme} onNavigate={select} />{busy && <div className="viewer-loading absolute top-3 right-[17px] flex gap-[9px] items-center py-[9px] px-3 bg-surface-raised border border-solid border-border rounded-[6px] text-foreground shadow-[var(--shadow)] text-[11px]" role="status"><LoaderCircle size={17} className="spin animate-spin motion-reduce:animate-none" />Analyzing method…</div>}</div>
          <div className="code-foot h-7 min-h-7 flex justify-between items-center py-0 px-[15px] border-t border-solid border-t-border text-faint gap-2.5 font-mono text-[10px] font-normal leading-normal [&>span:first-child]:overflow-hidden [&>span:first-child]:whitespace-nowrap [&>span:first-child]:text-ellipsis max-[1000px]:text-[8px]"><span>{view === 'C#' ? method?.quality === 'annotated_il' ? 'Annotated CIL · reconstruction limitation' : 'Reconstructed C# · synthetic local names' : view === 'IL' ? 'Original CIL · Ctrl/⌘-click a token to navigate' : 'Read-only metadata'}</span><span>{code.split('\n').length} lines</span></div>
        </>}
      </main>
      <PaneHandle name="inspector" width={rightWidth} onChange={setRightWidth} reverse />
      <aside className="inspector-pane flex flex-col min-w-0 min-h-0 bg-surface max-[1000px]:hidden"><div className="pane-heading h-10.5 min-h-10.5 flex items-center justify-between py-0 px-[17px] border-b border-solid border-b-border font-sans text-[12px] font-[500] leading-normal tracking-[0px] text-foreground"><span>Inspector</span><Braces size={13} /></div>
        {active ? <div className="inspector-content overflow-auto [scrollbar-width:thin] [scrollbar-color:var(--border)_transparent] py-5 px-4">
          <div className="inspector-title [&_h2]:font-mono [&_h2]:font-medium [&_h2]:break-words [&_h2]:mt-2.5 [&_h2]:mr-0 [&_h2]:mb-[17px] [&_h2]:ml-0 [&_h2]:text-[13px]"><span className="type-badge font-sans text-[11px] font-normal leading-normal text-secondary-foreground bg-secondary rounded-[4px] py-0.5 px-1.5">{activeDeclaration?.kind ?? 'assembly'}</span><h2>{selectedLabel}</h2></div>
          {resolvedDefinition && (resolvedDefinition.assembly !== primary || resolvedDefinition.token !== selected) && <button className={cn(ui.button, ui.primary, "definition-button mt-0 mr-0 mb-4.5 ml-0 text-[12px]")} onClick={() => {setPrimary(resolvedDefinition.assembly); setSelected(resolvedDefinition.token); setView('C#')}}><ArrowRight size={13} />Go to definition</button>}
          <dl className="properties grid grid-cols-[85px_minmax(0,1fr)] gap-y-2 gap-x-[5px] mt-0 mr-0 mb-5.5 ml-0 text-[12px] [&_dt]:text-faint [&_dd]:m-0 [&_dd]:break-words [&_dd]:font-mono [&_dd]:text-[11px] [&_dd]:font-normal [&_dd]:leading-[1.65] [&_dd]:text-foreground [&_dd.accent-text]:text-foreground"><dt>Assembly</dt><dd>{active.info.identity.name}</dd><dt>Version</dt><dd>{active.info.identity.version}</dd>{selected !== null && selected > 0 && <><dt>Token</dt><dd className="accent-text">{tokenHex(selected)}</dd></>}{metadata?.rva ? <><dt>RVA</dt><dd>{String(metadata.rva)}</dd></> : null}<dt>.NET target</dt><dd data-testid="target-framework" title={active.info.target_framework.moniker ?? 'No target framework attribute was declared.'}>{active.info.target_framework.display_name}</dd><dt>CLR metadata</dt><dd title="Metadata format version; this is not the target .NET version.">{active.info.runtime}</dd><dt>Obfuscation</dt><dd data-testid="obfuscation-status" title="Metadata markers and naming heuristics. No markers found does not rule out obfuscation.">{obfuscationLabels[active.info.obfuscation.status]}</dd><dt>Architecture</dt><dd>{active.info.machine}</dd><dt>File size</dt><dd>{bytes(active.info.size)}</dd></dl>
          {method?.body && <><div className="inspector-section-title border-t border-solid border-t-border pt-4 mt-4.5 mb-3.5 flex items-center justify-between font-sans text-[12px] font-[500] leading-normal tracking-[0px] text-foreground [&_.text-button]:tracking-[0px] [&_.text-button]:font-sans [&_.text-button]:text-[11px] [&_.text-button]:font-normal [&_.text-button]:leading-normal">Method analysis</div><dl className="properties grid grid-cols-[85px_minmax(0,1fr)] gap-y-2 gap-x-[5px] mt-0 mr-0 mb-5.5 ml-0 text-[12px] [&_dt]:text-faint [&_dd]:m-0 [&_dd]:break-words [&_dd]:font-mono [&_dd]:text-[11px] [&_dd]:font-normal [&_dd]:leading-[1.65] [&_dd]:text-foreground [&_dd.accent-text]:text-foreground"><dt>CIL size</dt><dd>{method.body.code_size} bytes</dd><dt>Instructions</dt><dd>{method.body.instructions.length}</dd><dt>Basic blocks</dt><dd>{method.cfg?.blocks.length ?? '—'}</dd><dt>Maximum stack</dt><dd>{method.stack?.maximum ?? method.body.max_stack}</dd><dt>Exception regions</dt><dd>{method.body.exceptions.length}</dd></dl></>}
          <div className="inspector-section-title border-t border-solid border-t-border pt-4 mt-4.5 mb-3.5 flex items-center justify-between font-sans text-[12px] font-[500] leading-normal tracking-[0px] text-foreground [&_.text-button]:tracking-[0px] [&_.text-button]:font-sans [&_.text-button]:text-[11px] [&_.text-button]:font-normal [&_.text-button]:leading-normal">References<span>{active.overview.references.length}</span></div>
          <div className="reference-summary text-faint mt-0 mr-0 mb-[9px] ml-0 text-[11px]">{active.overview.references.filter(r => r.status === 'resolved').length} resolved · {active.overview.references.filter(r => r.status !== 'resolved').length} unavailable</div>
          {active.overview.references.slice(0, 12).map(r => <button key={r.token} className="reference-row flex gap-2 items-center border-0 bg-transparent text-muted-foreground w-full py-[5px] px-0 text-left font-mono text-[11px] font-normal leading-normal [&>span:nth-child(2)]:whitespace-nowrap [&>span:nth-child(2)]:overflow-hidden [&>span:nth-child(2)]:text-ellipsis [&>span:nth-child(2)]:flex-1 [&>span:last-child]:text-faint [&>span:last-child]:text-[10px] hover:text-foreground" onClick={() => {select(r.token); setView('Metadata')}} title={`${r.identity.name}, ${r.identity.version}`}><span className={cn(`reference-dot w-[5px] h-[5px] border border-solid border-faint rounded-full shrink-0 [&.resolved]:bg-foreground [&.resolved]:border-foreground ${r.status}`)} /><span>{r.identity.name}</span><span>{r.status === 'resolved' ? 'local' : '—'}</span></button>)}
          <button className="inspector-add inline-flex gap-1.5 items-center border-0 bg-transparent p-0 text-[12px] text-foreground mt-3 hover:underline hover:underline-offset-1" onClick={() => dependencyPicker.current?.click()}><Plus size={13} />Add dependency assemblies</button>
          {outgoing.length > 0 && <><div className="inspector-section-title border-t border-solid border-t-border pt-4 mt-4.5 mb-3.5 flex items-center justify-between font-sans text-[12px] font-[500] leading-normal tracking-[0px] text-foreground [&_.text-button]:tracking-[0px] [&_.text-button]:font-sans [&_.text-button]:text-[11px] [&_.text-button]:font-normal [&_.text-button]:leading-normal">Referenced tokens<span>{outgoing.length}</span></div>{outgoing.map(([token, name]) => <button className="token-link hover:text-foreground bg-transparent border-0 flex items-center gap-[7px] text-left text-muted-foreground w-full py-1.5 px-0 text-[11px] [&>span]:flex-1 [&>span]:overflow-hidden [&>span]:text-ellipsis [&>span]:whitespace-nowrap [&>code]:text-faint [&>code]:font-mono [&>code]:text-[9px] [&>code]:font-normal [&>code]:leading-normal [&>svg]:shrink-0" key={token} title={name} onClick={() => {select(token); setView('Metadata')}}><LinkIcon size={12} /><span>{name}</span><code>{tokenHex(token)}</code></button>)}</>}
          {selected !== null && selected > 0 && <><div className="inspector-section-title border-t border-solid border-t-border pt-4 mt-4.5 mb-3.5 flex items-center justify-between font-sans text-[12px] font-[500] leading-normal tracking-[0px] text-foreground [&_.text-button]:tracking-[0px] [&_.text-button]:font-sans [&_.text-button]:text-[11px] [&_.text-button]:font-normal [&_.text-button]:leading-normal">Used by<button className="inline-flex gap-1.5 items-center border-0 bg-transparent p-0 text-[12px] text-foreground hover:underline hover:underline-offset-1" onClick={() => {if (xrefBusy) {xrefRequest.current?.abort(); setXrefBusy(false)} else void findXrefs()}}>{xrefBusy ? 'Cancel' : 'Find references'}</button></div>{xrefStatus && <p className="xref-status text-faint text-[11px]">{xrefStatus}</p>}{xrefs?.map((r, i) => <button className="token-link hover:text-foreground bg-transparent border-0 flex items-center gap-[7px] text-left text-muted-foreground w-full py-1.5 px-0 text-[11px] [&>span]:flex-1 [&>span]:overflow-hidden [&>span]:text-ellipsis [&>span]:whitespace-nowrap [&>code]:text-faint [&>code]:font-mono [&>code]:text-[9px] [&>code]:font-normal [&>code]:leading-normal [&>svg]:shrink-0" key={`${r.token}-${i}`} onClick={() => {select(r.token); setView('IL')}}><Code2 size={12} /><span>{r.name}</span></button>)}{xrefs?.length === 0 && !xrefBusy && <p className="inspector-note text-faint text-[11px]">No direct CIL token references found.</p>}</>}
        </div> : <div className="empty-inspector py-9.5 px-5.5 [&_h3]:mt-5 [&_h3]:mr-0 [&_h3]:mb-[7px] [&_h3]:ml-0 [&_h3]:text-[14px] [&_h3]:font-medium [&_h3]:text-foreground [&>p]:text-faint [&>p]:max-w-55 [&>p]:text-[12px] [&>p]:leading-[1.7] max-[1200px]:py-[25px] max-[1200px]:px-4"><div className="inspector-placeholder inline-flex text-faint bg-surface-raised border border-solid border-border rounded-[8px] p-3"><Braces size={26} /></div><h3>A little more context.</h3><p>Select a type or member to see its metadata, signature, and references here.</p><div className="supported-label mt-7.5 mr-0 mb-[13px] ml-0 text-faint font-sans text-[12px] font-[500] leading-normal tracking-[0px]">Supported inputs</div><span className="support-chip inline-block text-muted-foreground border border-solid border-border py-[3px] px-[7px] mt-0 mr-1 mb-1.5 ml-0 text-[11px] rounded-md bg-muted">.NET Framework</span><span className="support-chip inline-block text-muted-foreground border border-solid border-border py-[3px] px-[7px] mt-0 mr-1 mb-1.5 ml-0 text-[11px] rounded-md bg-muted">Modern .NET</span><span className="support-chip inline-block text-muted-foreground border border-solid border-border py-[3px] px-[7px] mt-0 mr-1 mb-1.5 ml-0 text-[11px] rounded-md bg-muted">IL-only assemblies</span></div>}
      </aside>
    </div>
    <footer className="statusbar h-7 min-h-7 flex items-center gap-2 justify-between border-t border-solid border-t-border bg-surface py-0 px-3.5 text-muted-foreground text-[11px]"><span className="status-message flex items-center gap-2 whitespace-nowrap overflow-hidden text-ellipsis">{busy || loadingFiles || xrefBusy ? <LoaderCircle size={12} className="spin animate-spin motion-reduce:animate-none" /> : <span className="status-dot inline-block w-[5px] h-[5px] rounded-full shrink-0 bg-muted-foreground shadow-none" />}{status}</span>{(busy || loadingFiles) && <button className="stop-button bg-transparent border-0 whitespace-nowrap text-foreground text-[11px]" onClick={() => reset('Analysis stopped. Workspace memory released.')}>Stop analysis</button>}<span className="status-right flex items-center gap-[7px] whitespace-nowrap text-faint text-[10px]">v0.1.0</span></footer>
    {dragging && <div className="drag-overlay fixed [inset:8px] z-100 rounded-[10px] flex flex-col gap-5 items-center justify-center pointer-events-none bg-background border-2 border-dashed border-ring text-foreground [&_strong]:text-[22px] [&_span]:text-[12px] [&_span]:text-muted-foreground"><Upload size={35} /><strong>{active ? 'Drop to add dependency assemblies' : 'Drop your assemblies to begin'}</strong></div>}
    {searchOpen && <div className="modal-backdrop fixed [inset:0] z-80 bg-[#0008] flex justify-center items-start pt-[min(15vh,130px)] [backdrop-filter:none]" onClick={() => setSearchOpen(false)}><div className="search-dialog w-[min(680px,90vw)] border border-solid border-border shadow-[var(--shadow)] overflow-hidden bg-popover rounded-[8px]" role="dialog" aria-modal="true" aria-label="Search assembly" onClick={e => e.stopPropagation()}><div className="search-input-row flex gap-[13px] items-center p-4 text-muted-foreground border-b border-solid border-b-border [&_input]:flex-1 [&_input]:min-w-0 [&_input]:border-0 [&_input]:bg-transparent [&_input]:text-text [&_input]:text-[14px] [&_input]:[outline:none]"><Search size={19} /><input ref={searchInput} aria-label="Search members and strings" placeholder={active ? 'Search types, methods, fields, or strings…' : 'Open an assembly to search its metadata'} value={query} onChange={e => setQuery(e.target.value)} maxLength={256} /><button className={cn(ui.iconButton)} aria-label="Close search" onClick={() => setSearchOpen(false)}><X size={17} /></button></div><div className="search-scope py-[9px] px-[17px] text-faint flex justify-between bg-surface text-[12px]">{active ? active.info.identity.name : 'No assembly loaded'}<span>{searching ? 'Searching…' : `${results.length}${results.length === 250 ? '+' : ''} results`}</span></div><div className="search-results max-h-102.5 overflow-auto [&>button]:flex [&>button]:items-center [&>button]:gap-3 [&>button]:text-left [&>button]:bg-transparent [&>button]:border-0 [&>button]:border-b [&>button]:border-solid [&>button]:border-b-border [&>button]:w-full [&>button]:py-[13px] [&>button]:px-[17px] [&>button:hover]:bg-hover [&>button>span:nth-child(2)]:min-w-0 [&>button>span:nth-child(2)]:flex-1 [&_strong]:block [&_strong]:font-medium [&_strong]:overflow-hidden [&_strong]:text-ellipsis [&_strong]:whitespace-nowrap [&_strong]:text-[13px] [&_small]:block [&_small]:text-faint [&_small]:whitespace-nowrap [&_small]:text-ellipsis [&_small]:overflow-hidden [&_small]:mt-0.5 [&_small]:text-[11px] [&_code]:font-mono [&_code]:text-[9px] [&_code]:font-normal [&_code]:leading-normal [&_code]:text-faint [&_svg]:text-muted-foreground">{results.map((r, i) => <button key={`${r.token}-${i}`} onClick={() => {select(r.token); setView(r.kind === 'method' || r.kind === 'constructor' ? 'C#' : 'Metadata')}}><span className="search-kind w-7 min-w-7 h-7 flex items-center justify-center rounded-[5px] text-foreground bg-accent-dim">{r.kind === 'string' ? '“ ”' : <Code2 size={15} />}</span><span><strong>{r.name}</strong><small>{r.context}</small></span><code>{tokenHex(r.token)}</code><ArrowRight size={14} /></button>)}{!results.length && <div className="search-empty text-center text-faint py-[55px] px-5 text-[13px]">{query.trim() ? searching ? 'Searching local metadata…' : 'No matching members or strings.' : 'Search metadata names, namespaces, user strings, or a token.'}</div>}</div><div className="dialog-footer flex items-center gap-[5px] py-[9px] px-4 border-t border-solid border-t-border text-faint text-[11px] [&>span]:ml-auto"><kbd>esc</kbd> to close<span>Local metadata search</span></div></div></div>}
    {projectOpen && primary && <ProjectExportDialog client={getClient()} assemblies={assemblies} primary={primary} onClose={() => setProjectOpen(false)} />}
    {exportOpen && active && <ExportDialog key={`${active.id}-${exportOpen}`} mode={exportOpen} client={getClient()} assembly={active.id} info={active.info} filename={active.filename} member={activeDeclaration} method={busy || methodAssembly !== active.id ? null : method} onClose={() => setExportOpen(null)} />}
    {editOpen && active && selected && selected > 0 && <ILEditorDialog key={`${active.id}-${selected}`} client={getClient()} assembly={active.id} token={selected} name={selectedLabel} settings={editorSettings} theme={theme} onClose={() => setEditOpen(false)} onExport={() => {setEditOpen(false); setExportOpen('binary')}} />}
    {settingsOpen && <SettingsDialog settings={editorSettings} onChange={setEditorSettings} appearance={theme} onAppearance={setTheme} onClose={() => setSettingsOpen(false)} />}
    {helpOpen && <div className="modal-backdrop fixed [inset:0] z-80 bg-[#0008] flex justify-center items-start pt-[min(15vh,130px)] [backdrop-filter:none]" onClick={() => setHelpOpen(false)}><div className="help-dialog w-122.5 max-w-[90vw] border border-solid border-border p-8 relative shadow-[var(--shadow)] rounded-[8px] bg-popover [&_h2]:tracking-[-.7px] [&_h2]:mt-[21px] [&_h2]:mr-0 [&_h2]:mb-[15px] [&_h2]:ml-0 [&_h2]:text-[24px] [&_h2]:font-semibold [&_p]:text-muted-foreground [&_p]:text-[13px] [&_p]:leading-[1.7] [&>.button]:mt-4" role="dialog" aria-modal="true" aria-label="About ILens" onClick={e => e.stopPropagation()}><button className={cn(ui.iconButton, "close-dialog absolute right-3.5 top-3.5")} aria-label="Close about" onClick={() => setHelpOpen(false)}><X size={18} /></button><span className="brand-mark inline-flex items-center justify-center w-[31px] h-[31px] text-foreground bg-transparent border border-solid border-border rounded-md"><Code2 size={25} /></span><h2>Understand the binary.</h2><p>ILens is a static .NET decompiler.</p><p>Open a managed DLL or EXE, choose a method, and compare reconstructed C# with its original CIL. Use <kbd>⌘/Ctrl K</kbd> to search metadata, and <kbd>⌘/Ctrl F</kbd> inside the code viewer.</p><p>This is an initial decompiler release. Simple methods and control-flow patterns reconstruct into C#-like code. Complex async, iterator, closure, and exception patterns may remain annotated IL. Local variable names are synthetic; this is not the original source.</p><p>Missing dependencies do not block analysis. Add them explicitly to resolve assembly identities. Nothing is downloaded automatically, and loaded assemblies are never executed.</p><button className={cn(ui.button, ui.primary)} onClick={() => setHelpOpen(false)}>Back to workspace<ArrowRight size={14} /></button></div></div>}
  </div>
}
function PackageSelect({assemblies, primary, onSelect}: {assemblies: WorkspaceAssembly[]; primary: number | null; onSelect: (id: number) => void}) {return <><Layers size={14} /><select aria-label="Primary assembly" value={primary ?? ''} onChange={e => onSelect(Number(e.target.value))}>{assemblies.map(a => <option key={a.id} value={a.id}>{a.filename}</option>)}</select></>}
