import type { Declaration, Overview, Reference } from './types'
// `target` marks navigation links (base/derived types) that point at another declaration
// without owning its token, so selection and reveal still find the real declaration.
export interface TreeNode { key: string; name: string; kind: string; token?: number; target?: number; assembly?: number; reference?: Reference; declaration?: Declaration; children: TreeNode[] }
// Synthetic selection tokens for PE header views; real metadata tokens are positive.
export const peTokens = {pe: -3, dos: -10, file: -11, optional: -12, directories: -13, sections: -14, cor20: -15, metadata: -16, tables: -17, protection: -18} as const
export const peGroups: {id: Exclude<keyof typeof peTokens, 'pe'>; name: string}[] = [
  {id: 'protection', name: 'Protection Scan'}, {id: 'dos', name: 'DOS Header'}, {id: 'file', name: 'File Header'}, {id: 'optional', name: 'Optional Header'}, {id: 'directories', name: 'Data Directories'},
  {id: 'sections', name: 'Section Headers'}, {id: 'cor20', name: 'Cor20 Header'}, {id: 'metadata', name: 'Storage Stream Headers'}, {id: 'tables', name: 'Tables Stream'},
]
export const isPeToken = (token: number | null) => token !== null && token <= peTokens.pe && token >= peTokens.protection
/** Explorer nodes for metadata tables use tokens -100 - table id. */
export const tableToken = (table: number) => -100 - table
export const isTableToken = (token: number | null) => token !== null && token <= -100 && token >= -144
export type Access = 'public' | 'internal' | 'protected' | 'protected internal' | 'private protected' | 'private'
const typeAccess: Access[] = ['internal', 'public', 'public', 'private', 'protected', 'internal', 'private protected', 'protected internal']
const memberAccess: Access[] = ['private', 'private', 'private protected', 'internal', 'protected', 'protected internal', 'public', 'private']
const rank: Access[] = ['private', 'private protected', 'internal', 'protected', 'protected internal', 'public']
// ECMA-335 visibility masks; properties and events take their widest accessor's access.
export function accessOf(node: TreeNode): {access: Access; isStatic: boolean; constant: boolean} | null {
  const d = node.declaration
  if (!d) return null
  if (d.token >>> 24 === 2) return {access: typeAccess[d.flags & 7], isStatic: (d.flags & 0x180) === 0x180 && node.kind === 'class', constant: false}
  if (d.token >>> 24 === 4 || d.token >>> 24 === 6) return {access: memberAccess[d.flags & 7], isStatic: (d.flags & 0x10) !== 0, constant: d.token >>> 24 === 4 && (d.flags & 0x40) !== 0}
  const accessors = node.children.map(accessOf).filter(a => a !== null)
  if (!accessors.length) return null
  return {access: accessors.reduce((best, a) => rank.indexOf(a.access) > rank.indexOf(best) ? a.access : best, accessors[0].access), isStatic: accessors.some(a => a.isStatic), constant: false}
}
export interface TreeRow { node: TreeNode; depth: number }
const order: Record<string, number> = {constructor: 0, method: 1, property: 2, event: 3, field: 4}
const compare = (a: TreeNode, b: TreeNode) => (order[a.kind] ?? 5) - (order[b.kind] ?? 5) || a.name.localeCompare(b.name, 'en', {numeric:true}) || (a.token ?? 0) - (b.token ?? 0)
/** Compiler-generated names start with `<` (closures, state machines, backing fields). */
const generated = (d: Declaration) => d.name.startsWith('<') && d.name !== '<Module>'
export function createTree(declarations: Declaration[], overview: Overview, options: {hideGenerated?: boolean} = {}): TreeNode[] {
  if (options.hideGenerated) {
    const hidden = new Set<number>()
    for (const d of declarations) if (generated(d)) hidden.add(d.token)
    // Members of hidden types go with them.
    let changed = true
    while (changed) {changed = false; for (const d of declarations) if (!hidden.has(d.token) && hidden.has(d.parent)) {hidden.add(d.token); changed = true}}
    declarations = declarations.filter(d => !hidden.has(d.token))
  }
  const byToken = new Map<number, TreeNode>()
  const namespaces = new Map<string, TreeNode>()
  for (const d of declarations) byToken.set(d.token, {key: `t:${d.token}`, name: d.display_name || d.name, kind: d.kind, token: d.token, declaration: d, children: []})
  for (const d of declarations) {
    const node = byToken.get(d.token)!
    const accessor = d.accessor_of ? byToken.get(d.accessor_of) : undefined
    const parent = accessor && accessor.declaration?.parent === d.parent && ['property','event'].includes(accessor.kind) ? accessor : byToken.get(d.parent)
    if (parent) parent.children.push(node)
    else {
      const namespace = d.namespace || '-'
      if (!namespaces.has(namespace)) namespaces.set(namespace, {key: `ns:${namespace}`, name: namespace, kind: 'namespace', children: []})
      namespaces.get(namespace)!.children.push(node)
    }
  }
  // Direct members, with property/event accessors underneath their declaration.
  // Keep real <Module> members, but omit the compiler's empty placeholder type.
  for (const node of byToken.values()) node.children.sort(compare)
  // dnSpy-style Base Type and Derived Types folders; derived types are limited to this assembly.
  const derived = new Map<number, Declaration[]>()
  for (const d of declarations) for (const link of [...(d.base ? [d.base] : []), ...(d.interfaces ?? [])]) if (link.token >>> 24 === 2) {const list = derived.get(link.token) ?? []; list.push(d); derived.set(link.token, list)}
  for (const node of byToken.values()) {
    const d = node.declaration!
    if (d.token >>> 24 !== 2) continue
    const folders: TreeNode[] = []
    const bases = [...(d.base ? [d.base] : []), ...(d.interfaces ?? [])]
    if (bases.length) folders.push({key: `base:${d.token}`, name: 'Base Type and Interfaces', kind: 'base-types', children: bases.map((link, i) => ({key: `base:${d.token}:${i}`, name: link.name, kind: 'base-type', target: link.token, children: []}))})
    const subtypes = derived.get(d.token)
    if (subtypes?.length) folders.push({key: `derived:${d.token}`, name: 'Derived Types', kind: 'derived-types', children: subtypes.map((sub, i) => ({key: `derived:${d.token}:${i}`, name: sub.full_name, kind: 'derived-type', target: sub.token, children: []})).sort(compare)})
    node.children.unshift(...folders)
  }
  for (const namespace of namespaces.values()) namespace.children = namespace.children.filter(n => n.declaration?.name !== '<Module>' || n.children.some(c => c.declaration)).sort(compare)
  return [
    {key: 'overview', name: overview.info.identity.name, kind: 'assembly', token: 0, children: []},
    {key: 'pe', name: 'PE', kind: 'pe', token: peTokens.pe, children: peGroups.map(g => ({key: `pe:${g.id}`, name: g.name, kind: g.id === 'protection' ? 'protection' : 'pe-header', token: peTokens[g.id], children: g.id === 'tables' ? (overview.tables ?? []).filter(t => t.rows > 0).map(t => ({key: `table:${t.id}`, name: `${t.name} (${t.rows})`, kind: 'metadata-table', token: tableToken(t.id), children: []})) : []}))},
    {key: 'references', name: 'References', kind: 'references', token: -1, children: overview.references.map(r => ({key: `ref:${r.token}`, name: r.identity.name, token: r.token, reference:r, kind: 'reference', children: []})).sort(compare)},
    {key: 'resources', name: 'Resources', kind: 'resources', token: -2, children: overview.resources.map(r => ({key: `res:${r.token}`, name: r.name, token: r.token, kind: 'resource', children: []})).sort(compare)},
    ...[...namespaces.values()].filter(n => n.children.length).sort(compare),
  ]
}
export function flattenTree(nodes: TreeNode[], expanded: Set<string>, depth = 0): TreeRow[] {
  const result: TreeRow[] = []
  for (const node of nodes) { result.push({node, depth}); if (expanded.has(node.key)) result.push(...flattenTree(node.children, expanded, depth + 1)) }
  return result
}
export function pathToToken(nodes: TreeNode[], token: number, assembly?: number): string[] | null {
  for (const node of nodes) { if (node.token === token && (assembly === undefined || node.assembly === assembly)) return [node.key]; const path = pathToToken(node.children, token, assembly); if (path) return [node.key, ...path] }
  return null
}
export function createWorkspaceTree(assemblies: {id:number; declarations:Declaration[]; overview:Overview}[], options: {hideGenerated?: boolean} = {}): TreeNode[] {
  const names = new Map<string,number>()
  for (const a of assemblies) names.set(a.overview.info.identity.name,(names.get(a.overview.info.identity.name) ?? 0) + 1)
  return assemblies.map(a => {
    const [root,...children] = createTree(a.declarations,a.overview,options)
    const qualify=(node:TreeNode):TreeNode=>({...node,key:`a:${a.id}:${node.key}`,assembly:a.id,children:node.children.map(qualify)})
    const module:TreeNode={key:'module',name:a.overview.info.module_name || `${root.name}.dll`,kind:'module',token:1,children}
    return qualify({...root,name:root.name+((names.get(root.name) ?? 0) > 1 ? ` (${a.overview.info.identity.version})` : ''),children:[module]})
  })
}
