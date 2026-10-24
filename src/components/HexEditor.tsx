'use client'
import {cn, ui} from '../lib/ui'
import {useEffect, useMemo, useRef, useState, type KeyboardEvent} from 'react'
import {Download, Pencil, RotateCcw} from 'lucide-react'
import type {DecompilerClient} from '../lib/rpc'
import {diagnostic, type Diagnostic} from '../lib/types'
const rowBytes = 16
const rowHeight = 20
const pageBytes = 4096
const hex = (n: number, width: number) => n.toString(16).toUpperCase().padStart(width, '0')
/** dnSpy's hex editor over the whole file: offsets, bytes and ASCII, with optional raw byte patches. */
export default function HexEditor({client, assembly, size, filename, initialOffset = 0}: {client: DecompilerClient; assembly: number; size: number; filename: string; initialOffset?: number}) {
  const scroller = useRef<HTMLDivElement>(null)
  const pages = useRef(new Map<number, Uint8Array>())
  const pending = useRef(new Set<number>())
  const [, setLoaded] = useState(0)
  const [top, setTop] = useState(0); const [height, setHeight] = useState(400)
  const [selected, setSelected] = useState<number | null>(null); const [nibble, setNibble] = useState<number | null>(null)
  const [editing, setEditing] = useState(false)
  const [patches, setPatches] = useState(new Map<number, number>())
  const [goto, setGoto] = useState('')
  const [problem, setProblem] = useState<string | null | undefined>(undefined)
  const [error, setError] = useState<Diagnostic | null>(null)
  const rows = Math.ceil(size / rowBytes)
  const first = Math.max(0, Math.floor(top / rowHeight) - 4)
  const last = Math.min(rows, Math.ceil((top + height) / rowHeight) + 4)
  useEffect(() => {pages.current.clear(); pending.current.clear(); setPatches(new Map()); setProblem(undefined)}, [assembly])
  useEffect(() => {
    const element = scroller.current
    if (!element) return
    const observer = new ResizeObserver(() => setHeight(element.clientHeight))
    observer.observe(element)
    return () => observer.disconnect()
  }, [])
  useEffect(() => {jump(initialOffset)}, [initialOffset]) // eslint-disable-line react-hooks/exhaustive-deps
  // Fetch the pages the visible rows need. Requests are not cancelled when the view
  // scrolls on (a cancelled page would stay pending); results for another assembly
  // are dropped instead.
  const current = useRef(assembly)
  current.current = assembly
  useEffect(() => {
    for (let page = Math.floor(first * rowBytes / pageBytes); page <= Math.floor(Math.max(0, last * rowBytes - 1) / pageBytes); page++) {
      if (pages.current.has(page) || pending.current.has(page)) continue
      pending.current.add(page)
      client.call('readBytes', {assembly, offset: page * pageBytes, length: pageBytes})
        .then(({buffer}) => {if (current.current === assembly) {pages.current.set(page, new Uint8Array(buffer)); setLoaded(n => n + 1)}})
        .catch(e => {if (current.current === assembly) setError(diagnostic(e))})
        .finally(() => pending.current.delete(page))
    }
  }, [client, assembly, first, last])
  const byte = (offset: number) => patches.get(offset) ?? pages.current.get(Math.floor(offset / pageBytes))?.[offset % pageBytes]
  function jump(offset: number) {
    const target = Math.max(0, Math.min(size - 1, offset))
    setSelected(target); setNibble(null)
    scroller.current?.scrollTo({top: Math.floor(target / rowBytes) * rowHeight - rowHeight * 2})
  }
  function write(offset: number, value: number) {
    const original = pages.current.get(Math.floor(offset / pageBytes))?.[offset % pageBytes]
    setPatches(previous => {const next = new Map(previous); if (value === original) next.delete(offset); else next.set(offset, value); return next})
    setProblem(undefined)
  }
  function keyDown(event: KeyboardEvent) {
    if (selected === null) return
    const moves: Record<string, number> = {ArrowLeft: -1, ArrowRight: 1, ArrowUp: -rowBytes, ArrowDown: rowBytes}
    if (event.key in moves) {event.preventDefault(); jump(selected + moves[event.key]); return}
    if (!editing || !/^[0-9a-f]$/i.test(event.key)) return
    event.preventDefault()
    const digit = parseInt(event.key, 16)
    const current = byte(selected) ?? 0
    // Two hex digits per byte: high nibble, then low nibble and advance.
    if (nibble === null) {write(selected, (digit << 4) | (current & 0x0f)); setNibble(digit)}
    else {write(selected, (nibble << 4) | digit); setNibble(null); if (selected + 1 < size) jump(selected + 1)}
  }
  const list = useMemo(() => [...patches.entries()].sort((a, b) => a[0] - b[0]), [patches])
  async function check() {
    try {setProblem(await client.call('checkPatched', {assembly, patches: list}))} catch (e) {setError(diagnostic(e))}
  }
  async function download() {
    try {
      const {buffer} = await client.call('exportPatched', {assembly, patches: list})
      const url = URL.createObjectURL(new Blob([buffer], {type: 'application/octet-stream'}))
      const anchor = document.createElement('a'); anchor.href = url; anchor.download = filename.replace(/(\.\w+)?$/, '.patched$1')
      document.body.append(anchor); anchor.click(); anchor.remove()
      setTimeout(() => URL.revokeObjectURL(url), 1000)
    } catch (e) {setError(diagnostic(e))}
  }
  return <section className="hex-editor h-full flex flex-col min-h-0 text-[12px]" aria-label="Hex editor">
    <header className="flex flex-wrap items-center gap-2 px-4 py-2 border-b border-solid border-border shrink-0">
      <form className="flex items-center gap-2" onSubmit={e => {e.preventDefault(); const n = parseInt(goto.replace(/^0x/i, ''), 16); if (Number.isFinite(n)) jump(n)}}><input aria-label="Go to offset" placeholder="Offset (hex)" value={goto} onChange={e => setGoto(e.target.value)} className="w-32 bg-background text-foreground border border-solid border-border rounded-[4px] py-1 px-2 font-mono text-[12px]" /><button className={cn(ui.button, ui.subtle)}>Go</button></form>
      <span className="text-muted-foreground">{size.toLocaleString()} bytes{selected !== null ? ` · offset 0x${hex(selected, 8)}` : ''}</span>
      <span className="ml-auto flex items-center gap-2">
        <button className={cn(ui.button, editing ? ui.primary : ui.subtle)} aria-pressed={editing} onClick={() => {setEditing(e => !e); setNibble(null)}}><Pencil size={14} />{editing ? 'Editing' : 'Edit bytes'}</button>
        {patches.size > 0 && <>
          <span className="text-muted-foreground">{patches.size} changed</span>
          <button className={cn(ui.button, ui.subtle)} onClick={() => {setPatches(new Map()); setProblem(undefined)}}><RotateCcw size={14} />Revert</button>
          <button className={cn(ui.button, ui.subtle)} onClick={() => void check()}>Check</button>
          <button className={cn(ui.button, ui.primary)} onClick={() => void download()}><Download size={14} />Download patched file</button>
        </>}
      </span>
    </header>
    {editing && <p className="px-4 py-1.5 text-[11px] text-muted-foreground border-b border-solid border-border">Raw byte patches bypass IL and metadata verification. Check reports whether the result still parses as a managed assembly.</p>}
    {problem !== undefined && <p role="status" className={cn("px-4 py-1.5 text-[11px] border-b border-solid border-border", problem ? 'text-warning-foreground bg-warning-background' : 'text-muted-foreground')}>{problem ? `The patched file no longer parses as a managed assembly: ${problem}` : 'The patched file still parses as a managed assembly.'}</p>}
    {error && <p role="alert" className="px-4 py-1.5 text-warning-foreground">{error.message} {error.detail}</p>}
    <div ref={scroller} className="flex-1 min-h-0 overflow-auto font-mono text-[12px] outline-none" tabIndex={0} onScroll={e => setTop(e.currentTarget.scrollTop)} onKeyDown={keyDown} aria-label="File bytes">
      <div style={{height: rows * rowHeight, position: 'relative'}}>
        {Array.from({length: Math.max(0, last - first)}, (_, i) => first + i).map(row => {
          const base = row * rowBytes
          const count = Math.min(rowBytes, size - base)
          return <div key={row} className="absolute left-0 flex gap-4 px-4 whitespace-pre" style={{top: row * rowHeight, height: rowHeight, lineHeight: `${rowHeight}px`}}>
            <span className="text-muted-foreground">{hex(base, 8)}</span>
            <span className="flex gap-[6px]">{Array.from({length: count}, (_, i) => {const offset = base + i; const value = byte(offset); return <button key={i} aria-label={`Byte 0x${hex(offset, 8)}`} onClick={() => {setSelected(offset); setNibble(null)}} className={cn("bg-transparent border-0 p-0 [font:inherit] cursor-pointer w-[2ch]", offset === selected ? 'bg-[var(--selection)] text-foreground' : patches.has(offset) ? 'text-[var(--syntax-string,#f1fa8c)]' : 'text-inherit', i === 8 ? 'ml-[6px]' : '')}>{value === undefined ? '··' : hex(value, 2)}</button>})}</span>
            <span className="text-muted-foreground">{Array.from({length: count}, (_, i) => {const value = byte(base + i); return value === undefined ? ' ' : value >= 0x20 && value < 0x7f ? String.fromCharCode(value) : '.'}).join('')}</span>
          </div>
        })}
      </div>
    </div>
  </section>
}
