'use client'
import {cn} from '../lib/ui'
import { useEffect, useMemo, useRef, useState } from 'react'
import { Box, ChevronRight, Braces, Code2, FileCode2, Folder, Hash, Link, Package, Variable, Zap } from 'lucide-react'
import { flattenTree, pathToToken, type TreeNode } from '../lib/tree'
function KindIcon({kind}: {kind: string}) {
  const Icon = ({assembly: Package, namespace: Braces, class: Box, struct: Box, interface: FileCode2, enum: Hash, delegate: Zap, field: Variable, property: Variable, event: Zap, method: Code2, constructor: Code2, reference: Link, references: Link, resource: FileCode2, resources: Folder, group: Folder} as Record<string, typeof Box>)[kind] ?? Box
  return <Icon size={14} className={`kind-${kind} text-muted-foreground`} aria-hidden="true" />
}
export default function Explorer({tree, selected, onSelect, onExport}: {tree: TreeNode[]; selected: number | null; onSelect: (token: number) => void; onExport: (token: number) => void}) {
  const [expanded, setExpanded] = useState<Set<string>>(new Set())
  const [scroll, setScroll] = useState(0)
  const [height, setHeight] = useState(600)
  const viewport = useRef<HTMLDivElement>(null)
  const rows = useMemo(() => flattenTree(tree, expanded), [tree, expanded])
  useEffect(() => {
    if (selected === null) return
    const path = pathToToken(tree, selected)
    if (path) setExpanded(previous => new Set([...previous, ...path.slice(0, -1)]))
  }, [tree, selected])
  useEffect(() => {
    const element = viewport.current
    if (!element) return
    const index = rows.findIndex(r => r.node.token === selected)
    if (index >= 0 && (index * 29 < element.scrollTop || index * 29 > element.scrollTop + element.clientHeight - 29)) element.scrollTop = Math.max(0, index * 29 - element.clientHeight / 2)
  }, [selected, rows])
  useEffect(() => { const element = viewport.current; if (!element) return; const observer = new ResizeObserver(() => setHeight(element.clientHeight)); observer.observe(element); return () => observer.disconnect() }, [])
  const start = Math.max(0, Math.floor(scroll / 29) - 10)
  const visible = rows.slice(start, start + Math.ceil(height / 29) + 20)
  const toggle = (key: string) => setExpanded(previous => {const next = new Set(previous); if (next.has(key)) next.delete(key); else next.add(key); return next})
  return <div ref={viewport} className="tree-scroll flex-1 overflow-auto [scrollbar-width:thin] [scrollbar-color:var(--border)_transparent] py-[5px] px-0" role="tree" aria-label="Assembly declarations" onScroll={e => setScroll(e.currentTarget.scrollTop)}>
    <div style={{height: rows.length * 29, position: 'relative'}}>
      {visible.map(({node, depth}, index) => <div key={node.key} style={{position: 'absolute', top: (start + index) * 29, height: 29, width: '100%'}}>
        <button role="treeitem" aria-level={depth + 1} aria-expanded={node.children.length ? expanded.has(node.key) : undefined} aria-selected={node.token === selected} className={cn(`tree-row flex gap-[7px] items-center h-[29px] border-0 bg-transparent text-left whitespace-nowrap w-full text-text pr-3 text-[12px] hover:bg-hover [&.selected]:shadow-none [&.selected]:bg-accent [&.selected]:text-accent-foreground [&.selected]:font-medium [&>svg]:shrink-0 ${node.token === selected ? 'selected' : ''}`)} style={{paddingLeft: depth * 15 + 10}} title={node.declaration?.full_name ?? node.name}
          onContextMenu={event => {if (node.token !== undefined && [2, 6].includes(node.token >>> 24)) {event.preventDefault(); onExport(node.token)}}}
          onClick={() => { if (node.token !== undefined) onSelect(node.token); else toggle(node.key) }}
          onDoubleClick={() => { if (node.children.length) toggle(node.key) }}
          onKeyDown={e => { if (e.key === 'ArrowRight' && node.children.length) {e.preventDefault(); setExpanded(new Set([...expanded, node.key]))} if (e.key === 'ArrowLeft' && expanded.has(node.key)) {e.preventDefault(); toggle(node.key)} if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {e.preventDefault(); const i = start + index + (e.key === 'ArrowDown' ? 1 : -1); const next = rows[i]; if (next) {if (next.node.token !== undefined) onSelect(next.node.token); viewport.current?.querySelectorAll<HTMLButtonElement>('[role="treeitem"]')[index + (e.key === 'ArrowDown' ? 1 : -1)]?.focus()}} }}>
          <span className={cn(`disclosure w-2.5 h-[19px] flex items-center justify-center shrink-0 text-muted-foreground [&.open_svg]:rotate-90 ${expanded.has(node.key) ? 'open' : ''}`)} onClick={e => {if (node.children.length) {e.stopPropagation(); toggle(node.key)}}}>{node.children.length > 0 && <ChevronRight size={12} />}</span>
          <KindIcon kind={node.kind} /><span className="tree-name overflow-hidden text-ellipsis">{node.name}</span>{node.children.length > 0 && <span className="tree-count ml-auto text-faint text-[9px]">{node.children.length}</span>}
        </button>
      </div>)}
    </div>
  </div>
}
