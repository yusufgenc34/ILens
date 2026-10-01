import {describe, expect, it, vi} from 'vitest'
import {emptyNavigation, visit, travel, toggleBookmark} from '../src/lib/navigation'
import {indexDependencyFolder, resolveDependencyFolder} from '../src/lib/dependency-folder'
import type {DecompilerClient} from '../src/lib/rpc'
import type {Identity, Loaded} from '../src/lib/types'

const location = (token: number) => ({assembly:1, token, view:'C#' as const})
const identity = (name: string, version = '1.0.0.0'): Identity => ({name, version, culture:'neutral', public_key_token:''})
const loaded = (id: number, name: string) => ({id, info:{identity:identity(name)}} as Loaded)
const reference = (name: string) => ({identity:identity(name), resolved_id:null})
describe('workspace navigation', () => {
  it('preserves per-document views, walks history and forks only on new navigation', () => {
    let state = visit(visit(emptyNavigation, location(1)), location(2))
    state = visit(state, {...location(2), view:'IL'})
    expect(state.history).toHaveLength(2)
    state = travel(state,-1)
    expect(state.history[state.index].token).toBe(1)
    expect(travel(state,1).history[1].view).toBe('IL')
    state = visit(state, location(3))
    expect(state.history.map(item => item.token)).toEqual([1,3])
    expect(state.tabs.map(item => item.token)).toEqual([1,2,3])
  })
  it('bounds tabs/history/bookmarks and distinguishes the same token in another assembly', () => {
    let state = emptyNavigation
    for (let token = 0; token < 200; token++) {state = visit(state, location(token)); state = toggleBookmark(state, location(token))}
    expect(state.tabs).toHaveLength(32); expect(state.history).toHaveLength(128); expect(state.bookmarks).toHaveLength(64)
    state = toggleBookmark(state, location(199)); expect(state.bookmarks).toHaveLength(63)
    state = visit(state, {...location(199), assembly:2})
    expect(state.tabs.at(-2)?.assembly).toBe(1); expect(state.tabs.at(-1)?.assembly).toBe(2)
    expect(travel(emptyNavigation,-1)).toEqual(emptyNavigation)
  })
})
describe('selected dependency folder', () => {
  it('resolves transitive references, skipping wrong identities and unrelated files', async () => {
    const files = [new File(['wrong'],'B.dll'),new File(['right'],'b.DLL'),new File(['next'],'C.exe'),new File(['unused'],'Unrelated.dll')]
    const folder = indexDependencyFolder([...files,new File(['text'],'README.md')])
    const call = vi.fn(async (operation, input, options) => {
      if (operation === 'getReferences') return input.assembly === 1 ? [reference('B'),reference('Missing')] : input.assembly === 2 ? [reference('C'),reference('B')] : []
      expect(options.transfer).toEqual([input.buffer]); expect(options.signal).toBeInstanceOf(AbortSignal)
      const content = new TextDecoder().decode(input.buffer)
      if (content === 'wrong') return null
      return content === 'right' ? loaded(2,'B') : loaded(3,'C')
    })
    const retained = vi.fn(async () => {})
    const result = await resolveDependencyFolder({call} as unknown as DecompilerClient,[loaded(1,'A')],folder,new AbortController().signal,retained,vi.fn())
    expect(result).toEqual({loaded:2,unavailable:1,rejected:0})
    expect(retained.mock.calls).toHaveLength(2)
    expect(call.mock.calls.filter(args => args[0] === 'loadDependency')).toHaveLength(3)
    expect(folder.files).toBe(4)
  })
  it('cancellation stops file reads and worker calls', async () => {
    const controller = new AbortController(); controller.abort()
    const call = vi.fn(), file = new File(['data'],'B.dll'), read = vi.spyOn(file,'arrayBuffer')
    await expect(resolveDependencyFolder({call} as unknown as DecompilerClient,[loaded(1,'A')],indexDependencyFolder([file]),controller.signal,vi.fn(),vi.fn())).rejects.toMatchObject({code:'cancelled'})
    expect(call).not.toHaveBeenCalled(); expect(read).not.toHaveBeenCalled()
  })
  it('rejects oversized indexes without reading file contents', () => {
    const file = new File(['x'],'A.dll')
    expect(() => indexDependencyFolder(Array(4097).fill(file))).toThrow('4,096')
    expect(() => indexDependencyFolder(Array(33).fill(file))).toThrow('Too many versions')
  })
})
