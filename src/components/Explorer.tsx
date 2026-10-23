'use client'
import {cn} from '../lib/ui'
import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react'
import { Box, ChevronRight, Braces, Code2, Cpu, FileCode2, FileText, Folder, FolderTree, GitFork, Hash, KeyRound, Layers, Link, Lock, Unlink, Package, Shield, ShieldAlert, Variable, Wrench, Zap, ArrowUpFromLine, ArrowDownToLine } from 'lucide-react'
import { accessOf, flattenTree, pathToToken, type TreeNode } from '../lib/tree'
import ContextMenu from './ContextMenu'
import type { MenuAction } from './AppMenuBar'
const icons: Record<string, typeof Box> = {assembly: Package, module: FileCode2, namespace: Braces, class: Box, struct: Layers, interface: GitFork, enum: Hash, delegate: Zap, field: Variable, property: Wrench, event: Zap, method: Code2, constructor: Code2, reference: Link, references: Link, resource: FileText, resources: Folder, pe: Cpu, 'pe-header': FileText, protection: ShieldAlert, 'base-types': ArrowUpFromLine, 'derived-types': ArrowDownToLine, 'base-type': FolderTree, 'derived-type': FolderTree}
const colors: Record<string, string> = {assembly: 'var(--tree-assembly)', module: 'var(--tree-assembly)', namespace: 'var(--tree-namespace)', class: 'var(--tree-type)', struct: 'var(--tree-value-type)', enum: 'var(--tree-value-type)', interface: 'var(--tree-interface)', delegate: 'var(--tree-delegate)', method: 'var(--tree-method)', constructor: 'var(--tree-method)', field: 'var(--tree-field)', property: 'var(--tree-property)', event: 'var(--tree-event)', references: 'var(--tree-folder)', resources: 'var(--tree-folder)', pe: 'var(--tree-folder)', protection: 'var(--warning-icon)', 'base-types': 'var(--tree-folder)', 'derived-types': 'var(--tree-folder)', 'base-type': 'var(--tree-type)', 'derived-type': 'var(--tree-type)'}
const overlays = {private: Lock, 'private protected': Lock, protected: KeyRound, 'protected internal': KeyRound, internal: Shield} as const
function KindIcon({node}: {node: TreeNode}) {
  const Icon = icons[node.kind] ?? Box
  const access = accessOf(node)
  const color = access?.constant ? 'var(--tree-constant)' : colors[node.kind]
  const Overlay = access && access.access !== 'public' ? overlays[access.access] : null
  return <span className="relative inline-flex shrink-0" aria-hidden="true"><Icon size={14} className={`kind-${node.kind}`} style={{color: color ?? 'var(--muted-foreground)'}} />{Overlay && <Overlay size={8} strokeWidth={3} className="access-overlay absolute -right-[3px] -bottom-[2px] rounded-[2px] bg-surface text-muted-foreground" />}</span>
}
// Member labels keep their signature, rendered in a quieter color than the name itself.
function Label({node}: {node: TreeNode}) {
  const text = node.name
  const split = ['method', 'constructor'].includes(node.kind) ? text.indexOf('(') : ['field', 'property', 'event'].includes(node.kind) ? text.indexOf(' : ') : -1
  const access = accessOf(node)
  const color = access?.constant ? 'var(--tree-constant)' : colors[node.kind]
  const name = split > 0 ? text.slice(0, split) : text
  // dnSpy-style metadata token suffix; it is decorative, so names stay the accessible label.
  const token = node.declaration ? ` @${node.declaration.token.toString(16).toUpperCase().padStart(8, '0')}` : ''
  return <span className="tree-name overflow-hidden text-ellipsis"><span style={{color}}>{name}</span>{split > 0 && <span style={{color: 'var(--tree-signature)'}}>{text.slice(split)}</span>}{token && <span aria-hidden="true" className="tree-token max-md:hidden" style={{color: 'var(--tree-signature)'}}>{token}</span>}</span>
}
function describe(node: TreeNode) {
  const access = accessOf(node)
  const prefix = access ? `${access.access}${access.isStatic ? ' static' : ''}${access.constant ? ' const' : ''} ` : ''
  return `${prefix}${node.declaration?.full_name ?? node.name}`
}
// Touch screens get taller rows so each item is a comfortable tap target.
const rowHeights = {fine: 26, coarse: 36}
const nodeId = (key:string) => `explorer-${key}`
export default function Explorer({tree, selected, onSelect, onLink, onReference, selectedAssembly, contextActions}: {selectedAssembly:number | null; onReference:(assembly:number,token:number)=>void; tree: TreeNode[]; selected: number | null; onSelect: (assembly:number, token:number) => void; onLink:(assembly:number,token:number)=>void; contextActions:(node:TreeNode, tree:{expand:(node:TreeNode)=>void; collapse:(node:TreeNode)=>void})=>(MenuAction | null)[]}) {
  const [expanded, setExpanded] = useState<Set<string>>(new Set())
  const [menu, setMenu] = useState<{x:number; y:number; node:TreeNode} | null>(null)
  const [coarse, setCoarse] = useState(false)
  useEffect(() => {const query = window.matchMedia('(pointer: coarse)'); const update = () => setCoarse(query.matches); update(); query.addEventListener('change', update); return () => query.removeEventListener('change', update)}, [])
  const rowHeight = coarse ? rowHeights.coarse : rowHeights.fine
  // iOS Safari has no contextmenu event, so a long press opens the same menu.
  const press = useRef<{timer: ReturnType<typeof setTimeout>; x: number; y: number; fired: boolean} | null>(null)
  const cancelPress = () => {if (press.current) clearTimeout(press.current.timer); press.current = null}
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
  }, [focused, rows, rowHeight])
  useEffect(() => { const element = viewport.current; if (!element) return; const observer = new ResizeObserver(() => setHeight(element.clientHeight)); observer.observe(element); return () => observer.disconnect() }, [])
  const start = Math.max(0, Math.floor(scroll / rowHeight) - 10)
  const visible = rows.slice(start, start + Math.ceil(height / rowHeight) + 20)
  const toggle = (key: string) => setExpanded(previous => {const next = new Set(previous); if (next.has(key)) next.delete(key); else next.add(key); return next})
  function choose(node:TreeNode) {
    setFocused(node.key); viewport.current?.focus({preventScroll:true})
    if (node.token !== undefined) onSelect(node.assembly!,node.token)
  }
  // Expand a node and every descendant, bounded so huge namespaces stay responsive.
  const expandAll = useCallback((node:TreeNode) => setExpanded(previous => {const next = new Set(previous); const stack = [node]; while (stack.length && next.size < previous.size + 5000) {const n = stack.pop()!; if (n.children.length) {next.add(n.key); stack.push(...n.children)}} return next}), [])
  const collapseAll = useCallback((node:TreeNode) => setExpanded(previous => {const next = new Set(previous); const stack = [node]; while (stack.length) {const n = stack.pop()!; next.delete(n.key); stack.push(...n.children)} return next}), [])
  const closeMenu = useCallback(() => {setMenu(null); viewport.current?.focus({preventScroll:true})}, [])
  // Touch menus leave the selection alone so a phone drawer stays open under the menu.
  function openMenu(node:TreeNode, x:number, y:number, select = true) {setFocused(node.key); if (select && node.token !== undefined && node.token !== selected) onSelect(node.assembly!, node.token); setMenu({x, y, node})}
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
      else if(row.node.target!==undefined) onLink(row.node.assembly!,row.node.target)
      else {choose(row.node);if(row.node.children.length) toggle(row.node.key)}
    } else if(event.key==='ContextMenu' || (event.shiftKey && event.key==='F10')) {
      const element = document.getElementById(nodeId(row.node.key))?.getBoundingClientRect()
      openMenu(row.node, (element?.left ?? 0) + 24, (element?.bottom ?? 0))
    } else if(event.key==='*') expandAll(row.node)
    else if(event.key==='Delete') contextActions(row.node,{expand:expandAll,collapse:collapseAll}).find(action=>action?.label==='Close Assembly')?.run()
    else return
    event.preventDefault()
    if(next!==index) choose(rows[next].node)
  }
  return <div ref={viewport} className="tree-scroll flex-1 overflow-auto outline-none [scrollbar-width:thin] [scrollbar-color:var(--border)_transparent] py-1 px-0" role="tree" aria-label="Assembly declarations" tabIndex={0} aria-activedescendant={visible.some(row=>row.node.key===focused) ? nodeId(focused!) : undefined} onKeyDown={keyboard} onScroll={e => setScroll(e.currentTarget.scrollTop)}>
    <div role="presentation" style={{height: rows.length * rowHeight, position: 'relative'}}>
      {visible.map(({node, depth}, index) => <div role="presentation" key={node.key} style={{position: 'absolute', top: (start + index) * rowHeight, height: rowHeight, width: '100%'}}>
        <button id={nodeId(node.key)} tabIndex={-1} role="treeitem" data-kind={node.kind} data-assembly-id={node.assembly} aria-label={node.name} aria-level={depth + 1} aria-expanded={node.children.length ? expanded.has(node.key) : undefined} aria-selected={node.token === selected && node.assembly === selectedAssembly} style={{paddingLeft: depth * 15 + 6, height: rowHeight}} onPointerDown={event => {if (event.pointerType !== 'touch') return; cancelPress(); const x = event.clientX, y = event.clientY; press.current = {x, y, fired: false, timer: setTimeout(() => {if (press.current) press.current.fired = true; openMenu(node, x, y, false)}, 550)}}}
          onPointerMove={event => {if (press.current && Math.hypot(event.clientX - press.current.x, event.clientY - press.current.y) > 10) cancelPress()}}
          onPointerUp={() => {if (press.current && !press.current.fired) cancelPress()}} onPointerCancel={cancelPress}
          className={cn('tree-row flex gap-1.5 items-center border-0 select-none [-webkit-touch-callout:none] bg-transparent text-left whitespace-nowrap w-full text-text pr-2 text-[12px] hover:bg-hover [&>svg]:shrink-0',node.token === selected && node.assembly === selectedAssembly && 'selected bg-accent text-accent-foreground',focused===node.key && '[[role=tree]:focus_&]:outline [[role=tree]:focus_&]:-outline-offset-1 [[role=tree]:focus_&]:outline-border')} title={node.reference ? `${node.name}, ${node.reference.identity.version} · ${node.reference.resolved_id !== null ? 'Resolved · double-click to open' : 'Unresolved · double-click to resolve'}` : node.target !== undefined ? `${node.name} · double-click to go to the type` : describe(node)}
          onContextMenu={event => {event.preventDefault(); if (!press.current) openMenu(node, event.clientX, event.clientY)}}
          onClick={() => {if (press.current?.fired) {press.current = null; return} choose(node); if(node.token===undefined && node.target===undefined) toggle(node.key)}}
          onDoubleClick={() => {if (node.reference) onReference(node.assembly!,node.token!); else if (node.target!==undefined) onLink(node.assembly!,node.target); else if (node.children.length && node.token!==undefined) toggle(node.key)}}>
          <span className={cn('disclosure w-2.5 h-[19px] flex items-center justify-center shrink-0 text-muted-foreground',expanded.has(node.key) && '[&_svg]:rotate-90')} onClick={e => {if (node.children.length) {e.stopPropagation();setFocused(node.key);viewport.current?.focus({preventScroll:true});toggle(node.key)}}}>{node.children.length > 0 && <ChevronRight size={12} />}</span>
          <KindIcon node={node} /><Label node={node} />{node.reference && <span className="ml-auto shrink-0 text-muted-foreground" aria-label={node.reference.resolved_id !== null ? 'Resolved reference' : 'Unresolved reference'}>{node.reference.resolved_id !== null ? <Link size={11}/> : <Unlink size={11}/>}</span>}
        </button>
      </div>)}
    </div>
    {menu && <ContextMenu x={menu.x} y={menu.y} label={`${menu.node.name} actions`} actions={contextActions(menu.node, {expand: expandAll, collapse: collapseAll})} onClose={closeMenu} />}
  </div>
}
