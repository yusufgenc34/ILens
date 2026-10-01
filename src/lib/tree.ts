import type { Declaration, Overview, Reference } from './types'
export interface TreeNode { key: string; name: string; kind: string; token?: number; assembly?: number; reference?: Reference; declaration?: Declaration; children: TreeNode[] }
export interface TreeRow { node: TreeNode; depth: number }
const order: Record<string, number> = {constructor: 0, method: 1, property: 2, event: 3, field: 4}
const compare = (a: TreeNode, b: TreeNode) => (order[a.kind] ?? 5) - (order[b.kind] ?? 5) || a.name.localeCompare(b.name, 'en', {numeric:true}) || (a.token ?? 0) - (b.token ?? 0)
export function createTree(declarations: Declaration[], overview: Overview): TreeNode[] {
  const byToken = new Map<number, TreeNode>()
  const namespaces = new Map<string, TreeNode>()
  for (const d of declarations) byToken.set(d.token, {key: `t:${d.token}`, name: d.display_name || d.name, kind: d.kind, token: d.token, declaration: d, children: []})
  for (const d of declarations) {
    const node = byToken.get(d.token)!
    const accessor = d.accessor_of ? byToken.get(d.accessor_of) : undefined
    const parent = accessor && accessor.declaration?.parent === d.parent && ['property','event'].includes(accessor.kind) ? accessor : byToken.get(d.parent)
    if (parent) parent.children.push(node)
    else {
      const namespace = d.namespace || '<global namespace>'
      if (!namespaces.has(namespace)) namespaces.set(namespace, {key: `ns:${namespace}`, name: namespace, kind: 'namespace', children: []})
      namespaces.get(namespace)!.children.push(node)
    }
  }
  // Direct members, with property/event accessors underneath their declaration.
  // Keep real <Module> members, but omit the compiler's empty placeholder type.
  for (const node of byToken.values()) node.children.sort(compare)
  for (const namespace of namespaces.values()) namespace.children = namespace.children.filter(n => n.declaration?.name !== '<Module>' || n.children.length > 0).sort(compare)
  return [
    {key: 'overview', name: overview.info.identity.name, kind: 'assembly', token: 0, children: []},
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
export function createWorkspaceTree(assemblies: {id:number; declarations:Declaration[]; overview:Overview}[]): TreeNode[] {
  const names = new Map<string,number>()
  for (const a of assemblies) names.set(a.overview.info.identity.name,(names.get(a.overview.info.identity.name) ?? 0) + 1)
  return assemblies.map(a => {
    const [root,...children] = createTree(a.declarations,a.overview)
    const qualify=(node:TreeNode):TreeNode=>({...node,key:`a:${a.id}:${node.key}`,assembly:a.id,children:node.children.map(qualify)})
    const module:TreeNode={key:'module',name:a.overview.info.module_name || `${root.name}.dll`,kind:'module',token:1,children}
    return qualify({...root,name:root.name+((names.get(root.name) ?? 0) > 1 ? ` (${a.overview.info.identity.version})` : ''),children:[module]})
  })
}
