import type { Declaration, Overview } from './types'
export interface TreeNode { key: string; name: string; kind: string; token?: number; declaration?: Declaration; children: TreeNode[] }
export interface TreeRow { node: TreeNode; depth: number }
const order: Record<string, number> = {field: 0, property: 1, event: 2, constructor: 3, method: 4}
export function createTree(declarations: Declaration[], overview: Overview): TreeNode[] {
  const byToken = new Map<number, TreeNode>()
  const namespaces = new Map<string, TreeNode>()
  for (const d of declarations) byToken.set(d.token, {key: `t:${d.token}`, name: d.name, kind: d.kind, token: d.token, declaration: d, children: []})
  for (const d of declarations) {
    const node = byToken.get(d.token)!
    const parent = byToken.get(d.parent)
    if (parent) parent.children.push(node)
    else {
      const namespace = d.namespace || '<global namespace>'
      if (!namespaces.has(namespace)) namespaces.set(namespace, {key: `ns:${namespace}`, name: namespace, kind: 'namespace', children: []})
      namespaces.get(namespace)!.children.push(node)
    }
  }
  // Group members without adding every member to the mounted DOM.
  for (const node of byToken.values()) {
    const groups = new Map<string, TreeNode>()
    const nested: TreeNode[] = []
    for (const child of node.children) {
      if (child.kind in order) {
        const title = ({field: 'Fields', property: 'Properties', event: 'Events', constructor: 'Constructors', method: 'Methods'} as Record<string, string>)[child.kind]
        if (!groups.has(title)) groups.set(title, {key: `${node.key}:${title}`, name: title, kind: 'group', children: []})
        groups.get(title)!.children.push(child)
      } else nested.push(child)
    }
    node.children = [...[...groups.values()].sort((a, b) => (order[a.children[0].kind] ?? 0) - (order[b.children[0].kind] ?? 0)), ...nested]
  }
  return [
    {key: 'overview', name: overview.info.identity.name, kind: 'assembly', token: 0, children: []},
    {key: 'references', name: 'References', kind: 'references', token: -1, children: overview.references.map(r => ({key: `ref:${r.token}`, name: r.identity.name, token: r.token, kind: 'reference', children: []}))},
    {key: 'resources', name: 'Resources', kind: 'resources', token: -2, children: overview.resources.map(r => ({key: `res:${r.token}`, name: r.name, token: r.token, kind: 'resource', children: []}))},
    ...[...namespaces.values()].sort((a, b) => a.name.localeCompare(b.name)),
  ]
}
export function flattenTree(nodes: TreeNode[], expanded: Set<string>, depth = 0): TreeRow[] {
  const result: TreeRow[] = []
  for (const node of nodes) { result.push({node, depth}); if (expanded.has(node.key)) result.push(...flattenTree(node.children, expanded, depth + 1)) }
  return result
}
export function pathToToken(nodes: TreeNode[], token: number): string[] | null {
  for (const node of nodes) { if (node.token === token) return [node.key]; const path = pathToToken(node.children, token); if (path) return [node.key, ...path] }
  return null
}
