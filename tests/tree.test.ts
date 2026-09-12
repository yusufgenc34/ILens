import {it, expect} from 'vitest'
import {createTree, flattenTree, pathToToken} from '../src/lib/tree'
import type {Declaration, Overview} from '../src/lib/types'
it('keeps methods navigable through namespace, type, and category groups', () => {
 const declarations: Declaration[] = [{token: 0x02000001, parent: 0, kind: 'class', name: 'Example', namespace: 'Fixture', full_name: 'Fixture.Example', flags: 1}, {token: 0x06000001, parent: 0x02000001, kind: 'method', name: 'Compute', namespace: '', full_name: 'Fixture.Example.Compute', flags: 6}]
 const overview = {info: {identity: {name: 'Fixture'}}, references: [], resources: []} as unknown as Overview
 const tree = createTree(declarations, overview); const path = pathToToken(tree, 0x06000001)!
 expect(path).toEqual(['ns:Fixture', 't:33554433', 't:33554433:Methods', 't:100663297'])
 expect(flattenTree(tree, new Set()).some(r => r.node.name === 'Compute')).toBe(false)
 expect(flattenTree(tree, new Set(path)).some(r => r.node.name === 'Compute')).toBe(true)
})
