'use client'
import {cn, ui} from '../lib/ui'
import {useEffect, useState} from 'react'
import type {DecompilerClient} from '../lib/rpc'
import {diagnostic, tokenHex, type Diagnostic, type TableRows} from '../lib/types'
const pageSize = 200
/** dnSpy's metadata table view: every row with its ECMA columns; index cells navigate. */
export default function MetadataTableView({client, assembly, table, name, onNavigate}: {client: DecompilerClient; assembly: number; table: number; name: string; onNavigate: (token: number) => void}) {
  const [page, setPage] = useState(0)
  const [data, setData] = useState<TableRows | null>(null)
  const [error, setError] = useState<Diagnostic | null>(null)
  useEffect(() => {
    const abort = new AbortController()
    setError(null)
    client.call('getTableRows', {assembly, table, start: page * pageSize + 1, count: pageSize}, {signal: abort.signal}).then(setData).catch(e => {if (!abort.signal.aborted) setError(diagnostic(e))})
    return () => abort.abort()
  }, [client, assembly, table, page])
  useEffect(() => setPage(0), [assembly, table])
  const pages = data ? Math.max(1, Math.ceil(data.total / pageSize)) : 1
  return <section className="metadata-table h-full flex flex-col min-h-0 text-[12px]" aria-label={`${name} metadata table`}>
    <header className="flex items-center gap-3 px-4 py-2 border-b border-solid border-border shrink-0"><strong className="font-medium">{name}</strong><span className="text-muted-foreground">{data ? `${data.total} rows · table 0x${table.toString(16).padStart(2, '0').toUpperCase()}` : 'Loading…'}</span>
      <span className="ml-auto flex items-center gap-2"><button className={cn(ui.button, ui.subtle)} disabled={page === 0} onClick={() => setPage(p => p - 1)}>Previous</button><span className="text-muted-foreground">Page {page + 1} / {pages}</span><button className={cn(ui.button, ui.subtle)} disabled={page + 1 >= pages} onClick={() => setPage(p => p + 1)}>Next</button></span></header>
    {error && <p role="alert" className="p-4 text-warning-foreground">{error.message} {error.detail}</p>}
    {data && <div className="overflow-auto flex-1 min-h-0"><table className="border-collapse w-max min-w-full font-mono text-[11.5px] [&_th]:sticky [&_th]:top-0 [&_th]:bg-muted [&_th]:text-left [&_th]:font-medium [&_th]:text-muted-foreground [&_th]:px-3 [&_th]:py-1.5 [&_td]:px-3 [&_td]:py-1 [&_td]:border-t [&_td]:border-solid [&_td]:border-t-border [&_td]:whitespace-nowrap [&_td]:max-w-[480px] [&_td]:overflow-hidden [&_td]:text-ellipsis">
      <thead><tr><th>RID</th><th>Token</th>{data.columns.map(c => <th key={c}>{c}</th>)}</tr></thead>
      <tbody>{data.rows.map(row => <tr key={row.row} className="hover:bg-accent"><td className="text-muted-foreground">{row.row}</td><td><button className="bg-transparent border-0 p-0 text-inherit cursor-pointer hover:underline" onClick={() => onNavigate(row.token)}>{tokenHex(row.token)}</button></td>{row.cells.map((cell, i) => <td key={i} title={cell.text}>{cell.token !== undefined && (cell.token & 0xffffff) !== 0 ? <button className="bg-transparent border-0 p-0 text-[var(--tree-type)] cursor-pointer hover:underline [font:inherit]" onClick={() => onNavigate(cell.token!)}>{cell.text}</button> : cell.text}</td>)}</tr>)}</tbody>
    </table></div>}
  </section>
}
