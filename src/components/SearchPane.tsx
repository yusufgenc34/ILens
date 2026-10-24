'use client'
import {cn, ui} from '../lib/ui'
import {useEffect, useState} from 'react'
import {Search, X} from 'lucide-react'
import type {DecompilerClient} from '../lib/rpc'
import {diagnostic, tokenHex, type Diagnostic, type SearchHit} from '../lib/types'
const filters = {
  all: {label: 'All', kinds: null},
  types: {label: 'Types', kinds: ['class', 'struct', 'interface', 'enum', 'delegate']},
  methods: {label: 'Methods', kinds: ['method', 'constructor']},
  fields: {label: 'Fields', kinds: ['field']},
  properties: {label: 'Properties', kinds: ['property']},
  events: {label: 'Events', kinds: ['event']},
  strings: {label: 'Strings', kinds: ['string']},
  resources: {label: 'Resources', kinds: ['resource']},
} as const
type Filter = keyof typeof filters
type Hit = SearchHit & {assembly: number}
/** dnSpy's docked Search window: results across the selected or every assembly, filtered by kind. */
export default function SearchPane({client, assemblies, current, onOpen, onClose}: {client: DecompilerClient; assemblies: {id: number; name: string}[]; current: number | null; onOpen: (assembly: number, hit: SearchHit) => void; onClose: () => void}) {
  const [query, setQuery] = useState('')
  const [filter, setFilter] = useState<Filter>('all')
  const [everywhere, setEverywhere] = useState(false)
  const [hits, setHits] = useState<Hit[]>([])
  const [searching, setSearching] = useState(false)
  const [error, setError] = useState<Diagnostic | null>(null)
  useEffect(() => {
    const text = query.trim()
    if (!text) {setHits([]); return}
    const abort = new AbortController()
    const scope = everywhere ? assemblies : assemblies.filter(a => a.id === current)
    setSearching(true); setError(null)
    const timer = setTimeout(() => {
      void Promise.all(scope.map(a => client.call('search', {assembly: a.id, query: text.slice(0, 256)}, {signal: abort.signal}).then(found => found.map(hit => ({...hit, assembly: a.id})))))
        .then(found => {if (!abort.signal.aborted) setHits(found.flat())})
        .catch(e => {if (!abort.signal.aborted) setError(diagnostic(e))})
        .finally(() => {if (!abort.signal.aborted) setSearching(false)})
    }, 250)
    return () => {clearTimeout(timer); abort.abort()}
  }, [client, query, everywhere, assemblies, current])
  const kinds = filters[filter].kinds as readonly string[] | null
  const shown = kinds ? hits.filter(h => kinds.includes(h.kind)) : hits
  const name = (id: number) => assemblies.find(a => a.id === id)?.name ?? ''
  return <section className="search-pane flex flex-col min-h-0 h-full text-[12px]" aria-label="Search window">
    <header className="flex flex-wrap items-center gap-2 px-3 py-1.5 border-b border-solid border-border">
      <Search size={14} className="text-muted-foreground" />
      <input aria-label="Search window query" placeholder="Search names, strings or a token…" value={query} maxLength={256} onChange={e => setQuery(e.target.value)} className="flex-1 min-w-40 bg-background text-foreground border border-solid border-border rounded-[4px] py-1 px-2 text-[12px]" />
      <select aria-label="Search kind" value={filter} onChange={e => setFilter(e.target.value as Filter)} className="bg-background text-foreground border border-solid border-border rounded-[4px] py-1 px-2 text-[12px]">{Object.entries(filters).map(([id, f]) => <option key={id} value={id}>{f.label}</option>)}</select>
      <label className="flex items-center gap-1.5 text-muted-foreground"><input type="checkbox" checked={everywhere} onChange={e => setEverywhere(e.target.checked)} />All assemblies</label>
      <span className="text-muted-foreground">{searching ? 'Searching…' : `${shown.length} results`}</span>
      <button className={cn(ui.iconButton)} aria-label="Close search window" onClick={onClose}><X size={14} /></button>
    </header>
    {error && <p role="alert" className="px-3 py-1 text-warning-foreground">{error.message} {error.detail}</p>}
    <div className="flex-1 min-h-0 overflow-auto"><table className="w-full border-collapse [&_th]:sticky [&_th]:top-0 [&_th]:bg-muted [&_th]:text-left [&_th]:font-medium [&_th]:text-muted-foreground [&_th]:px-3 [&_th]:py-1 [&_td]:px-3 [&_td]:py-1 [&_td]:border-t [&_td]:border-solid [&_td]:border-t-border [&_td]:whitespace-nowrap [&_td]:overflow-hidden [&_td]:text-ellipsis [&_td]:max-w-[420px]">
      <thead><tr><th>Name</th><th>Kind</th><th>Location</th>{everywhere && <th>Assembly</th>}<th>Token</th></tr></thead>
      <tbody>{shown.map((hit, i) => <tr key={`${hit.assembly}-${hit.token}-${i}`} className="cursor-pointer hover:bg-accent" onClick={() => onOpen(hit.assembly, hit)}><td><button className="bg-transparent border-0 p-0 text-inherit cursor-pointer [font:inherit] text-left" onClick={e => {e.stopPropagation(); onOpen(hit.assembly, hit)}}>{hit.name}</button></td><td className="text-muted-foreground">{hit.kind}</td><td className="text-muted-foreground">{hit.context}</td>{everywhere && <td className="text-muted-foreground">{name(hit.assembly)}</td>}<td className="font-mono text-muted-foreground">{tokenHex(hit.token)}</td></tr>)}</tbody>
    </table>{!shown.length && <p className="text-center text-muted-foreground py-6">{query.trim() ? searching ? 'Searching local metadata…' : 'No matches.' : 'Type to search the selected assembly, or all assemblies.'}</p>}</div>
  </section>
}
