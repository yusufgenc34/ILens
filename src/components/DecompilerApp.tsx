'use client'
import { useCallback, useEffect, useMemo, useRef, useState, type ChangeEvent, type DragEvent, type CSSProperties } from 'react'
import { ArrowRight, Binary, BookOpen, Box, Braces, Check, ChevronRight, CircleHelp, Code2, Copy, FileCode2, FolderOpen, HardDrive, Layers, Link as LinkIcon, LoaderCircle, Moon, Plus, Search, Settings2, Sun, Upload, WrapText, X } from 'lucide-react'
import { DecompilerClient } from '../lib/rpc'
import { diagnostic, jsonText, tokenHex, type Declaration, type Diagnostic, type Loaded, type MethodAnalysis, type Overview, type SearchHit, type XrefPage } from '../lib/types'
import { createTree } from '../lib/tree'
import Explorer from './Explorer'
import CodeViewer from './CodeViewer'
import PaneHandle from './PaneHandle'
import SettingsDialog from './SettingsDialog'
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
  const [metadata, setMetadata] = useState<Record<string, unknown> | null>(null)
  const [declaration, setDeclaration] = useState('')
  const [status, setStatus] = useState('Ready to open an assembly')
  const [busy, setBusy] = useState(false)
  const [loadingFiles, setLoadingFiles] = useState(false)
  const [errors, setErrors] = useState<Diagnostic[]>([])
  const [theme, setTheme] = useState('dark')
  const [editorSettings, setEditorSettings] = useState(defaultEditorSettings)
  const [settingsOpen, setSettingsOpen] = useState(false)
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
    setAssemblies([]); setPrimary(null); setSelected(null); setMethod(null); setMetadata(null); setDeclaration(''); setBusy(false); setLoadingFiles(false); setXrefs(null); setXrefBusy(false); setStatus(message)
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
          setMethod(analysis)
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
  async function openSample() {try {const response = await fetch('/samples/ILens.Patterns.dll'); if (!response.ok) throw new Error('The sample assembly could not be loaded.'); const blob = await response.blob(); await openFiles([new File([blob], 'ILens.Patterns.dll')])} catch (error) {report(error)}}
  const resolvedDefinition = metadata?.resolved_definition as {assembly: number; token: number} | null | undefined
  const selectedLabel = activeDeclaration?.name ?? (selected === -1 ? 'Assembly references' : selected === -2 ? 'Manifest resources' : selected === 0 ? 'Assembly overview' : metadata?.name ? String(metadata.name) : 'Select a member')
  return <div data-ready={ready} className={`application ${theme}`} onDragOver={e => {e.preventDefault(); if (e.dataTransfer.types.includes('Files')) setDragging(true)}} onDragLeave={e => {if (!e.currentTarget.contains(e.relatedTarget as Node)) setDragging(false)}} onDrop={drop}>
    <input ref={filePicker} type="file" disabled={!ready || loadingFiles} accept=".dll,.exe" multiple hidden aria-label="Open assembly files" onChange={e => onFiles(e, false)} />
    <input ref={dependencyPicker} type="file" disabled={!ready || loadingFiles} accept=".dll,.exe" multiple hidden aria-label="Add dependency files" onChange={e => onFiles(e, true)} />
    <header className="toolbar">
      <a className="brand" href="/" aria-label="ILens home"><span className="brand-mark"><Code2 size={21} /></span>ILens<span className="preview-label">PREVIEW</span></a>
      <div className="toolbar-divider" />
      <button className="button open-button" onClick={() => filePicker.current?.click()} disabled={!ready || loadingFiles}><FolderOpen size={15} />Open Assembly<kbd>⌘ O</kbd></button>
      <button className="button subtle dependencies-button" onClick={() => dependencyPicker.current?.click()} disabled={!ready || loadingFiles}><Plus size={15} />Add Dependencies</button>
      <div className="toolbar-spacer" />
      <button className="search-trigger" onClick={() => setSearchOpen(true)}><Search size={14} /><span>Search assembly…</span><kbd>⌘ K</kbd></button>
      <button className="icon-button" title="Switch color theme" aria-label="Switch color theme" onClick={() => setTheme(theme === 'dark' ? 'light' : 'dark')}>{theme === 'dark' ? <Sun size={17} /> : <Moon size={17} />}</button>
      <button className="icon-button" title="Settings" aria-label="Settings" disabled={!ready} onClick={() => setSettingsOpen(true)}><Settings2 size={17} /></button>
      <button className="icon-button" title="About ILens" aria-label="About ILens" onClick={() => setHelpOpen(true)}><CircleHelp size={17} /></button>
    </header>
    {errors.length > 0 && <div className="errors" role="alert">{errors.map((error, i) => <div className="error-item" key={`${error.code}-${i}`}><div><strong>{error.message}</strong><details><summary>Technical details</summary><code>{error.code}: {error.detail}</code></details></div><button className="icon-button" aria-label="Dismiss error" onClick={() => setErrors(previous => previous.filter((_, index) => index !== i))}><X size={15} /></button></div>)}</div>}
    {active && <AssemblyWarnings info={active.info} onNavigate={token => {select(token); setView('Metadata')}} />}
    <div className="workspace" style={{'--left-width': `${leftWidth}px`, '--right-width': `${rightWidth}px`} as CSSProperties}>
      <aside className="explorer-pane">
        <div className="pane-heading"><span>Assembly Explorer</span><span className="count-pill">{assemblies.length}</span></div>
        {active ? <>
          <div className="assembly-selector"><PackageSelect assemblies={assemblies} primary={primary} onSelect={id => {setPrimary(id); setSelected(0); setView('Metadata')}} /><button className="icon-button" aria-label="Close workspace" title="Close workspace and release memory" onClick={() => reset()}><X size={14} /></button></div>
          <Explorer tree={tree} selected={selected} onSelect={select} />
          <div className="explorer-foot"><HardDrive size={13} /><span>{bytes(assemblies.reduce((sum, a) => sum + a.info.size, 0))} in workspace</span></div>
        </> : <div className="empty-explorer"><Layers size={25} /><p>No assemblies loaded</p><span>Open a DLL or EXE to explore its structure.</span><button className="text-button" onClick={() => filePicker.current?.click()}>Open an assembly <ArrowRight size={13} /></button></div>}
      </aside>
      <PaneHandle name="assembly explorer" width={leftWidth} onChange={setLeftWidth} />
      <main className="center-pane">
        {!active ? <div className="welcome">
          <div className="welcome-eyebrow">Assembly workspace</div>
          <h1>Look inside<br /><span>your .NET assemblies.</span></h1>
          <p className="welcome-intro">A workspace for exploring metadata, inspecting IL,<br className="wide-only" /> and reconstructing readable C#.</p>
          <button className={`drop-zone ${loadingFiles ? 'loading' : ''}`} onClick={() => filePicker.current?.click()} disabled={!ready || loadingFiles}>
            <span className="upload-icon">{loadingFiles ? <LoaderCircle className="spin" size={27} /> : <Upload size={27} />}</span>
            <strong>{loadingFiles ? 'Opening your assembly…' : 'Drop a managed .NET DLL or EXE here'}</strong>
            <span>or <em>browse files</em> to get started</span>
            <div className="drop-footer"><span>.dll</span><span>.exe</span><i />Multiple files supported · up to 64 MB each</div>
          </button>
          <button className="sample-button" onClick={() => void openSample()} disabled={!ready || loadingFiles}><FileCode2 size={15} />Explore a compiled sample<ArrowRight size={14} /></button>
          <div className="feature-row"><div><Braces size={17} /><strong>Understand the structure</strong><span>Types, members, and references</span></div><div><Binary size={17} /><strong>Inspect every instruction</strong><span>Decoded CIL, with resolved tokens</span></div></div>
        </div> : <>
          <div className="document-tab"><FileCode2 size={14} /><span>{selectedLabel}</span><span className="document-language">{active.info.identity.name}</span></div>
          <div className="code-toolbar"><div className="view-tabs" role="tablist" aria-label="Code view">{views.map(v => <button role="tab" aria-selected={view === v} className={view === v ? 'active' : ''} key={v} onClick={() => setView(v)}>{v === 'C#' && <Braces size={13} />}{v === 'IL' && <Binary size={13} />}{v}</button>)}</div><span className="code-tools"><button className={`icon-button ${wrap ? 'enabled' : ''}`} aria-label="Toggle word wrap" aria-pressed={wrap} title="Word wrap" onClick={() => setEditorSettings(previous => ({...previous, wrap: !previous.wrap}))}><WrapText size={15} /></button><button className="icon-button" aria-label="Copy code" title="Copy code" onClick={() => void copy()}>{copied ? <Check size={15} /> : <Copy size={15} />}</button></span></div>
          <div className="breadcrumbs"><Box size={12} /><span>{active.info.identity.name}</span><ChevronRight size={11} /><span>{activeDeclaration?.full_name ?? selectedLabel}</span>{selected && selected > 0 ? <code>{tokenHex(selected)}</code> : null}</div>
          {method?.quality === 'annotated_il' && view === 'C#' && <div className="limitation-banner"><BookOpen size={14} /><span>{method.diagnostics[0]?.detail ?? 'This method is shown as annotated IL.'}</span><button onClick={() => setView('IL')}>View IL</button></div>}
          <div className="viewer-container"><CodeViewer text={code} language={view === 'C#' && method?.quality === 'annotated_il' ? 'IL' : view} settings={editorSettings} theme={theme} onNavigate={select} />{busy && <div className="viewer-loading" role="status"><LoaderCircle size={17} className="spin" />Analyzing method…</div>}</div>
          <div className="code-foot"><span>{view === 'C#' ? method?.quality === 'annotated_il' ? 'Annotated CIL · reconstruction limitation' : 'Reconstructed C# · synthetic local names' : view === 'IL' ? 'Original CIL · Ctrl/⌘-click a token to navigate' : 'Read-only metadata'}</span><span>{code.split('\n').length} lines</span></div>
        </>}
      </main>
      <PaneHandle name="inspector" width={rightWidth} onChange={setRightWidth} reverse />
      <aside className="inspector-pane"><div className="pane-heading"><span>Inspector</span><Braces size={13} /></div>
        {active ? <div className="inspector-content">
          <div className="inspector-title"><span className="type-badge">{activeDeclaration?.kind ?? 'assembly'}</span><h2>{selectedLabel}</h2></div>
          {resolvedDefinition && (resolvedDefinition.assembly !== primary || resolvedDefinition.token !== selected) && <button className="button open-button definition-button" onClick={() => {setPrimary(resolvedDefinition.assembly); setSelected(resolvedDefinition.token); setView('C#')}}><ArrowRight size={13} />Go to definition</button>}
          <dl className="properties"><dt>Assembly</dt><dd>{active.info.identity.name}</dd><dt>Version</dt><dd>{active.info.identity.version}</dd>{selected !== null && selected > 0 && <><dt>Token</dt><dd className="accent-text">{tokenHex(selected)}</dd></>}{metadata?.rva ? <><dt>RVA</dt><dd>{String(metadata.rva)}</dd></> : null}<dt>.NET target</dt><dd data-testid="target-framework" title={active.info.target_framework.moniker ?? 'No target framework attribute was declared.'}>{active.info.target_framework.display_name}</dd><dt>CLR metadata</dt><dd title="Metadata format version; this is not the target .NET version.">{active.info.runtime}</dd><dt>Obfuscation</dt><dd data-testid="obfuscation-status" title="Metadata markers and naming heuristics. No markers found does not rule out obfuscation.">{obfuscationLabels[active.info.obfuscation.status]}</dd><dt>Architecture</dt><dd>{active.info.machine}</dd><dt>File size</dt><dd>{bytes(active.info.size)}</dd></dl>
          {method?.body && <><div className="inspector-section-title">Method analysis</div><dl className="properties"><dt>CIL size</dt><dd>{method.body.code_size} bytes</dd><dt>Instructions</dt><dd>{method.body.instructions.length}</dd><dt>Basic blocks</dt><dd>{method.cfg?.blocks.length ?? '—'}</dd><dt>Maximum stack</dt><dd>{method.stack?.maximum ?? method.body.max_stack}</dd><dt>Exception regions</dt><dd>{method.body.exceptions.length}</dd></dl></>}
          <div className="inspector-section-title">References<span>{active.overview.references.length}</span></div>
          <div className="reference-summary">{active.overview.references.filter(r => r.status === 'resolved').length} resolved · {active.overview.references.filter(r => r.status !== 'resolved').length} unavailable</div>
          {active.overview.references.slice(0, 12).map(r => <button key={r.token} className="reference-row" onClick={() => {select(r.token); setView('Metadata')}} title={`${r.identity.name}, ${r.identity.version}`}><span className={`reference-dot ${r.status}`} /><span>{r.identity.name}</span><span>{r.status === 'resolved' ? 'local' : '—'}</span></button>)}
          <button className="text-button inspector-add" onClick={() => dependencyPicker.current?.click()}><Plus size={13} />Add dependency assemblies</button>
          {outgoing.length > 0 && <><div className="inspector-section-title">Referenced tokens<span>{outgoing.length}</span></div>{outgoing.map(([token, name]) => <button className="token-link" key={token} title={name} onClick={() => {select(token); setView('Metadata')}}><LinkIcon size={12} /><span>{name}</span><code>{tokenHex(token)}</code></button>)}</>}
          {selected !== null && selected > 0 && <><div className="inspector-section-title">Used by<button className="text-button" onClick={() => {if (xrefBusy) {xrefRequest.current?.abort(); setXrefBusy(false)} else void findXrefs()}}>{xrefBusy ? 'Cancel' : 'Find references'}</button></div>{xrefStatus && <p className="xref-status">{xrefStatus}</p>}{xrefs?.map((r, i) => <button className="token-link" key={`${r.token}-${i}`} onClick={() => {select(r.token); setView('IL')}}><Code2 size={12} /><span>{r.name}</span></button>)}{xrefs?.length === 0 && !xrefBusy && <p className="inspector-note">No direct CIL token references found.</p>}</>}
        </div> : <div className="empty-inspector"><div className="inspector-placeholder"><Braces size={26} /></div><h3>A little more context.</h3><p>Select a type or member to see its metadata, signature, and references here.</p><div className="supported-label">Supported inputs</div><span className="support-chip">.NET Framework</span><span className="support-chip">Modern .NET</span><span className="support-chip">IL-only assemblies</span></div>}
      </aside>
    </div>
    <footer className="statusbar"><span className="status-message">{busy || loadingFiles || xrefBusy ? <LoaderCircle size={12} className="spin" /> : <span className="status-dot" />}{status}</span>{(busy || loadingFiles) && <button className="stop-button" onClick={() => reset('Analysis stopped. Workspace memory released.')}>Stop analysis</button>}<span className="status-right">v0.1.0</span></footer>
    {dragging && <div className="drag-overlay"><Upload size={35} /><strong>{active ? 'Drop to add dependency assemblies' : 'Drop your assemblies to begin'}</strong></div>}
    {searchOpen && <div className="modal-backdrop" onClick={() => setSearchOpen(false)}><div className="search-dialog" role="dialog" aria-modal="true" aria-label="Search assembly" onClick={e => e.stopPropagation()}><div className="search-input-row"><Search size={19} /><input ref={searchInput} aria-label="Search members and strings" placeholder={active ? 'Search types, methods, fields, or strings…' : 'Open an assembly to search its metadata'} value={query} onChange={e => setQuery(e.target.value)} maxLength={256} /><button className="icon-button" aria-label="Close search" onClick={() => setSearchOpen(false)}><X size={17} /></button></div><div className="search-scope">{active ? active.info.identity.name : 'No assembly loaded'}<span>{searching ? 'Searching…' : `${results.length}${results.length === 250 ? '+' : ''} results`}</span></div><div className="search-results">{results.map((r, i) => <button key={`${r.token}-${i}`} onClick={() => {select(r.token); setView(r.kind === 'method' || r.kind === 'constructor' ? 'C#' : 'Metadata')}}><span className="search-kind">{r.kind === 'string' ? '“ ”' : <Code2 size={15} />}</span><span><strong>{r.name}</strong><small>{r.context}</small></span><code>{tokenHex(r.token)}</code><ArrowRight size={14} /></button>)}{!results.length && <div className="search-empty">{query.trim() ? searching ? 'Searching local metadata…' : 'No matching members or strings.' : 'Search metadata names, namespaces, user strings, or a token.'}</div>}</div><div className="dialog-footer"><kbd>esc</kbd> to close<span>Local metadata search</span></div></div></div>}
    {settingsOpen && <SettingsDialog settings={editorSettings} onChange={setEditorSettings} appearance={theme} onAppearance={setTheme} onClose={() => setSettingsOpen(false)} />}
    {helpOpen && <div className="modal-backdrop" onClick={() => setHelpOpen(false)}><div className="help-dialog" role="dialog" aria-modal="true" aria-label="About ILens" onClick={e => e.stopPropagation()}><button className="icon-button close-dialog" aria-label="Close about" onClick={() => setHelpOpen(false)}><X size={18} /></button><span className="brand-mark"><Code2 size={25} /></span><h2>Understand the binary.</h2><p>ILens is a static .NET decompiler.</p><p>Open a managed DLL or EXE, choose a method, and compare reconstructed C# with its original CIL. Use <kbd>⌘/Ctrl K</kbd> to search metadata, and <kbd>⌘/Ctrl F</kbd> inside the code viewer.</p><p>This is an initial decompiler release. Simple methods and control-flow patterns reconstruct into C#-like code. Complex async, iterator, closure, and exception patterns may remain annotated IL. Local variable names are synthetic; this is not the original source.</p><p>Missing dependencies do not block analysis. Add them explicitly to resolve assembly identities. Nothing is downloaded automatically, and loaded assemblies are never executed.</p><button className="button open-button" onClick={() => setHelpOpen(false)}>Back to workspace<ArrowRight size={14} /></button></div></div>}
  </div>
}
function PackageSelect({assemblies, primary, onSelect}: {assemblies: WorkspaceAssembly[]; primary: number | null; onSelect: (id: number) => void}) {return <><Layers size={14} /><select aria-label="Primary assembly" value={primary ?? ''} onChange={e => onSelect(Number(e.target.value))}>{assemblies.map(a => <option key={a.id} value={a.id}>{a.filename}</option>)}</select></>}
