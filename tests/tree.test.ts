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

it('adds dnSpy base/derived type folders, a PE node and access information', async () => {
 const {createWorkspaceTree, accessOf, peTokens}=await import('../src/lib/tree')
 const declarations: Declaration[] = [
  {token: 0x02000002, parent: 0, kind: 'class', name: 'Base', namespace: 'N', full_name: 'N.Base', flags: 1, base: {token: 0x01000001, name: 'System.Object'}},
  {token: 0x02000003, parent: 0, kind: 'class', name: 'Derived', namespace: 'N', full_name: 'N.Derived', flags: 0, base: {token: 0x02000002, name: 'N.Base'}},
  {token: 0x04000001, parent: 0x02000003, kind: 'field', name: 'Limit', display_name: 'Limit : int', namespace: '', full_name: 'N.Derived.Limit', flags: 0x51},
  {token: 0x06000001, parent: 0x02000003, kind: 'method', name: 'get_Value', accessor_of: 0x17000001, namespace: '', full_name: 'N.Derived.get_Value', flags: 0x04},
  {token: 0x17000001, parent: 0x02000003, kind: 'property', name: 'Value', namespace: '', full_name: 'N.Derived.Value', flags: 0},
 ]
 const overview={info:{identity:{name:'Fixture'},module_name:'Fixture.dll'},references:[],resources:[]} as unknown as Overview
 const module=createWorkspaceTree([{id:1,declarations,overview}])[0].children[0]
 expect(module.children[0]).toMatchObject({name: 'PE', token: peTokens.pe})
 expect(module.children[0].children.map(n=>n.token)).toContain(peTokens.cor20)
 const types=module.children.find(n=>n.kind==='namespace')!.children
 const base=types.find(n=>n.name==='Base')!, derived=types.find(n=>n.name==='Derived')!
 expect(base.children.map(n=>n.name)).toEqual(['Base Type and Interfaces','Derived Types'])
 expect(base.children[1].children[0]).toMatchObject({name: 'N.Derived', target: 0x02000003}); expect(base.children[1].children[0].token).toBeUndefined()
 expect(derived.children[0].children[0]).toMatchObject({name: 'N.Base', target: 0x02000002})
 // Links do not own tokens, so revealing a type still finds the declaration itself.
 expect(pathToToken([module],0x02000003,1)?.at(-1)).toBe('a:1:t:33554435')
 expect(accessOf(base)).toEqual({access:'public',isStatic:false,constant:false})
 expect(accessOf(derived.children.find(n=>n.name==='Limit : int')!)).toEqual({access:'private',isStatic:true,constant:true})
 expect(accessOf(derived.children.find(n=>n.kind==='property')!)?.access).toBe('protected')
})
