'use client'
import type {PeHeaders} from '../lib/types'
import {peTokens} from '../lib/tree'
const hex = (value: number, width = 8) => `0x${value.toString(16).toUpperCase().padStart(width, '0')}`
const cell = 'py-[5px] px-2.5 border-t border-solid border-t-border text-left align-top'
const head = 'sticky top-0 z-1 bg-muted py-[7px] px-2.5 text-left font-medium text-muted-foreground'
// Entropy above ~7.2 bits/byte usually means compressed or encrypted section data.
function Entropy({value}: {value: number}) {
  const high = value >= 7.2
  return <span className="inline-flex items-center gap-2"><span className="inline-block h-[5px] w-15 overflow-hidden rounded-full bg-muted"><span className="block h-full rounded-full" style={{width: `${value / 8 * 100}%`, background: high ? 'var(--warning-icon)' : 'var(--muted-foreground)'}} /></span><span className={high ? 'text-warning-foreground' : ''}>{value.toFixed(3)}</span></span>
}
export default function PeView({headers, token, onOffset}: {headers: PeHeaders | null; token: number; onOffset?: (offset: number) => void}) {
  // File offsets open the hex editor at that byte.
  const offset = (value: number) => onOffset ? <button className="bg-transparent border-0 p-0 [font:inherit] text-inherit cursor-pointer hover:underline" aria-label={`Show ${hex(value)} in hex editor`} onClick={() => onOffset(value)}>{hex(value)}</button> : hex(value)
  if (!headers) return <p role="status" className="p-4 text-[12px] text-muted-foreground">Reading PE headers…</p>
  const group = headers.groups.find(g => peTokens[g.id as keyof typeof peTokens] === token)
  return <div className="pe-view h-full overflow-auto font-mono text-[11px] [scrollbar-width:thin]" aria-label="PE header view">
    {token === peTokens.pe || token === peTokens.sections ? <>
      <div className="flex flex-wrap gap-x-6 gap-y-1 px-3 py-2.5 border-b border-solid border-b-border text-muted-foreground"><span>File entropy <Entropy value={headers.entropy} /></span><span>{headers.overlay ? `Overlay ${hex(headers.overlay[0])} · ${headers.overlay[1]} bytes` : 'No overlay'}</span></div>
      <table className="w-full border-collapse" aria-label="Sections"><thead><tr>{['Name', 'Virtual address', 'Virtual size', 'Raw offset', 'Raw size', 'Characteristics', 'Entropy'].map(h => <th key={h} className={head}>{h}</th>)}</tr></thead>
        <tbody>{headers.sections.map((s, i) => <tr key={`${s.name}-${i}`} className="hover:bg-hover"><td className={cell}>{s.name || '(unnamed)'}</td><td className={cell}>{hex(s.virtual_address)}</td><td className={cell}>{hex(s.virtual_size)}</td><td className={cell}>{offset(s.raw_offset)}</td><td className={cell}>{hex(s.raw_size)}</td><td className={cell} title={s.flags}>{hex(s.characteristics)} <span className="text-muted-foreground">{s.flags}</span></td><td className={cell}><Entropy value={s.entropy} /></td></tr>)}</tbody>
      </table>
    </> : null}
    {group && token !== peTokens.pe && <table className="w-full border-collapse" aria-label={group.title}><thead><tr>{['Name', 'Offset', 'Size', 'Value', 'Info'].map(h => <th key={h} className={head}>{h}</th>)}</tr></thead>
      <tbody>{group.fields.map((f, i) => <tr key={`${f.name}-${i}`} className="hover:bg-hover"><td className={cell} style={{color: 'var(--tree-field)'}}>{f.name}</td><td className={cell}>{offset(f.offset)}</td><td className={cell}>{f.size}</td><td className={cell}>{f.value}</td><td className={`${cell} font-sans text-muted-foreground [overflow-wrap:anywhere]`}>{f.info}</td></tr>)}</tbody>
    </table>}
    {!group && token !== peTokens.pe && token !== peTokens.sections && <p className="p-4 font-sans text-[12px] text-muted-foreground">This header is not present in the image.</p>}
  </div>
}
