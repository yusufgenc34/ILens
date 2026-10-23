'use client'
import {cn, ui} from '../lib/ui'
import { useCallback, useEffect, useMemo, useRef, useState, type ChangeEvent, type DragEvent, type CSSProperties } from 'react'
import { ArrowLeft, ArrowRight, Info, PanelLeft, Binary, BookOpen, Box, Braces, Check, ChevronRight, Code2, Copy, Pencil, FileCode2, FolderOpen, HardDrive, Layers, Link as LinkIcon, LoaderCircle, Plus, Save, Search, Upload, WrapText, X } from 'lucide-react'
import { DecompilerClient } from '../lib/rpc'
import { diagnostic, jsonText, tokenHex, type Declaration, type Diagnostic, type Loaded, type Reference, type MethodAnalysis, type Overview, type DetectionReport, type PeHeaders, type SearchHit, type SourceReference } from '../lib/types'
import {indexDependencyFolder, resolveDependencyFolder, type DependencyFolder} from '../lib/dependency-folder'
import {companionAssembly} from '../lib/companion-assembly'
import {resolveAutomaticDependencies} from '../lib/automatic-dependencies'
import { createWorkspaceTree, isPeToken, peGroups, peTokens, type TreeNode } from '../lib/tree'
import PeView from './PeView'
import ProtectionView, {describeReport, verdictLabels} from './ProtectionView'
import type { MenuAction } from './AppMenuBar'
import Explorer from './Explorer'
import DocumentTabs from './DocumentTabs'
import AnalysisPanel from './AnalysisPanel'
import ResourceViewer from './ResourceViewer'
import {emptyNavigation, visit, travel, toggleBookmark, locationKey, forgetAssembly, type Location} from '../lib/navigation'
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
  const [navigation, setNavigation] = useState(emptyNavigation)
  const [bookmarksOpen, setBookmarksOpen] = useState(false)
  const restoringNavigation = useRef(false)
  const [method, setMethod] = useState<MethodAnalysis | null>(null)
  const [methodAssembly, setMethodAssembly] = useState<number | null>(null)
  const [metadata, setMetadata] = useState<Record<string, unknown> | null>(null)
  const [declaration, setDeclaration] = useState('')
  // Navigation ranges for the whole-type C# document, keyed to its selection.
  const [typeReferences, setTypeReferences] = useState<{token: number; references: SourceReference[]} | null>(null)
  const [status, setStatus] = useState('Ready to open an assembly')
  const [busy, setBusy] = useState(false)
  const [loadingFiles, setLoadingFiles] = useState(false)
  const [errors, setErrors] = useState<Diagnostic[]>([])
  const [theme, setTheme] = useState('dark')
  const [editorSettings, setEditorSettings] = useState(defaultEditorSettings)
  const [settingsOpen, setSettingsOpen] = useState(false)
  const [onlineReferences, setOnlineReferences] = useState(true)
  const onlineReferencesEnabled = useRef(onlineReferences)
  onlineReferencesEnabled.current = onlineReferences
  const [resolvingReference, setResolvingReference] = useState(false)
  const navigationIntent = useRef(0)
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
  const [peHeaders, setPeHeaders] = useState<Record<number, PeHeaders>>({})
  const [protection, setProtection] = useState<Record<number, DetectionReport>>({})
  // dnSpy-style tool windows; the document area always stays visible.
  // Like dnSpy, the analyzer opens on demand (context menu Analyze or View > Analyzer).
  const [panes, setPanes] = useState({explorer: true, analyzer: false, inspector: true})
  const togglePane = (pane: keyof typeof panes) => setPanes(previous => ({...previous, [pane]: !previous[pane]}))
  // Phones show one pane at a time; the explorer and inspector become slide-over drawers.
  const [mobile, setMobile] = useState(false)
  const [drawer, setDrawer] = useState<'explorer' | 'inspector' | null>(null)
  useEffect(() => {
    const query = window.matchMedia('(max-width: 767px)')
    const update = () => {setMobile(query.matches); if (!query.matches) setDrawer(null)}
    update(); query.addEventListener('change', update); return () => query.removeEventListener('change', update)
  }, [])
  const client = useRef<DecompilerClient | null>(null)
  const filePicker = useRef<HTMLInputElement>(null)
  const dependencyPicker = useRef<HTMLInputElement>(null)
  const folderPicker = useRef<HTMLInputElement>(null)
  const dependencyFolder = useRef<DependencyFolder | null>(null)
  const pendingExecutables = useRef<File[]>([])
  const [folderCount, setFolderCount] = useState(0)
  const dependencyRequest = useRef<AbortController | null>(null)
  const searchInput = useRef<HTMLInputElement>(null)
  const selectionRequest = useRef<AbortController | null>(null)
  const selectionVersion = useRef(0)
  const generation = useRef(0)
  const loadLock = useRef(false)
  const active = assemblies.find(a => a.id === primary) ?? null
  const resourceSelected = selected !== null && selected >>> 24 === 40
  const resourceView = resourceSelected && view !== 'Metadata'
  const activeDeclaration = active?.declarations.find(d => d.token === selected)
  const tree = useMemo(() => createWorkspaceTree(assemblies), [assemblies])
  const report = useCallback((error: unknown) => {const d = diagnostic(error); if (d.code !== 'cancelled') setErrors(previous => [...previous.slice(-4), d])}, [])
  const reset = useCallback((message = 'Workspace closed. Assembly memory released.') => {
    navigationIntent.current++; setResolvingReference(false)
    dependencyRequest.current?.abort(); dependencyFolder.current = null; pendingExecutables.current = []; setFolderCount(0)
    generation.current++; selectionVersion.current++; selectionRequest.current?.abort()
    client.current?.terminate(); client.current = null; loadLock.current = false
    setNavigation(emptyNavigation); setBookmarksOpen(false); restoringNavigation.current = false
    setExportOpen(null); setProjectOpen(false); setEditOpen(false); setPeHeaders({}); setProtection({}); setAssemblies([]); setPrimary(null); setSelected(null); setMethod(null); setMetadata(null); setDeclaration(''); setBusy(false); setLoadingFiles(false); setStatus(message)
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
    try {setOnlineReferences(localStorage.getItem('ilens:framework-downloads') !== 'false'); setTheme((localStorage.getItem('ilens:theme') ?? localStorage.getItem('monoasm:theme')) === 'light' ? 'light' : 'dark'); setEditorSettings(parseEditorSettings(localStorage.getItem(editorSettingsKey) ?? localStorage.getItem('monoasm:editor-settings')))} catch { /* Storage is optional. */ }
    return () => {client.current?.terminate()}
  }, [])
  useEffect(() => {if (!ready) return; try {localStorage.setItem('ilens:framework-downloads', String(onlineReferences)); localStorage.setItem('ilens:theme', theme); localStorage.setItem(editorSettingsKey, JSON.stringify(editorSettings))} catch { /* Storage is optional. */ }}, [ready, theme, editorSettings, onlineReferences])
  useEffect(() => {
    const keyboard = (e: KeyboardEvent) => {
      if ((e.target as Element | null)?.closest('dialog[open]')) return
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'o') {e.preventDefault(); filePicker.current?.click()}
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {e.preventDefault(); setSearchOpen(open => !open)}
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key.toLowerCase() === 's') {e.preventDefault(); setExportOpen(open => open ?? 'binary')}
      if (e.key === 'Escape') {setSearchOpen(false); setHelpOpen(false); setBookmarksOpen(false); setDrawer(null)}
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
  // The original buffer was transferred to the worker, so files that fail to load are re-read.
  async function scanFile(file: File) {
    try {const buffer = await file.arrayBuffer(); return `\n${describeReport(await getClient().call('detectFile', {buffer}, {transfer: [buffer]}))}`}
    catch {return ''}
  }
  async function openFiles(files: File[], dependencies = false, resolveOnly = false) {
    if (loadLock.current || (!files.length && !resolveOnly)) return
    loadLock.current = true; setLoadingFiles(true); setErrors([])
    const version = generation.current
    let firstId: number | null = null
    let initialSelectionPublished = false
    const loaded: WorkspaceAssembly[] = []
    let companions = 0, rejectedInputs = 0
    const rejectInput = (error:unknown) => {rejectedInputs++; report(error)}
    const processed = new Set<File>()
    const controller = new AbortController(); dependencyRequest.current = controller
    const core = getClient()
    const retain = async (result: Loaded, filename: string, started = performance.now()) => {
      firstId ??= result.id
      if (assemblies.some(a => a.id === result.id) || loaded.some(a => a.id === result.id)) return
      try {
        const declarations = await core.call('getTree', {assembly:result.id}, {signal:controller.signal})
        const overview = await core.call('getInfo', {assembly:result.id}, {signal:controller.signal})
        loaded.push({...result, filename, declarations, overview, elapsed:performance.now() - started})
        firstId ??= result.id
      } catch (error) {if (!controller.signal.aborted) await core.call('close', {assembly:result.id}); throw error}
    }
    const publish = async () => {
      const next: WorkspaceAssembly[] = []
      for (const assembly of [...assemblies, ...loaded]) {
        if (version !== generation.current) return
        const references = await core.call('getReferences', {assembly:assembly.id}, {signal:controller.signal})
        next.push({...assembly, overview:{...assembly.overview, references}})
      }
      if (version !== generation.current) return
      setAssemblies(next)
      if (!initialSelectionPublished && firstId !== null && (!dependencies || !primary)) {setPrimary(firstId); setSelected(0); setView('C#')}
      initialSelectionPublished = true
      return new Set(next.flatMap(a=>a.overview.references.filter(r=>r.resolved_id===null).map(r=>JSON.stringify(r.identity)))).size
    }
    try {
      if (files.length > 64) throw new Error('Open at most 64 assemblies at a time; use a dependency folder for larger collections.')
      for (const file of files) {
        if (processed.has(file)) continue
        if (version !== generation.current) return
        if (!/\.(dll|exe)$/i.test(file.name)) {rejectInput({code: 'invalid_pe', message: `${file.name} is not a DLL or EXE.`, detail: 'Choose a managed PE assembly.'}); continue}
        if (file.size > 64 * 1024 * 1024) {rejectInput({code: 'size_limit', message: `${file.name} exceeds 64 MiB.`, detail: 'The file was rejected before reading it into memory.'}); continue}
        const started = performance.now()
        setStatus(`Reading ${file.name}…`)
        try {
          const core = getClient()
          const buffer = await file.arrayBuffer()
          if (version !== generation.current) return
          const result = await core.call('load', {buffer}, {signal:controller.signal,transfer: [buffer]})
          processed.add(file)
          await retain(result, file.name, started)
        } catch (error) {
          if (version !== generation.current) return
          const failure = diagnostic(error)
          if (failure.code === 'not_managed' && /\.exe$/i.test(file.name)) {
            try {
              const companion = companionAssembly(file,files,dependencyFolder.current)
              if (!companion) {
                pendingExecutables.current = [...pendingExecutables.current.filter(f=>f.name!==file.name),file].slice(-64)
                const scan = await scanFile(file)
                rejectInput({...failure,message:`${file.name} has no CLR header for .NET decompilation. If it is a .NET launcher, open its managed DLL or select its application folder.`,detail:`${failure.detail} Native/NativeAOT executables cannot be decompiled to CIL; single-file bundles need their original managed DLLs.${scan}`})
                continue
              }
              if(companion.size > 64 * 1024 * 1024) throw {code:'size_limit',message:`${companion.name} exceeds 64 MiB.`,detail:'Companion DLL rejected before reading it.'}
              const buffer = await companion.arrayBuffer()
              controller.signal.throwIfAborted()
              const result = await core.call('load',{buffer},{signal:controller.signal,transfer:[buffer]})
              await retain(result,companion.name,started)
              processed.add(companion); companions++
              pendingExecutables.current = pendingExecutables.current.filter(f=>f.name!==file.name)
            } catch (companionError) {rejectInput(companionError)}
          } else rejectInput({...failure,message:`${file.name}: ${failure.message}`,detail:`${failure.detail}${['not_managed','unsupported_native','invalid_clr','corrupt_metadata','unsupported_metadata'].includes(failure.code) ? await scanFile(file) : ''}`})
        }
      }
      if (version !== generation.current) return
      // Show the input immediately; network resolution must not delay opening it.
      await publish()
      const resolution = await resolveAutomaticDependencies(core, [...assemblies,...loaded], dependencyFolder.current, controller.signal, () => onlineReferencesEnabled.current, async(result,filename)=>{
        await retain(result,filename)
        await publish() // Add roots as they arrive without changing the user's selection.
      },setStatus)
      if (version !== generation.current) return
      const unavailable = await publish()
      if (version !== generation.current) return
      if (resolution.failures.length) report({code:'dependency_download',message:'Some .NET dependencies could not be resolved. Your assembly is still available.',detail:resolution.failures.map(e=>e.message).join('\n')})
      if (resolution.limited) report({code:'size_limit',message:'Automatic dependency resolution reached its limit. Loaded assemblies are still available.',detail:'At most 64 framework resolutions are attempted per load. Add remaining dependencies locally.'})
      if (files.length && firstId === null && rejectedInputs) {setStatus(`No assembly was opened · ${rejectedInputs} rejected ${rejectedInputs === 1 ? 'file' : 'files'}. See diagnostics.`); return}
      setStatus(`${loaded.length} assemblies added${companions ? ` · ${companions} companion DLLs opened` : ''} · ${resolution.downloaded} downloaded · ${unavailable ?? 0} unresolved references${resolution.rejected ? ` · ${resolution.rejected} invalid files skipped` : ''}`)
    } catch (error) {
      if (version === generation.current) {
        report(error)
        try {await publish()} catch (refreshError) {if (version === generation.current) report(refreshError)}
        if (version === generation.current) setStatus(`${loaded.length} assemblies added; dependency resolution stopped. See diagnostics.`)
      }
    }
    finally {if (version === generation.current) {loadLock.current = false; setLoadingFiles(false)}}
  }
  function updateOnlineReferences(enabled:boolean) {
    onlineReferencesEnabled.current=enabled;setOnlineReferences(enabled)
    if(enabled && assemblies.length && !loadLock.current) void openFiles([],true,true)
  }
  function onFiles(event: ChangeEvent<HTMLInputElement>, dependencies: boolean) {const files = Array.from(event.target.files ?? []); event.target.value = ''; void openFiles(files, dependencies)}
  function drop(event: DragEvent) {event.preventDefault(); setDragging(false); void openFiles(Array.from(event.dataTransfer.files), !!active)}
  function select(token: number) {navigationIntent.current++; setSelected(token); setSearchOpen(false); if (token === -1 || token === -2 || token === 0 || token === 1) setView('Metadata'); else if (token >>> 24 === 40) setView('C#')}
  useEffect(() => {
    selectionRequest.current?.abort(); setMethod(null); setMetadata(null); setDeclaration(''); setTypeReferences(null)
    const version = ++selectionVersion.current
    if (!active || selected === null) {setBusy(false); return}
    if (selected <= 0) {setBusy(false); return}
    const controller = new AbortController(); selectionRequest.current = controller
    setBusy(true)
    const core = getClient()
    const run = async () => {
      try {
        const type = await core.call('getType', {assembly: active.id, token: selected}, {signal: controller.signal})
        if (version !== selectionVersion.current) return
        setMetadata(type.metadata); setDeclaration(type.declaration); setTypeReferences(type.references ? {token: selected, references: type.references} : null)
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
  // Every opened assembly is scanned automatically; the result is shown in the inspector.
  useEffect(() => {
    if (!active || protection[active.id]) return
    const controller = new AbortController(), id = active.id
    void getClient().call('detectProtection', {assembly: id}, {signal: controller.signal}).then(report => {
      setProtection(previous => ({...previous, [id]: report}))
      if (report.verdict !== 'no_findings') setStatus(`${active.info.identity.name}: ${describeReport(report).slice(0, 240)}`)
    }).catch(report)
    return () => controller.abort()
  }, [active, protection, report])
  useEffect(() => {
    if (!active || !isPeToken(selected) || selected === peTokens.protection || peHeaders[active.id]) return
    const controller = new AbortController(), id = active.id
    void getClient().call('getPeHeaders', {assembly: id}, {signal: controller.signal}).then(headers => setPeHeaders(previous => ({...previous, [id]: headers}))).catch(report)
    return () => controller.abort()
  }, [active, selected, peHeaders, report])
  const currentLocation: Location | null = primary !== null && selected !== null ? {assembly:primary, token:selected, view} : null
  useEffect(() => {
    if (restoringNavigation.current) {restoringNavigation.current = false; return}
    if (primary !== null && selected !== null) setNavigation(previous => visit(previous, {assembly:primary, token:selected, view}))
  }, [primary, selected, view])
  function navigate(location: Location) {navigationIntent.current++; setPrimary(location.assembly); setSelected(location.token); setView(location.view); setSearchOpen(false)}
  function navigateHistory(direction: -1 | 1) {
    const next = travel(navigation, direction), location = next.history[next.index]
    if (!location || next.index === navigation.index) return
    restoringNavigation.current = true; setNavigation(next); navigate(location)
  }
  function closeTab(location: Location) {
    const tabs = navigation.tabs.filter(t => locationKey(t) !== locationKey(location))
    if (currentLocation && locationKey(currentLocation) === locationKey(location)) {
      const next = tabs.at(-1)
      if (next) navigate(next); else setSelected(null)
    }
    setNavigation({...navigation, tabs})
  }
  function documentLabel(location: Location) {
    const assembly = assemblies.find(a => a.id === location.assembly)
    const name = assembly?.declarations.find(d => d.token === location.token)?.name ?? assembly?.overview.resources.find(r => r.token === location.token)?.name ?? assembly?.overview.references.find(r => r.token === location.token)?.identity.name ?? (location.token === 1 ? assembly?.info.module_name ?? 'Module' : location.token === 0 ? 'Overview' : location.token === peTokens.pe ? 'PE' : isPeToken(location.token) ? peGroups.find(g => peTokens[g.id] === location.token)?.name ?? 'PE' : location.token === -1 ? 'References' : location.token === -2 ? 'Resources' : tokenHex(location.token))
    return `${name} · ${assembly?.info.identity.name ?? 'Assembly'}`
  }
  async function resolveReference(source: number, token: number): Promise<number | null> {
    if (loadLock.current) throw new Error('Wait for the current assembly load to finish.')
    loadLock.current = true; setLoadingFiles(true); setResolvingReference(true)
    const controller = new AbortController(); dependencyRequest.current = controller
    const version = generation.current, core = getClient(), added: WorkspaceAssembly[] = []
    let completion = 'Dependency unavailable. See diagnostics or add a matching DLL.'
    const retain = async (loaded: Loaded, filename: string) => {
      if (assemblies.some(a=>a.id===loaded.id) || added.some(a=>a.id===loaded.id)) return
      try {
        const declarations = await core.call('getTree',{assembly:loaded.id},{signal:controller.signal})
        const overview = await core.call('getInfo',{assembly:loaded.id},{signal:controller.signal})
        added.push({...loaded,filename,declarations,overview,elapsed:0})
      } catch(error) {if (!controller.signal.aborted) await core.call('close',{assembly:loaded.id}); throw error}
    }
    const reference = async () => (await core.call('getReferences',{assembly:source},{signal:controller.signal})).find(r=>r.token===token)
    try {
      let target = await reference()
      if (!target) throw new Error('This metadata token is not an assembly reference.')
      if (target.resolved_id !== null) {completion=`Opened ${target.identity.name}`; return target.resolved_id}
      const owner = assemblies.find(a=>a.id===source)
      if (!owner) return null
      if (dependencyFolder.current) {
        await resolveDependencyFolder(core,[owner],dependencyFolder.current,controller.signal,retain,setStatus)
        target = await reference()
        if (target?.resolved_id !== null && target?.resolved_id !== undefined) {completion=`Resolved ${target.identity.name} from the selected folder`; return target.resolved_id}
      }
      if (!onlineReferences) throw new Error('This reference is unavailable locally. Add its DLL, choose a dependency folder, or enable .NET reference downloads in Settings.')
      const result = await core.call('resolveFrameworkReference',{assembly:source,token},{signal:controller.signal})
      await retain(result,`${result.info.identity.name}.dll`)
      completion=`Resolved ${result.info.identity.name} from ${result.package.id} ${result.package.version}`
      return result.id
    } finally {
      if (version === generation.current) {
        // Even an interrupted transitive search can have loaded useful dependencies.
        try {
          const next: WorkspaceAssembly[] = []
          for (const a of [...assemblies,...added]) {
            const references = await core.call('getReferences',{assembly:a.id},{signal:controller.signal})
            next.push({...a,overview:{...a.overview,references}})
          }
          if (version === generation.current) setAssemblies(next)
        } finally {if (version === generation.current) {loadLock.current=false; setLoadingFiles(false); setResolvingReference(false); setStatus(completion)}}
      }
    }
  }
  async function openReference(source: number, token: number) {
    const intent=navigationIntent.current, version=generation.current
    try {
      const target=await resolveReference(source,token)
      if (target !== null && navigationIntent.current === intent && generation.current === version) navigate({assembly:target,token:0,view:'Metadata'})
    } catch(error) {if (generation.current===version) report(error)}
  }
  async function followReference(token: number, from = primary) {
    if (!from) return
    const source = from, intent=navigationIntent.current, version=generation.current
    try {
      let info = await getClient().call('getMetadata', {assembly:source, token})
      if (navigationIntent.current !== intent || generation.current !== version) return
      const required=info.required_reference as Reference | null
      if (!info.resolved_definition && required) {
        try {await resolveReference(source,required.token)} catch(error) {if (generation.current===version) report(error)}
        if (navigationIntent.current !== intent || generation.current !== version) return
        info=await getClient().call('getMetadata',{assembly:source,token})
      }
      if (navigationIntent.current !== intent || generation.current !== version) return
      const target = info.resolved_definition as {assembly:number; token:number} | null
      navigate(target ? {...target, view:'C#'} : {assembly:source, token, view:'Metadata'})
    } catch (error) {if (generation.current===version) report(error)}
  }
  useEffect(() => {
    const listener = (event: KeyboardEvent) => {
      if ((event.target as Element | null)?.closest('input, textarea, [role="dialog"], dialog')) return
      if (event.altKey && (event.key === 'ArrowLeft' || event.key === 'ArrowRight')) {event.preventDefault(); navigateHistory(event.key === 'ArrowLeft' ? -1 : 1)}
    }
    window.addEventListener('keydown', listener); return () => window.removeEventListener('keydown', listener)
  })
  // Base/derived type links: local definitions open directly, external ones resolve first.
  function linkTo(assembly: number, token: number) {
    if (token >>> 24 === 2) navigate({assembly, token, view: 'C#'})
    else {setPrimary(assembly); void followReference(token, assembly)}
  }
  async function closeAssembly(id: number) {
    if (loadLock.current) {report({code: 'busy', message: 'Wait for the current assembly load to finish.', detail: 'Assemblies cannot be closed while dependencies are being resolved.'}); return}
    const remaining = assemblies.filter(a => a.id !== id)
    if (!remaining.length) {reset(); return}
    const version = generation.current, core = getClient()
    loadLock.current = true
    try {
      await core.call('close', {assembly: id})
      // Other assemblies may have resolved references to the closed one.
      const next: WorkspaceAssembly[] = []
      for (const a of remaining) next.push({...a, overview: {...a.overview, references: await core.call('getReferences', {assembly: a.id})}})
      if (version !== generation.current) return
      setAssemblies(next); setNavigation(previous => forgetAssembly(previous, id)); setPeHeaders(previous => {const copy = {...previous}; delete copy[id]; return copy}); setProtection(previous => {const copy = {...previous}; delete copy[id]; return copy})
      if (primary === id) {setPrimary(next[0].id); setSelected(0); setView('C#')}
      setStatus(`Closed ${assemblies.find(a => a.id === id)?.info.identity.name ?? 'assembly'}. Its memory is reusable by the worker.`)
    } catch (error) {if (version === generation.current) report(error)}
    finally {if (version === generation.current) loadLock.current = false}
  }
  async function copyText(text: string) {try {await navigator.clipboard.writeText(text); setStatus(`Copied ${text.length > 60 ? `${text.slice(0, 57)}…` : text}`)} catch (error) {report(error)}}
  function contextActions(node: TreeNode, tree: {expand: (node: TreeNode) => void; collapse: (node: TreeNode) => void}): (MenuAction | null)[] {
    const assembly = node.assembly!, token = node.token, owner = assemblies.find(a => a.id === assembly)
    const table = token !== undefined && token > 0 ? token >>> 24 : -1
    const open = (view: Location['view']) => {if (token !== undefined) navigate({assembly, token, view})}
    const folder = node.children.length ? [{label: 'Expand All Children', shortcut: '*', run: () => tree.expand(node)}, {label: 'Collapse All Children', run: () => tree.collapse(node)}] : []
    if (node.target !== undefined) return [{label: 'Go to Type', shortcut: 'Enter', run: () => linkTo(assembly, node.target!)}, {label: 'Copy Name', run: () => void copyText(node.name)}, {label: 'Copy Token', run: () => void copyText(tokenHex(node.target!))}]
    if (node.reference) return [{label: 'Open Referenced Assembly', disabled: loadingFiles, run: () => void openReference(assembly, token!)}, {label: 'Copy Identity', run: () => void copyText(`${node.reference!.identity.name}, Version=${node.reference!.identity.version}, Culture=${node.reference!.identity.culture}, PublicKeyToken=${node.reference!.identity.public_key_token}`)}]
    if (node.kind === 'assembly' || node.kind === 'module') return [
      ...folder, null,
      {label: 'Save Module…', disabled: loadingFiles, run: () => {setPrimary(assembly); setExportOpen('binary')}},
      {label: 'Export to Project…', disabled: loadingFiles, run: () => {setPrimary(assembly); setProjectOpen(true)}}, null,
      {label: 'Copy Identity', run: () => void copyText(owner ? `${owner.info.identity.name}, Version=${owner.info.identity.version}, Culture=${owner.info.identity.culture}, PublicKeyToken=${owner.info.identity.public_key_token}` : node.name)},
      {label: 'Close Assembly', shortcut: 'Del', disabled: loadingFiles, run: () => void closeAssembly(assembly)},
    ]
    if (token === undefined || token < 0 || isPeToken(token ?? null)) return folder.length ? folder : [{label: 'Open', disabled: token === undefined, run: () => open('Metadata')}]
    const declaration = [2, 4, 6, 20, 23].includes(table)
    return [
      {label: table === 6 ? 'Decompile' : 'Open', shortcut: 'Enter', run: () => open(table === 40 ? 'C#' : declaration ? 'C#' : 'Metadata')},
      ...(table === 6 ? [{label: 'Show IL', run: () => open('IL')}, {label: 'Edit Method Body (IL)…', disabled: loadingFiles, run: () => {navigate({assembly, token, view: 'IL'}); setEditOpen(true)}}] : []),
      {label: 'Show Metadata', run: () => open('Metadata')}, null,
      ...(declaration ? [{label: 'Analyze', run: () => {navigate({assembly, token, view: 'C#'}); setPanes(previous => ({...previous, analyzer: true}))}}] : []),
      ...([2, 6].includes(table) ? [{label: 'Save Code…', run: () => {setPrimary(assembly); select(token); setExportOpen('source')}}] : []),
      {label: navigation.bookmarks.some(b => b.assembly === assembly && b.token === token) ? 'Remove Bookmark' : 'Add Bookmark', run: () => setNavigation(previous => toggleBookmark(previous, {assembly, token, view: 'C#'}))}, null,
      {label: 'Copy Name', run: () => void copyText(node.declaration?.display_name || node.name)},
      {label: 'Copy Full Name', run: () => void copyText(node.declaration?.full_name ?? node.name)},
      {label: 'Copy Token', run: () => void copyText(tokenHex(token))},
      ...(folder.length ? [null, ...folder] : []),
    ]
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
  const selectedLabel = activeDeclaration?.name ?? (selected === -1 ? 'Assembly references' : selected === -2 ? 'Manifest resources' : selected === 0 ? 'Assembly overview' : selected === peTokens.pe ? 'PE' : isPeToken(selected) ? peGroups.find(g => peTokens[g.id] === selected)?.name ?? 'PE' : metadata?.name ? String(metadata.name) : 'Select a member')
  return <div data-ready={ready} className={cn(`application bg-background text-foreground font-sans text-[13px] font-normal flex flex-col h-dvh min-w-0 md:min-w-165 leading-[1.5] ${theme}`)} onDragOver={e => {e.preventDefault(); if (e.dataTransfer.types.includes('Files')) setDragging(true)}} onDragLeave={e => {if (!e.currentTarget.contains(e.relatedTarget as Node)) setDragging(false)}} onDrop={drop}>
    <input ref={filePicker} type="file" disabled={!ready || loadingFiles} accept=".dll,.exe" multiple hidden aria-label="Open assembly files" onChange={e => onFiles(e, false)} />
    <input ref={dependencyPicker} type="file" disabled={!ready || loadingFiles} accept=".dll,.exe" multiple hidden aria-label="Add dependency files" onChange={e => onFiles(e, true)} />
    <input ref={folderPicker} type="file" {...{webkitdirectory:''}} multiple hidden aria-label="Dependency folder" disabled={!ready || loadingFiles} onChange={event => {
      const files = Array.from(event.target.files ?? []); event.target.value = ''
      if (!files.length) return
      try {const folder = indexDependencyFolder(files); dependencyFolder.current = folder; setFolderCount(folder.files); void openFiles(pendingExecutables.current, true, true)} catch (error) {report(error)}
    }} />
    <AppMenuBar context={active?.filename ?? ''} menus={[
      {label: 'File', actions: [
        {label: 'Open Assembly…', shortcut: 'Ctrl/⌘ O', disabled: !ready || loadingFiles, run: () => filePicker.current?.click()},
        {label: 'Add Dependencies…', disabled: !ready || loadingFiles, run: () => dependencyPicker.current?.click()},
        {label: 'Set Dependency Folder…', disabled: !ready || loadingFiles, run: () => folderPicker.current?.click()},
        {label: 'Clear Dependency Folder', disabled: !folderCount || loadingFiles, run: () => {dependencyFolder.current = null; setFolderCount(0); setStatus('Dependency folder released. Loaded assemblies remain available.')}},
        {label: 'Retry Missing Dependencies', disabled: !active || loadingFiles, run: () => {void openFiles([],true,true)}}, null,
        {label: 'Save Module…', shortcut: 'Ctrl/⌘ Shift S', disabled: !active || loadingFiles, run: () => setExportOpen('binary')},
        {label: 'Save Code…', disabled: !active || !activeDeclaration || busy, run: () => setExportOpen('source')},
        {label: 'Export to Project…', disabled: !active || loadingFiles, run: () => setProjectOpen(true)}, null,
        {label: 'Close Assembly', disabled: !active || loadingFiles, run: () => {if (active) void closeAssembly(active.id)}},
        {label: 'Close Workspace', disabled: !active, run: () => reset()},
      ]},
      {label: 'Edit', actions: [
        {label: 'Copy Code', disabled: !active || busy, run: () => {void copy()}}, null,
        {label: 'Search Assembly…', shortcut: 'Ctrl/⌘ K', disabled: !active, run: () => setSearchOpen(true)},
        {label: 'Go to Token…', disabled: !active, run: () => {setQuery('0x'); setSearchOpen(true)}},
        {label: 'Toggle Bookmark', disabled: !currentLocation, run: () => {if (currentLocation) setNavigation(previous => toggleBookmark(previous, currentLocation))}}, null,
        {label: 'Settings…', disabled: !ready, run: () => setSettingsOpen(true)},
      ]},
      {label: 'View', actions: [
        {label: 'Back', shortcut: 'Alt+Left', disabled: navigation.index <= 0, run: () => navigateHistory(-1)},
        {label: 'Forward', shortcut: 'Alt+Right', disabled: navigation.index >= navigation.history.length - 1, run: () => navigateHistory(1)},
        {label: 'Bookmarks…', disabled: !navigation.bookmarks.length, run: () => setBookmarksOpen(true)}, null,
        {label: 'Assembly Explorer', checked: panes.explorer, run: () => togglePane('explorer')},
        {label: 'Analyzer', checked: panes.analyzer, run: () => togglePane('analyzer')},
        {label: 'Inspector', checked: panes.inspector, run: () => togglePane('inspector')}, null,
        ...views.map(item => ({label: item === 'Hex' ? 'Hex / Raw Metadata' : item, checked: view === item, disabled: !active, run: () => setView(item)})), null,
        {label: 'Word Wrap', checked: wrap, run: () => setEditorSettings(previous => ({...previous, wrap: !previous.wrap}))},
        {label: 'Light Appearance', checked: theme === 'light', run: () => setTheme(theme === 'dark' ? 'light' : 'dark')},
      ]},
      {label: 'Window', actions: navigation.tabs.length ? [
        ...navigation.tabs.slice(-12).map(tab => ({label: documentLabel(tab), checked: currentLocation !== null && locationKey(tab) === locationKey(currentLocation), run: () => navigate(tab)})), null,
        {label: 'Close Document', disabled: !currentLocation, run: () => {if (currentLocation) closeTab(currentLocation)}},
        {label: 'Close All Documents', run: () => {setNavigation(previous => ({...previous, tabs: []})); setSelected(null)}},
      ] : [{label: 'No open documents', disabled: true, run: () => {}}]},
      {label: 'Help', actions: [{label: 'About ILens', run: () => setHelpOpen(true)}]},
    ]} />
    {active && <div className="app-toolbar flex h-[33px] shrink-0 items-center gap-1 border-b border-solid border-b-border bg-surface px-2 [&_button]:h-[25px] [&_button]:w-[27px] [&_button]:justify-center" role="toolbar" aria-label="Main toolbar">
      <button className={cn(ui.iconButton, 'md:!hidden')} aria-label="Show assembly explorer" aria-expanded={drawer === 'explorer'} onClick={() => setDrawer(drawer === 'explorer' ? null : 'explorer')}><PanelLeft size={15} /></button>
      <button className={ui.iconButton} aria-label="Back" title="Back (Alt+Left)" disabled={navigation.index <= 0} onClick={() => navigateHistory(-1)}><ArrowLeft size={15} /></button>
      <button className={ui.iconButton} aria-label="Forward" title="Forward (Alt+Right)" disabled={navigation.index >= navigation.history.length - 1} onClick={() => navigateHistory(1)}><ArrowRight size={15} /></button>
      <span className="mx-1 h-4 w-px bg-border" aria-hidden="true" />
      <button className={ui.iconButton} aria-label="Open assembly" title="Open (Ctrl/⌘ O)" disabled={!ready || loadingFiles} onClick={() => filePicker.current?.click()}><FolderOpen size={15} /></button>
      <button className={ui.iconButton} aria-label="Save module" title="Save Module…" disabled={loadingFiles} onClick={() => setExportOpen('binary')}><Save size={15} /></button>
      <span className="mx-1 h-4 w-px bg-border" aria-hidden="true" />
      <button className={ui.iconButton} aria-label="Search assemblies" title="Search (Ctrl/⌘ K)" onClick={() => setSearchOpen(true)}><Search size={15} /></button>
      <label className="ml-1 flex items-center gap-1.5 text-[11px] text-muted-foreground"><span className="max-md:sr-only">Language</span><select aria-label="Decompiler language" className="h-[23px] rounded-[4px] border border-solid border-input bg-background px-1.5 text-[11px] text-foreground" value={view === 'IL' ? 'IL' : 'C#'} onChange={event => setView(event.target.value as View)}><option value="C#">C#</option><option value="IL">IL</option></select></label>
      <button className={cn(ui.iconButton, '!w-auto px-2 gap-1 text-[11px]')} aria-label="Edit method body" title="Edit Method Body (IL)…" disabled={busy || loadingFiles || selected === null || selected >>> 24 !== 6 || !method?.body || methodAssembly !== active.id || method.token !== selected} onClick={() => {setView('IL'); setEditOpen(true)}}><Pencil size={13} /><span className="max-md:sr-only">Edit IL</span></button>
      <button className={cn(ui.iconButton, 'ml-auto md:!hidden')} aria-label="Show inspector" aria-expanded={drawer === 'inspector'} onClick={() => setDrawer(drawer === 'inspector' ? null : 'inspector')}><Info size={15} /></button>
    </div>}
    {errors.length > 0 && <div className="errors py-0 px-4 max-h-45 overflow-auto bg-destructive-bg text-destructive border-b border-solid border-b-border" role="alert">{errors.map((error, i) => <div className="error-item flex justify-between gap-[15px] py-2.5 px-0 text-[12px] [&_strong]:font-[550] [&_details]:text-[10px] [&_details]:mt-[3px] [&_code]:[overflow-wrap:anywhere]" key={`${error.code}-${i}`}><div><strong>{error.message}</strong><details><summary>Technical details</summary><code>{error.code}: {error.detail}</code></details></div><button className={cn(ui.iconButton)} aria-label="Dismiss error" onClick={() => setErrors(previous => previous.filter((_, index) => index !== i))}><X size={15} /></button></div>)}</div>}
    {active && <AssemblyWarnings info={active.info} onNavigate={token => {select(token); setView('Metadata')}} />}
    <div className="workspace flex-1 grid grid-cols-[var(--left-width)_1px_minmax(200px,1fr)_1px_var(--right-width)] min-h-0 max-[1000px]:grid-cols-[240px_1px_minmax(200px,1fr)] max-[1000px]:[&>.pane-handle:nth-of-type(2)]:hidden max-md:grid-cols-[minmax(0,1fr)] max-md:[&>.pane-handle]:hidden max-md:[&>span]:hidden" style={{'--left-width': panes.explorer ? `${leftWidth}px` : '0px', '--right-width': panes.inspector ? `${rightWidth}px` : '0px'} as CSSProperties}>
      {mobile && drawer && <div className="drawer-backdrop fixed inset-0 z-60 bg-black/50" aria-hidden="true" onClick={() => setDrawer(null)} />}
      <aside className={cn("explorer-pane flex flex-col min-w-0 min-h-0 bg-surface", mobile ? (drawer === 'explorer' ? 'fixed inset-y-0 left-0 z-70 w-[min(88vw,360px)] border-r border-solid border-r-border shadow-[var(--shadow)]' : 'hidden') : !panes.explorer && 'hidden')} aria-label="Assembly Explorer">
        <div className="pane-heading h-10.5 min-h-10.5 flex items-center justify-between py-0 px-[17px] border-b border-solid border-b-border font-sans text-[12px] font-[500] leading-normal tracking-[0px] text-foreground"><span>Assembly Explorer</span><span aria-label="Loaded assemblies" className="count-pill tracking-[0px] bg-hover min-w-[19px] text-center text-muted-foreground text-[10px] rounded-[4px] py-px px-[5px]">{assemblies.length}</span>{active && <button className={cn(ui.iconButton)} aria-label="Close workspace" title="Close workspace and release memory" onClick={() => reset()}><X size={14} /></button>}</div>
        {active ? <>

          <Explorer tree={tree} selectedAssembly={primary} selected={selected} onSelect={(assembly,token) => {setPrimary(assembly); select(token); if (mobile && ([4, 6, 20, 23, 35, 40].includes(token >>> 24) || token <= peTokens.dos)) setDrawer(null)}} onReference={(assembly,token)=>void openReference(assembly,token)} onLink={linkTo} contextActions={contextActions} />
          <div className="explorer-foot flex gap-2 items-center py-[11px] px-3.5 border-t border-solid border-t-border text-[9px] text-faint"><HardDrive size={13} /><span>{bytes(assemblies.reduce((sum, a) => sum + a.info.size, 0))} in workspace</span></div>
        </> : <div className="empty-explorer flex items-center flex-col text-faint py-10.5 px-7 text-center [&_p]:mt-4 [&_p]:mr-0 [&_p]:mb-1.5 [&_p]:ml-0 [&_p]:text-[13px] [&_p]:font-medium [&_p]:text-foreground [&>span]:max-w-47.5 [&>span]:text-[12px] [&>span]:leading-[1.7] [&_.text-button]:mt-[21px]"><Layers size={25} /><p>No assemblies loaded</p><span>Open a DLL or EXE to explore its structure.</span><button className="inline-flex gap-1.5 items-center border-0 bg-transparent p-0 text-[12px] text-foreground hover:underline hover:underline-offset-1" onClick={() => filePicker.current?.click()}>Open an assembly <ArrowRight size={13} /></button></div>}
      </aside>
      {panes.explorer ? <PaneHandle name="assembly explorer" width={leftWidth} onChange={setLeftWidth} /> : <span />}
      <main data-assembly-id={primary ?? undefined} className="center-pane flex flex-col min-w-0 min-h-0 bg-editor relative">
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
          <DocumentTabs state={navigation} current={currentLocation} label={documentLabel} onSelect={navigate} onClose={closeTab} onTravel={navigateHistory} onBookmark={() => {if (currentLocation) setNavigation(previous => toggleBookmark(previous, currentLocation))}} />
          <div className="code-toolbar flex justify-between border-b border-solid border-b-border py-0 px-3 h-11 min-h-11 items-center"><div className="view-tabs flex items-stretch bg-muted rounded-md h-8 p-[3px] gap-0.5 [&_button]:flex [&_button]:items-center [&_button]:gap-[5px] [&_button]:bg-transparent [&_button]:text-muted-foreground [&_button]:border [&_button]:border-solid [&_button]:border-transparent [&_button]:rounded-[4px] [&_button]:text-[12px] [&_button]:py-0 [&_button]:px-2.5 [&_button.active]:text-foreground [&_button.active]:bg-background [&_button.active]:border-b-transparent [&_button.active]:shadow-[0_1px_2px_#00000014]" role="tablist" aria-label="Code view">{views.map(v => <button role="tab" aria-selected={view === v} disabled={resourceSelected && v === 'IL'} className={cn(view === v ? "active" : "")} key={v} onClick={() => setView(v)}>{v === 'C#' && <Braces size={13} />}{v === 'IL' && <Binary size={13} />}{resourceSelected && v === 'C#' ? 'Resource' : v}</button>)}</div><span className="code-tools ml-auto flex items-center gap-1">
            {view === 'IL' && selected !== null && selected >>> 24 === 6 && <button className={cn(ui.button, ui.subtle, "h-7 gap-1.5 px-2 text-[12px]")} disabled={busy || loadingFiles || !method?.body || methodAssembly !== active.id || method.token !== selected} onClick={() => setEditOpen(true)}><Pencil size={13} />Edit IL</button>}
            {!resourceView && <><button className={cn(`${ui.iconButton} ${wrap ? 'enabled' : ''}`)} aria-label="Toggle word wrap" aria-pressed={wrap} title="Word wrap" onClick={() => setEditorSettings(previous => ({...previous, wrap: !previous.wrap}))}><WrapText size={15} /></button><button className={cn(ui.iconButton)} aria-label="Copy code" title="Copy code" onClick={() => void copy()}>{copied ? <Check size={15} /> : <Copy size={15} />}</button></>}</span></div>
          <div className="breadcrumbs flex gap-[7px] items-center h-[33px] min-h-[33px] py-0 px-4 border-b border-solid border-b-border text-faint text-[11px] [&>span]:whitespace-nowrap [&>span]:overflow-hidden [&>span]:text-ellipsis [&_code]:ml-auto [&_code]:whitespace-nowrap [&_code]:text-muted-foreground [&_code]:font-mono [&_code]:text-[9px] [&_code]:font-normal [&_code]:leading-normal"><Box size={12} /><span>{active.info.identity.name}</span><ChevronRight size={11} /><span>{activeDeclaration?.full_name ?? selectedLabel}</span>{selected && selected > 0 ? <code>{tokenHex(selected)}</code> : null}</div>
          {active.info.reference_assembly && <div role="note" className="border-b border-border bg-muted px-4 py-2 text-xs text-muted-foreground">Reference assembly · API metadata only. Method implementations are not included.</div>}
          {method?.quality === 'annotated_il' && view === 'C#' && <div className="limitation-banner flex items-start gap-2 py-[9px] px-3.5 bg-muted border-b border-solid border-b-border text-foreground text-[12px] [&>svg]:shrink-0 [&>svg]:mt-0.5 [&_button]:shrink-0 [&_button]:text-foreground [&_button]:bg-transparent [&_button]:border-0 [&_button]:ml-auto [&_button]:text-[11px]"><BookOpen size={14} /><span>{method.diagnostics[0]?.detail ?? 'This method is shown as annotated IL.'}</span><button onClick={() => setView('IL')}>View IL</button></div>}
          <div className="viewer-container relative flex-1 min-h-0">{selected === peTokens.protection ? <ProtectionView report={protection[active.id] ?? null} /> : isPeToken(selected) && <PeView headers={peHeaders[active.id] ?? null} token={selected!} />}{resourceView && <ResourceViewer client={getClient()} assembly={active.id} token={selected!} name={active.overview.resources.find(r => r.token === selected)?.name ?? selectedLabel} hex={view === 'Hex'} />}<div className={resourceView || isPeToken(selected) ? 'hidden' : 'h-full'}><CodeViewer documentKey={!resourceView && !busy && (selected === null || selected >>> 24 !== 6 || (methodAssembly === active.id && method?.token === selected)) ? `${active.id}:${selected}:${view}` : undefined} references={resourceView || busy || view !== 'C#' ? undefined : methodAssembly === active.id && method?.token === selected ? method.references : typeReferences?.token === selected ? typeReferences.references : undefined} onReferenceNavigate={token => void followReference(token)} text={code} language={view === 'C#' && method?.quality === 'annotated_il' ? 'IL' : view} settings={editorSettings} theme={theme} onNavigate={select} /></div>{busy && <div className="viewer-loading absolute top-3 right-[17px] flex gap-[9px] items-center py-[9px] px-3 bg-surface-raised border border-solid border-border rounded-[6px] text-foreground shadow-[var(--shadow)] text-[11px]" role="status"><LoaderCircle size={17} className="spin animate-spin motion-reduce:animate-none" />Analyzing method…</div>}</div>
          {panes.analyzer && selected !== null && selected > 0 && [1,2,4,6,10,20,23,27,43].includes(selected >>> 24) && <div className="analyzer-pane relative max-h-[38%] max-md:max-h-[24%] min-h-0 overflow-auto border-t border-solid border-t-border bg-surface px-4 pb-3 [&>section]:mt-0 [&>section]:border-t-0 [&>section]:pt-3"><button className={cn(ui.iconButton, 'absolute right-2 top-1.5 z-1')} aria-label="Close analyzer" onClick={() => togglePane('analyzer')}><X size={14} /></button><AnalysisPanel getClient={getClient} assemblies={assemblies} assembly={active.id} token={selected} onNavigate={navigate} /></div>}
          <div className="code-foot h-7 min-h-7 flex justify-between items-center py-0 px-[15px] border-t border-solid border-t-border text-faint gap-2.5 font-mono text-[10px] font-normal leading-normal [&>span:first-child]:overflow-hidden [&>span:first-child]:whitespace-nowrap [&>span:first-child]:text-ellipsis max-[1000px]:text-[8px]"><span>{isPeToken(selected) ? 'PE/CLI header · read-only file view' : resourceView ? 'Embedded resource · read-only' : view === 'C#' ? method?.quality === 'annotated_il' ? 'Annotated CIL · reconstruction limitation' : 'Reconstructed C# · synthetic local names' : view === 'IL' ? 'Original CIL · Ctrl/⌘-click a token to navigate' : 'Read-only metadata'}</span>{!resourceView && !isPeToken(selected) && <span>{code.split('\n').length} lines</span>}</div>
        </>}
      </main>
      {panes.inspector ? <PaneHandle name="inspector" width={rightWidth} onChange={setRightWidth} reverse /> : <span className="pane-handle" />}
      <aside className={cn("inspector-pane flex flex-col min-w-0 min-h-0 bg-surface", mobile ? (drawer === 'inspector' ? 'fixed inset-y-0 right-0 z-70 w-[min(88vw,340px)] border-l border-solid border-l-border shadow-[var(--shadow)]' : 'hidden') : cn('max-[1000px]:hidden', !panes.inspector && 'hidden'))}><div className="pane-heading h-10.5 min-h-10.5 flex items-center justify-between py-0 px-[17px] border-b border-solid border-b-border font-sans text-[12px] font-[500] leading-normal tracking-[0px] text-foreground"><span>Inspector</span><Braces size={13} /></div>
        {active ? <div className="inspector-content overflow-auto [scrollbar-width:thin] [scrollbar-color:var(--border)_transparent] py-5 px-4">
          <div className="inspector-title [&_h2]:font-mono [&_h2]:font-medium [&_h2]:break-words [&_h2]:mt-2.5 [&_h2]:mr-0 [&_h2]:mb-[17px] [&_h2]:ml-0 [&_h2]:text-[13px]"><span className="type-badge font-sans text-[11px] font-normal leading-normal text-secondary-foreground bg-secondary rounded-[4px] py-0.5 px-1.5">{activeDeclaration?.kind ?? (resourceSelected ? 'resource' : isPeToken(selected) ? 'pe header' : 'assembly')}</span><h2>{selectedLabel}</h2></div>
          {resolvedDefinition && (resolvedDefinition.assembly !== primary || resolvedDefinition.token !== selected) && <button className={cn(ui.button, ui.primary, "definition-button mt-0 mr-0 mb-4.5 ml-0 text-[12px]")} onClick={() => navigate({...resolvedDefinition,view:'C#'})}><ArrowRight size={13} />Go to definition</button>}
          {selected !== null && selected >>> 24 === 35 && <button className={cn(ui.button,'mb-4')} disabled={loadingFiles} onClick={()=>void openReference(active.id,selected)}>Open referenced assembly</button>}
          {!resolvedDefinition && selected !== null && selected >>> 24 !== 35 && metadata?.required_reference ? <button className={cn(ui.button,'mb-4')} disabled={loadingFiles} onClick={()=>void followReference(selected)}>Resolve dependency and follow</button> : null}
          {resolvingReference && <p role="status" className="mb-3 flex items-center gap-2 text-xs text-muted-foreground"><LoaderCircle size={14} className="animate-spin"/>Resolving dependency…</p>}
          <dl className="properties grid grid-cols-[85px_minmax(0,1fr)] gap-y-2 gap-x-[5px] mt-0 mr-0 mb-5.5 ml-0 text-[12px] [&_dt]:text-faint [&_dd]:m-0 [&_dd]:break-words [&_dd]:font-mono [&_dd]:text-[11px] [&_dd]:font-normal [&_dd]:leading-[1.65] [&_dd]:text-foreground [&_dd.accent-text]:text-foreground"><dt>Assembly</dt><dd>{active.info.identity.name}</dd><dt>Version</dt><dd>{active.info.identity.version}</dd>{selected !== null && selected > 0 && <><dt>Token</dt><dd className="accent-text">{tokenHex(selected)}</dd></>}{metadata?.rva ? <><dt>RVA</dt><dd>{String(metadata.rva)}</dd></> : null}<dt>.NET target</dt><dd data-testid="target-framework" title={active.info.target_framework.moniker ?? 'No target framework attribute was declared.'}>{active.info.target_framework.display_name}</dd><dt>CLR metadata</dt><dd title="Metadata format version; this is not the target .NET version.">{active.info.runtime}</dd><dt>Obfuscation</dt><dd data-testid="obfuscation-status" title="Metadata markers and naming heuristics. No markers found does not rule out obfuscation.">{obfuscationLabels[active.info.obfuscation.status]}</dd><dt>Protection</dt><dd data-testid="protection-status">{protection[active.id] ? <button className="border-0 bg-transparent p-0 text-left font-mono text-[11px] text-foreground underline-offset-2 hover:underline" title={describeReport(protection[active.id])} onClick={() => navigate({assembly: active.id, token: peTokens.protection, view: 'Metadata'})}>{verdictLabels[protection[active.id].verdict]}{protection[active.id].detections.length ? ` · ${protection[active.id].detections.map(d => d.name).join(', ')}` : ''}</button> : 'Scanning…'}</dd><dt>Architecture</dt><dd>{active.info.machine}</dd><dt>File size</dt><dd>{bytes(active.info.size)}</dd></dl>
          {method?.body && <><div className="inspector-section-title border-t border-solid border-t-border pt-4 mt-4.5 mb-3.5 flex items-center justify-between font-sans text-[12px] font-[500] leading-normal tracking-[0px] text-foreground [&_.text-button]:tracking-[0px] [&_.text-button]:font-sans [&_.text-button]:text-[11px] [&_.text-button]:font-normal [&_.text-button]:leading-normal">Method analysis</div><dl className="properties grid grid-cols-[85px_minmax(0,1fr)] gap-y-2 gap-x-[5px] mt-0 mr-0 mb-5.5 ml-0 text-[12px] [&_dt]:text-faint [&_dd]:m-0 [&_dd]:break-words [&_dd]:font-mono [&_dd]:text-[11px] [&_dd]:font-normal [&_dd]:leading-[1.65] [&_dd]:text-foreground [&_dd.accent-text]:text-foreground"><dt>CIL size</dt><dd>{method.body.code_size} bytes</dd><dt>Instructions</dt><dd>{method.body.instructions.length}</dd><dt>Basic blocks</dt><dd>{method.cfg?.blocks.length ?? '—'}</dd><dt>Maximum stack</dt><dd>{method.stack?.maximum ?? method.body.max_stack}</dd><dt>Exception regions</dt><dd>{method.body.exceptions.length}</dd></dl></>}
          <div className="inspector-section-title border-t border-solid border-t-border pt-4 mt-4.5 mb-3.5 flex items-center justify-between font-sans text-[12px] font-[500] leading-normal tracking-[0px] text-foreground [&_.text-button]:tracking-[0px] [&_.text-button]:font-sans [&_.text-button]:text-[11px] [&_.text-button]:font-normal [&_.text-button]:leading-normal">References<span>{active.overview.references.length}</span></div>
          <div className="reference-summary text-faint mt-0 mr-0 mb-[9px] ml-0 text-[11px]">{active.overview.references.filter(r => r.status === 'resolved').length} resolved · {active.overview.references.filter(r => r.status !== 'resolved').length} unavailable</div>
          {active.overview.references.slice(0, 12).map(r => <button key={r.token} className="reference-row flex gap-2 items-center border-0 bg-transparent text-muted-foreground w-full py-[5px] px-0 text-left font-mono text-[11px] font-normal leading-normal [&>span:nth-child(2)]:whitespace-nowrap [&>span:nth-child(2)]:overflow-hidden [&>span:nth-child(2)]:text-ellipsis [&>span:nth-child(2)]:flex-1 [&>span:last-child]:text-faint [&>span:last-child]:text-[10px] hover:text-foreground" onClick={() => {select(r.token); setView('Metadata')}} title={`${r.identity.name}, ${r.identity.version}`}><span className={cn(`reference-dot w-[5px] h-[5px] border border-solid border-faint rounded-full shrink-0 [&.resolved]:bg-foreground [&.resolved]:border-foreground ${r.status}`)} /><span>{r.identity.name}</span><span>{r.status === 'resolved' ? 'loaded' : '—'}</span></button>)}
          <button className="inspector-add inline-flex gap-1.5 items-center border-0 bg-transparent p-0 text-[12px] text-foreground mt-3 hover:underline hover:underline-offset-1" onClick={() => dependencyPicker.current?.click()}><Plus size={13} />Add dependency assemblies</button>
          {outgoing.length > 0 && <><div className="inspector-section-title border-t border-solid border-t-border pt-4 mt-4.5 mb-3.5 flex items-center justify-between font-sans text-[12px] font-[500] leading-normal tracking-[0px] text-foreground [&_.text-button]:tracking-[0px] [&_.text-button]:font-sans [&_.text-button]:text-[11px] [&_.text-button]:font-normal [&_.text-button]:leading-normal">Referenced tokens<span>{outgoing.length}</span></div>{outgoing.map(([token, name]) => <button className="token-link hover:text-foreground bg-transparent border-0 flex items-center gap-[7px] text-left text-muted-foreground w-full py-1.5 px-0 text-[11px] [&>span]:flex-1 [&>span]:overflow-hidden [&>span]:text-ellipsis [&>span]:whitespace-nowrap [&>code]:text-faint [&>code]:font-mono [&>code]:text-[9px] [&>code]:font-normal [&>code]:leading-normal [&>svg]:shrink-0" key={token} title={name} onClick={() => {select(token); setView('Metadata')}}><LinkIcon size={12} /><span>{name}</span><code>{tokenHex(token)}</code></button>)}</>}

        </div> : <div className="empty-inspector py-9.5 px-5.5 [&_h3]:mt-5 [&_h3]:mr-0 [&_h3]:mb-[7px] [&_h3]:ml-0 [&_h3]:text-[14px] [&_h3]:font-medium [&_h3]:text-foreground [&>p]:text-faint [&>p]:max-w-55 [&>p]:text-[12px] [&>p]:leading-[1.7] max-[1200px]:py-[25px] max-[1200px]:px-4"><div className="inspector-placeholder inline-flex text-faint bg-surface-raised border border-solid border-border rounded-[8px] p-3"><Braces size={26} /></div><h3>A little more context.</h3><p>Select a type or member to see its metadata, signature, and references here.</p><div className="supported-label mt-7.5 mr-0 mb-[13px] ml-0 text-faint font-sans text-[12px] font-[500] leading-normal tracking-[0px]">Supported inputs</div><span className="support-chip inline-block text-muted-foreground border border-solid border-border py-[3px] px-[7px] mt-0 mr-1 mb-1.5 ml-0 text-[11px] rounded-md bg-muted">.NET Framework</span><span className="support-chip inline-block text-muted-foreground border border-solid border-border py-[3px] px-[7px] mt-0 mr-1 mb-1.5 ml-0 text-[11px] rounded-md bg-muted">Modern .NET</span><span className="support-chip inline-block text-muted-foreground border border-solid border-border py-[3px] px-[7px] mt-0 mr-1 mb-1.5 ml-0 text-[11px] rounded-md bg-muted">IL-only assemblies</span></div>}
      </aside>
    </div>
    <footer className="statusbar h-7 min-h-7 flex items-center gap-2 justify-between border-t border-solid border-t-border bg-surface py-0 px-3.5 text-muted-foreground text-[11px]"><span className="status-message flex items-center gap-2 whitespace-nowrap overflow-hidden text-ellipsis">{busy || loadingFiles ? <LoaderCircle size={12} className="spin animate-spin motion-reduce:animate-none" /> : <span className="status-dot inline-block w-[5px] h-[5px] rounded-full shrink-0 bg-muted-foreground shadow-none" />}{status}</span>{(busy || loadingFiles) && <button className="stop-button bg-transparent border-0 whitespace-nowrap text-foreground text-[11px]" onClick={() => reset('Analysis stopped. Workspace memory released.')}>Stop analysis</button>}<span className="status-right flex items-center gap-[7px] whitespace-nowrap text-faint text-[10px]">{folderCount > 0 && <span className="max-md:hidden" title="Only this selected folder is searched for dependencies">{folderCount} dependency candidates · </span>}v0.1.0</span></footer>
    {dragging && <div className="drag-overlay fixed [inset:8px] z-100 rounded-[10px] flex flex-col gap-5 items-center justify-center pointer-events-none bg-background border-2 border-dashed border-ring text-foreground [&_strong]:text-[22px] [&_span]:text-[12px] [&_span]:text-muted-foreground"><Upload size={35} /><strong>{active ? 'Drop to add dependency assemblies' : 'Drop your assemblies to begin'}</strong></div>}
    {searchOpen && <div className="modal-backdrop fixed [inset:0] z-80 bg-[#0008] flex justify-center items-start pt-[min(15vh,130px)] [backdrop-filter:none]" onClick={() => setSearchOpen(false)}><div className="search-dialog w-[min(680px,90vw)] border border-solid border-border shadow-[var(--shadow)] overflow-hidden bg-popover rounded-[8px]" role="dialog" aria-modal="true" aria-label="Search assembly" onClick={e => e.stopPropagation()}><div className="search-input-row flex gap-[13px] items-center p-4 text-muted-foreground border-b border-solid border-b-border [&_input]:flex-1 [&_input]:min-w-0 [&_input]:border-0 [&_input]:bg-transparent [&_input]:text-text [&_input]:text-[14px] [&_input]:[outline:none]"><Search size={19} /><input ref={searchInput} aria-label="Search members and strings" placeholder={active ? 'Search types, methods, fields, or strings…' : 'Open an assembly to search its metadata'} value={query} onChange={e => setQuery(e.target.value)} maxLength={256} /><button className={cn(ui.iconButton)} aria-label="Close search" onClick={() => setSearchOpen(false)}><X size={17} /></button></div><div className="search-scope py-[9px] px-[17px] text-faint flex justify-between bg-surface text-[12px]">{active ? active.info.identity.name : 'No assembly loaded'}<span>{searching ? 'Searching…' : `${results.length}${results.length === 250 ? '+' : ''} results`}</span></div><div className="search-results max-h-102.5 overflow-auto [&>button]:flex [&>button]:items-center [&>button]:gap-3 [&>button]:text-left [&>button]:bg-transparent [&>button]:border-0 [&>button]:border-b [&>button]:border-solid [&>button]:border-b-border [&>button]:w-full [&>button]:py-[13px] [&>button]:px-[17px] [&>button:hover]:bg-hover [&>button>span:nth-child(2)]:min-w-0 [&>button>span:nth-child(2)]:flex-1 [&_strong]:block [&_strong]:font-medium [&_strong]:overflow-hidden [&_strong]:text-ellipsis [&_strong]:whitespace-nowrap [&_strong]:text-[13px] [&_small]:block [&_small]:text-faint [&_small]:whitespace-nowrap [&_small]:text-ellipsis [&_small]:overflow-hidden [&_small]:mt-0.5 [&_small]:text-[11px] [&_code]:font-mono [&_code]:text-[9px] [&_code]:font-normal [&_code]:leading-normal [&_code]:text-faint [&_svg]:text-muted-foreground">{results.map((r, i) => <button key={`${r.token}-${i}`} onClick={() => {select(r.token); setView(r.kind === 'method' || r.kind === 'constructor' || r.kind === 'resource' ? 'C#' : 'Metadata')}}><span className="search-kind w-7 min-w-7 h-7 flex items-center justify-center rounded-[5px] text-foreground bg-accent-dim">{r.kind === 'string' ? '“ ”' : <Code2 size={15} />}</span><span><strong>{r.name}</strong><small>{r.context}</small></span><code>{tokenHex(r.token)}</code><ArrowRight size={14} /></button>)}{!results.length && <div className="search-empty text-center text-faint py-[55px] px-5 text-[13px]">{query.trim() ? searching ? 'Searching local metadata…' : 'No matching members or strings.' : 'Search metadata names, namespaces, user strings, or a token.'}</div>}</div><div className="dialog-footer flex items-center gap-[5px] py-[9px] px-4 border-t border-solid border-t-border text-faint text-[11px] [&>span]:ml-auto"><kbd>esc</kbd> to close<span>Local metadata search</span></div></div></div>}
    {projectOpen && primary && <ProjectExportDialog client={getClient()} assemblies={assemblies} primary={primary} onClose={() => setProjectOpen(false)} />}
    {exportOpen && active && <ExportDialog key={`${active.id}-${exportOpen}`} mode={exportOpen} client={getClient()} assembly={active.id} info={active.info} filename={active.filename} member={activeDeclaration} method={busy || methodAssembly !== active.id ? null : method} onClose={() => setExportOpen(null)} />}
    {editOpen && active && selected && selected > 0 && <ILEditorDialog key={`${active.id}-${selected}`} client={getClient()} assembly={active.id} token={selected} name={selectedLabel} settings={editorSettings} theme={theme} onClose={() => setEditOpen(false)} onExport={() => {setEditOpen(false); setExportOpen('binary')}} />}
    {settingsOpen && <SettingsDialog onlineReferences={onlineReferences} onOnlineReferences={updateOnlineReferences} settings={editorSettings} onChange={setEditorSettings} appearance={theme} onAppearance={setTheme} onClose={() => setSettingsOpen(false)} />}
    {bookmarksOpen && <div className="fixed inset-0 z-80 flex items-center justify-center bg-black/50 p-4" onClick={() => setBookmarksOpen(false)}><div role="dialog" aria-modal="true" aria-label="Bookmarks" className="max-h-[80dvh] w-120 max-w-full overflow-auto rounded-lg border border-border bg-popover p-5 shadow-lg" onClick={event => event.stopPropagation()}><div className="mb-4 flex items-center justify-between"><h2 className="text-sm font-semibold">Bookmarks</h2><button className={ui.iconButton} aria-label="Close bookmarks" onClick={() => setBookmarksOpen(false)}><X size={16}/></button></div>{navigation.bookmarks.map(location => <div key={locationKey(location)} className="flex items-center gap-2 border-t border-border py-2"><button className="min-w-0 flex-1 truncate text-left text-xs hover:underline" onClick={() => {navigate(location); setBookmarksOpen(false)}}>{documentLabel(location)}</button><button className={ui.iconButton} aria-label={`Remove ${documentLabel(location)}`} onClick={() => setNavigation(previous => toggleBookmark(previous, location))}><X size={12}/></button></div>)}</div></div>}
    {helpOpen && <div className="modal-backdrop fixed [inset:0] z-80 bg-[#0008] flex justify-center items-start pt-[min(15vh,130px)] [backdrop-filter:none]" onClick={() => setHelpOpen(false)}>
      <div className="help-dialog w-122.5 max-w-[90vw] max-h-[calc(100dvh-min(15vh,130px)-16px)] overflow-y-auto border border-solid border-border p-8 relative shadow-[var(--shadow)] rounded-[8px] bg-popover [&_h2]:tracking-[-.7px] [&_h2]:mt-[21px] [&_h2]:mr-0 [&_h2]:mb-[15px] [&_h2]:ml-0 [&_h2]:text-[24px] [&_h2]:font-semibold [&_p]:text-muted-foreground [&_p]:text-[13px] [&_p]:leading-[1.7] [&>.button]:mt-4" role="dialog" aria-modal="true" aria-label="About ILens" onClick={e => e.stopPropagation()}>
        <button className={cn(ui.iconButton, "close-dialog absolute right-3.5 top-3.5")} aria-label="Close about" onClick={() => setHelpOpen(false)}><X size={18} /></button>
        <span className="brand-mark inline-flex items-center justify-center w-[31px] h-[31px] text-foreground bg-transparent border border-solid border-border rounded-md"><Code2 size={25} /></span>
        <h2>Understand the binary.</h2>
        <p>ILens is a static .NET decompiler.</p>
        <p>Open a managed DLL or EXE, choose a method, and compare reconstructed C# with its original CIL. Use <kbd>⌘/Ctrl K</kbd> to search metadata, and <kbd>⌘/Ctrl F</kbd> inside the code viewer.</p>
        <p>This is an initial decompiler release. Simple methods and control-flow patterns reconstruct into C#-like code. Complex async, iterator, closure, and exception patterns may remain annotated IL. Local variable names are synthetic; this is not the original source.</p>
        <p>Missing dependencies do not block analysis. Opening an EXE or DLL automatically resolves dependencies from loaded assemblies, your selected folder, and supported Microsoft .NET reference packs. Online downloads can be disabled in Settings. Loaded assemblies are never executed.</p>
        <section aria-labelledby="legal-notice-heading" className="mt-5 space-y-3 border-t border-border pt-5">
          <h3 id="legal-notice-heading" className="text-sm font-semibold">Legal use and disclaimer</h3>
          <p>ILens is intended for lawful software inspection, debugging, interoperability, and security research. You are responsible for ensuring that you have the legal right to inspect, decompile, modify, or redistribute the software you analyze, and for complying with applicable laws, licenses, and third-party rights. The project does not endorse unlawful use.</p>
          <p>ILens is provided &quot;as is&quot;, without warranties, under the <a className="underline underline-offset-2 hover:text-foreground" href="https://github.com/yusufgenc34/ILens/blob/main/LICENSE" target="_blank" rel="noopener noreferrer">MIT License</a>. To the extent permitted by applicable law, its authors and contributors disclaim liability for damages arising from use or misuse. This notice does not modify the MIT License or exclude liability that cannot legally be excluded.</p>
        </section>
        <button className={cn(ui.button, ui.primary)} onClick={() => setHelpOpen(false)}>Back to workspace<ArrowRight size={14} /></button>
      </div>
    </div>}
  </div>
}
