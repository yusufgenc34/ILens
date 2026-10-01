'use client'
import {cn} from '../lib/ui'
import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react'
import { Box, ChevronRight, Braces, Code2, FileCode2, Folder, Hash, Link, Unlink, Package, Variable, Zap } from 'lucide-react'
import { flattenTree, pathToToken, type TreeNode } from '../lib/tree'
function KindIcon({kind}: {kind: string}) {
  const Icon = ({assembly: Package, module:FileCode2, namespace: Braces, class: Box, struct: Box, interface: FileCode2, enum: Hash, delegate: Zap, field: Variable, property: Variable, event: Zap, method: Code2, constructor: Code2, reference: Link, references: Link, resource: FileCode2, resources: Folder} as Record<string, typeof Box>)[kind] ?? Box
  return <Icon size={14} className={`kind-${kind} text-muted-foreground`} aria-hidden="true" />
}
const rowHeight = 26
const nodeId = (key:string) => `explorer-${key}`
export default function Explorer({tree, selected, onSelect, onExport, onReference, selectedAssembly}: {selectedAssembly:number | null; onReference:(assembly:number,token:number)=>void; tree: TreeNode[]; selected: number | null; onSelect: (assembly:number, token:number) => void; onExport:(assembly:number,token:number)=>void}) {
  const [expanded, setExpanded] = useState<Set<string>>(new Set())
  const [focused, setFocused] = useState<string | null>(null)
  const [scroll, setScroll] = useState(0)
  const [height, setHeight] = useState(600)
  const revealed = useRef('')
  const viewport = useRef<HTMLDivElement>(null)
  const rows = useMemo(() => flattenTree(tree, expanded), [tree, expanded])
  useEffect(() => {
    if (selected === null) return
    const key = `${selectedAssembly}:${selected}`
    if (revealed.current === key) return
    const path = pathToToken(tree, selected, selectedAssembly ?? undefined)
    if (path) {
      revealed.current = key
      const root = tree.find(node => node.assembly === selectedAssembly)
      setExpanded(previous => new Set([...previous, ...path.slice(0, -1), ...(selected === 0 ? [...path,...(root?.children.map(n=>n.key) ?? [])] : [])]))
      setFocused(path.at(-1)!)
    }
  }, [tree, selected, selectedAssembly])
  useEffect(() => {
    const element = viewport.current
    if (!element) return
    const index = rows.findIndex(r => r.node.key === focused)
    if (index >= 0 && (index * rowHeight < element.scrollTop || index * rowHeight > element.scrollTop + element.clientHeight - rowHeight)) element.scrollTop = Math.max(0, index * rowHeight - element.clientHeight / 2)
  }, [focused, rows])
  useEffect(() => { const element = viewport.current; if (!element) return; const observer = new ResizeObserver(() => setHeight(element.clientHeight)); observer.observe(element); return () => observer.disconnect() }, [])
  const start = Math.max(0, Math.floor(scroll / rowHeight) - 10)
  const visible = rows.slice(start, start + Math.ceil(height / rowHeight) + 20)
  const toggle = (key: string) => setExpanded(previous => {const next = new Set(previous); if (next.has(key)) next.delete(key); else next.add(key); return next})
  function choose(node:TreeNode) {
    setFocused(node.key); viewport.current?.focus({preventScroll:true})
    if (node.token !== undefined) onSelect(node.assembly!,node.token)
  }
  function keyboard(event:KeyboardEvent) {
    const index = Math.max(0,rows.findIndex(row=>row.node.key===focused)), row=rows[index]
    if (!row) return
    let next=index
    if(event.key==='ArrowDown') next=Math.min(rows.length-1,index+1)
    else if(event.key==='ArrowUp') next=Math.max(0,index-1)
    else if(event.key==='Home') next=0
    else if(event.key==='End') next=rows.length-1
    else if(event.key==='ArrowRight') {
      if(row.node.children.length && !expanded.has(row.node.key)) setExpanded(previous=>new Set([...previous,row.node.key]))
      else if(row.node.children.length) next=index+1
    } else if(event.key==='ArrowLeft') {
      if(expanded.has(row.node.key)) toggle(row.node.key)
      else {for(let i=index-1;i>=0;i--) if(rows[i].depth<row.depth) {next=i;break}}
    } else if(event.key==='Enter') {
      if(row.node.reference) onReference(row.node.assembly!,row.node.token!)
      else {choose(row.node);if(row.node.children.length) toggle(row.node.key)}
    } else return
    event.preventDefault()
    if(next!==index) choose(rows[next].node)
  }
  return <div ref={viewport} className="tree-scroll flex-1 overflow-auto outline-none [scrollbar-width:thin] [scrollbar-color:var(--border)_transparent] py-1 px-0" role="tree" aria-label="Assembly declarations" tabIndex={0} aria-activedescendant={visible.some(row=>row.node.key===focused) ? nodeId(focused!) : undefined} onKeyDown={keyboard} onScroll={e => setScroll(e.currentTarget.scrollTop)}>
    <div role="presentation" style={{height: rows.length * rowHeight, position: 'relative'}}>
      {visible.map(({node, depth}, index) => <div role="presentation" key={node.key} style={{position: 'absolute', top: (start + index) * rowHeight, height: rowHeight, width: '100%'}}>
        <button id={nodeId(node.key)} tabIndex={-1} role="treeitem" data-kind={node.kind} data-assembly-id={node.assembly} aria-label={node.name} aria-level={depth + 1} aria-expanded={node.children.length ? expanded.has(node.key) : undefined} aria-selected={node.token === selected && node.assembly === selectedAssembly} className={cn('tree-row flex gap-1.5 items-center h-[26px] border-0 bg-transparent text-left whitespace-nowrap w-full text-text pr-2 text-[12px] hover:bg-hover [&>svg]:shrink-0',node.token === selected && node.assembly === selectedAssembly && 'selected bg-accent text-accent-foreground',focused===node.key && '[[role=tree]:focus_&]:outline [[role=tree]:focus_&]:-outline-offset-1 [[role=tree]:focus_&]:outline-border')} style={{paddingLeft: depth * 15 + 6}} title={node.reference ? `${node.name}, ${node.reference.identity.version} · ${node.reference.resolved_id !== null ? 'Resolved · double-click to open' : 'Unresolved · double-click to resolve'}` : node.declaration?.full_name ?? node.name}
          onContextMenu={event => {if (node.token !== undefined && [2, 6].includes(node.token >>> 24)) {event.preventDefault(); onExport(node.assembly!,node.token)}}}
          onClick={() => {choose(node); if(node.token===undefined) toggle(node.key)}}
          onDoubleClick={() => {if (node.reference) onReference(node.assembly!,node.token!); else if (node.children.length && node.token!==undefined) toggle(node.key)}}>
          <span className={cn('disclosure w-2.5 h-[19px] flex items-center justify-center shrink-0 text-muted-foreground',expanded.has(node.key) && '[&_svg]:rotate-90')} onClick={e => {if (node.children.length) {e.stopPropagation();setFocused(node.key);viewport.current?.focus({preventScroll:true});toggle(node.key)}}}>{node.children.length > 0 && <ChevronRight size={12} />}</span>
          <KindIcon kind={node.kind} /><span className="tree-name overflow-hidden text-ellipsis">{node.name}</span>{node.reference && <span className="ml-auto shrink-0 text-muted-foreground" aria-label={node.reference.resolved_id !== null ? 'Resolved reference' : 'Unresolved reference'}>{node.reference.resolved_id !== null ? <Link size={11}/> : <Unlink size={11}/>}</span>}
        </button>
      </div>)}
    </div>
  </div>
}
