'use client'
import {useEffect, useRef, useState} from 'react'
import {ArrowUpRight, LoaderCircle} from 'lucide-react'
import type {DecompilerClient} from '../lib/rpc'
import {diagnostic, tokenHex, type AnalysisHit, type Loaded} from '../lib/types'
import type {Location} from '../lib/navigation'
import {ui} from '../lib/ui'

export default function AnalysisPanel({getClient, assemblies, assembly, token, onNavigate}: {getClient: () => DecompilerClient; assemblies: Loaded[]; assembly: number; token: number; onNavigate: (location: Location) => void}) {
  const [hits, setHits] = useState<AnalysisHit[]>([])
  const [status, setStatus] = useState('')
  const [busy, setBusy] = useState(false)
  const [filter, setFilter] = useState('all')
  const [outgoing, setOutgoing] = useState(false)
  const request = useRef<AbortController | null>(null)
  useEffect(() => {request.current?.abort(); setHits([]); setStatus(''); setBusy(false); setFilter('all'); return () => request.current?.abort()}, [assembly, token, assemblies])
  async function analyze(out: boolean) {
    request.current?.abort()
    const controller = new AbortController(); request.current = controller
    setBusy(true); setHits([]); setOutgoing(out); setFilter('all')
    const found: AnalysisHit[] = []; let skipped = 0, truncated = false
    try {
      const sources = out ? assemblies.filter(a => a.id === assembly) : assemblies
      for (const source of sources) {
        let cursor = 0
        while (!controller.signal.aborted) {
          const page = await getClient().call('analyzeReferences', {assembly, token, source:source.id, cursor, outgoing:out}, {signal:controller.signal})
          if (controller.signal.aborted) return
          skipped += page.skipped; truncated ||= page.truncated
          found.push(...page.hits)
          setHits(found.slice(0,1000))
          setStatus(`${source.info.identity.name}: ${page.next}/${page.total} declarations`)
          if (page.done || found.length >= 1000) break
          cursor = page.next
        }
        if (found.length >= 1000) {truncated = true; break}
      }
      setStatus(`${Math.min(found.length,1000)} references${skipped ? ` · ${skipped} unreadable methods skipped` : ''}${truncated ? ' · results truncated' : ''}`)
    } catch (error) {if (!controller.signal.aborted) setStatus(diagnostic(error).message)}
    finally {if (!controller.signal.aborted) setBusy(false)}
  }
  const visible = hits.filter(hit => filter === 'all' || hit.relation === filter)
  return <section className="mt-5 border-t border-border pt-4" aria-label="Analyze declaration">
    <div className="mb-3 flex items-center justify-between gap-2"><h3 className="text-xs font-medium">Analyze</h3>{busy && <button className="text-xs hover:underline" onClick={() => {request.current?.abort(); setBusy(false); setStatus('Analysis cancelled; partial results shown.')}}>Cancel</button>}</div>
    <div className="flex flex-wrap gap-2"><button className={ui.button} disabled={busy} onClick={() => void analyze(false)}>Find references</button><button className={ui.button} disabled={busy || token >>> 24 !== 6} onClick={() => void analyze(true)}>Calls &amp; uses</button></div>
    <p className="my-2 text-[11px] leading-relaxed text-muted-foreground">Static references across loaded assemblies. Runtime dispatch and reflection are not inferred.</p>
    {status && <p role="status" className="my-2 flex items-center gap-1 text-[11px] text-muted-foreground">{busy && <LoaderCircle size={12} className="animate-spin"/>}{status}</p>}
    {hits.length > 0 && <><select aria-label="Analysis relation" className="my-2 w-full rounded border border-input bg-background px-2 py-1 text-xs" value={filter} onChange={event => setFilter(event.target.value)}>{[['all','All relationships'],['calls','Method calls'],['creates','Object creation'],['reads','Field reads'],['writes','Field writes'],['address','Field addresses'],['extends','Derived types'],['implements','Interface implementations'],['method_group','Method groups'],['references','Other references']].map(([value,label]) => <option key={value} value={value}>{label}</option>)}</select>
      <div className="max-h-100 overflow-auto">{visible.map((hit,index) => <button key={`${hit.assembly}:${hit.token}:${hit.offset}:${index}`} className="block w-full rounded px-1 py-2 text-left hover:bg-accent" onClick={() => onNavigate({assembly:hit.assembly, token:hit.token, view:outgoing ? 'C#' : hit.offset === null ? 'Metadata' : 'IL'})} title={`${hit.name} · ${tokenHex(hit.token)}`}><span className="flex items-center gap-1 text-[11px]"><span className="truncate">{hit.name}</span><ArrowUpRight size={11} className="shrink-0"/></span><span className="block truncate text-[10px] text-muted-foreground">{assemblies.find(a => a.id === hit.assembly)?.info.identity.name} · {hit.relation}{hit.offset !== null ? ` · IL_${hit.offset.toString(16).padStart(4,'0').toUpperCase()}` : ''}</span></button>)}</div></>}
  </section>
}
