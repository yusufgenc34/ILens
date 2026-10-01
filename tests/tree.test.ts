import {it, expect} from 'vitest'
import {createTree, flattenTree, pathToToken} from '../src/lib/tree'
import type {Declaration, Overview} from '../src/lib/types'
it('keeps methods navigable through namespace and type without member category folders', () => {
 const declarations: Declaration[] = [{token: 0x02000001, parent: 0, kind: 'class', name: 'Example', namespace: 'Fixture', full_name: 'Fixture.Example', flags: 1}, {token: 0x06000001, parent: 0x02000001, kind: 'method', name: 'Compute', namespace: '', full_name: 'Fixture.Example.Compute', flags: 6}]
 const overview = {info: {identity: {name: 'Fixture'}}, references: [], resources: []} as unknown as Overview
 const tree = createTree(declarations, overview); const path = pathToToken(tree, 0x06000001)!
 expect(path).toEqual(['ns:Fixture', 't:33554433', 't:100663297'])
 expect(flattenTree(tree, new Set()).some(r => r.node.name === 'Compute')).toBe(false)
 expect(flattenTree(tree, new Set(path)).some(r => r.node.name === 'Compute')).toBe(true)
})

it('puts modules, direct members, nested types and real property accessors in a stable tree', async () => {
 const {createWorkspaceTree}=await import('../src/lib/tree')
 const d=(token:number,parent:number,kind:string,name:string,extra={})=>({token,parent,kind,name,namespace:parent ? '' : 'Fixture',full_name:name,flags:0,...extra})
 const declarations=[d(2,0,'class','Example'),d(6,2,'method','Z', {display_name:'Z(int) : void'}),d(7,2,'method','Z',{display_name:'Z(string) : void'}),d(8,2,'method','get_Name',{accessor_of:23}),d(23,2,'property','Name'),d(3,2,'class','Nested')]
 const overview={info:{identity:{name:'Fixture'},module_name:'Fixture.exe'},references:[],resources:[]} as unknown as Overview
 const tree=createWorkspaceTree([{id:1,declarations,overview}])
 expect(tree[0].children[0].name).toBe('Fixture.exe')
 const path=pathToToken(tree,8,1)!
 expect(path).toEqual(['a:1:overview','a:1:module','a:1:ns:Fixture','a:1:t:2','a:1:t:23','a:1:t:8'])
 const members=tree[0].children[0].children.find(n=>n.kind==='namespace')!.children[0].children
 expect(members.map(n=>n.name)).toEqual(['Z(int) : void','Z(string) : void','Name','Nested'])
 expect(flattenTree(tree,new Set(path)).some(r=>r.node.kind==='group')).toBe(false)
})
