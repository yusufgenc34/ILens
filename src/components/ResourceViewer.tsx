'use client'
import {useEffect, useState} from 'react'
import {Download, LoaderCircle} from 'lucide-react'
import type {DecompilerClient} from '../lib/rpc'
import {diagnostic, type ResourcePreview} from '../lib/types'
import {downloadFile, safeName} from '../lib/export'
import {ui} from '../lib/ui'

export default function ResourceViewer({client, assembly, token, name, hex}: {client:DecompilerClient; assembly:number; token:number; name:string; hex:boolean}) {
  const [preview, setPreview] = useState<ResourcePreview | null>(null)
  const [detail, setDetail] = useState<ResourcePreview | null>(null)
  const [entry, setEntry] = useState<number | null>(null)
  const [image, setImage] = useState<string | null>(null)
  const [error, setError] = useState('')
  const [query, setQuery] = useState('')
  const [page, setPage] = useState(0)
  useEffect(() => {
    const controller = new AbortController(); setPreview(null); setError(''); setEntry(null); setDetail(null); setQuery(''); setPage(0)
    void client.call('getResourcePreview', {assembly,token,entry:-1}, {signal:controller.signal}).then(setPreview).catch(error => {if (!controller.signal.aborted) setError(diagnostic(error).detail)})
    return () => controller.abort()
  }, [client, assembly, token])
  useEffect(() => {
    const controller = new AbortController(); setDetail(null)
    if (entry === null) return
    void client.call('getResourcePreview', {assembly,token,entry}, {signal:controller.signal}).then(setDetail).catch(error => {if (!controller.signal.aborted) setError(diagnostic(error).detail)})
    return () => controller.abort()
  }, [client, assembly, token, entry])
  const shown = entry === null ? preview : detail
  useEffect(() => {
    const controller = new AbortController(); let url: string | undefined; setImage(null)
    if (shown?.kind !== 'image/png') return
    void client.call('getResourceBytes', {assembly,token,entry:entry ?? -1}, {signal:controller.signal}).then(({buffer}) => {
      if (controller.signal.aborted) return
      url = URL.createObjectURL(new Blob([buffer],{type:'image/png'})); setImage(url)
    }).catch(error => {if (!controller.signal.aborted) setError(diagnostic(error).detail)})
    return () => {controller.abort(); if (url) URL.revokeObjectURL(url)}
  }, [client, assembly, token, entry, shown?.kind])
  async function download(selected: number | null) {
    try {
      const {buffer} = await client.call('getResourceBytes', {assembly,token,entry:selected ?? -1})
      const item = preview?.entries.find(e => e.index === selected)
      downloadFile({buffer, mime:'application/octet-stream', name:safeName(item?.name ?? name) + (item?.kind === 'string' ? '.txt' : '')})
    } catch (error) {setError(diagnostic(error).detail)}
  }
  const entries = (preview?.entries ?? []).filter(e => e.name.toLowerCase().includes(query.toLowerCase()))
  return <section className="h-full overflow-auto p-5 text-xs" aria-label="Resource viewer">
    <div className="flex flex-wrap items-center justify-between gap-3"><div className="min-w-0"><h2 className="break-all text-sm font-medium">{name}</h2>{preview && <p className="mt-1 text-muted-foreground">{preview.size.toLocaleString()} bytes · {preview.kind}</p>}</div><button className={ui.button} onClick={() => void download(null)}><Download size={14}/>Save resource</button></div>
    {error && <p role="alert" className="my-3 break-words text-destructive">{error}</p>}
    {!preview && !error && <p className="mt-4 flex items-center gap-2"><LoaderCircle size={14} className="animate-spin"/>Reading resource…</p>}
    {preview?.diagnostic && <p className="my-3 text-muted-foreground">{preview.diagnostic}</p>}
    {preview?.kind === 'resources' && !hex && <>
      <input aria-label="Filter resource entries" placeholder="Filter entries…" value={query} onChange={event => {setQuery(event.target.value); setPage(0)}} className="mb-3 w-full rounded border border-input bg-background px-3 py-2"/>
      <div className="max-h-72 overflow-auto rounded border border-border"><table className="w-full table-fixed text-left"><thead className="sticky top-0 bg-muted"><tr><th className="w-1/3 px-3 py-2 font-medium">Name</th><th className="w-24 px-3 py-2 font-medium">Type</th><th className="px-3 py-2 font-medium">Value</th></tr></thead><tbody>{entries.slice(page*100,(page+1)*100).map(item => <tr key={item.index} className={entry === item.index ? 'bg-accent' : ''}><td className="border-t border-border px-3 py-2"><button className="max-w-full truncate text-left hover:underline" title={item.name} onClick={() => {setError(''); setEntry(item.index)}}>{item.name}</button></td><td className="border-t border-border px-3 py-2">{item.kind}</td><td className="truncate border-t border-border px-3 py-2" title={item.value ?? ''}>{item.value ?? `${item.size} bytes`}</td></tr>)}</tbody></table></div>
      {entries.length > 100 && <div className="my-2 flex gap-2"><button className={ui.button} disabled={page === 0} onClick={() => setPage(p => p-1)}>Previous</button><span>{page+1}/{Math.ceil(entries.length/100)}</span><button className={ui.button} disabled={(page+1)*100 >= entries.length} onClick={() => setPage(p => p+1)}>Next</button></div>}
      {entry !== null && <div className="my-3 flex items-center justify-between gap-2"><span className="truncate">{preview.entries.find(e => e.index === entry)?.name}</span><button className={ui.button} onClick={() => void download(entry)}><Download size={14}/>Save entry</button></div>}
    </>}
    {image && !hex && <img src={image} alt="Embedded resource preview" className="my-4 max-h-100 max-w-full object-contain"/>}
    {shown && (hex || shown.kind !== 'resources') && <pre className="mt-4 overflow-auto whitespace-pre-wrap break-words rounded border border-border bg-muted p-4 font-mono text-xs leading-relaxed">{hex ? preview?.hex : shown.text ?? shown.hex}</pre>}
    {(shown?.size ?? 0) > 4096 && (hex || shown?.kind === 'binary') && <p className="mt-2 text-muted-foreground">Hex preview shows the first 4,096 bytes. Download preserves the complete resource.</p>}
  </section>
}
