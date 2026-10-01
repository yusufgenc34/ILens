import {describe,it,expect,vi} from 'vitest'
import {resolveAutomaticDependencies} from '../src/lib/automatic-dependencies'
import {indexDependencyFolder} from '../src/lib/dependency-folder'
import {diagnostic,type AssemblyInfo,type Identity,type Loaded} from '../src/lib/types'
import type {DecompilerClient} from '../src/lib/rpc'
const identity=(name:string):Identity=>({name,version:'10.0.0.0',culture:'neutral',public_key_token:'b03f5f7f11d50a3a'})
const info=(name:string)=>({identity:identity(name),target_framework:{status:'declared',moniker:'.NETCoreApp,Version=v10.0'}} as AssemblyInfo)
const loaded=(id:number,name:string):Loaded=>({id,info:info(name)})
const refs=(...names:string[])=>names.map((name,i)=>({token:0x23000001+i,identity:identity(name),resolved_id:null}))
const core=(call:ReturnType<typeof vi.fn>)=>({call} as unknown as DecompilerClient)
describe('automatic workspace dependencies',()=>{
  it('walks supported transitive references, inherits framework context and skips private names',async()=>{
    const retained=vi.fn(async()=>{}),seen=new Map<string,number>()
    const call=vi.fn(async(op,input)=>{
      const names=input.assembly===1 ? ['System.Collections','Company.Private','System.Collections'] : input.assembly===2 ? ['System.Runtime','System.Collections'] : []
      if(op==='getReferences') return refs(...names).map(r=>({...r,resolved_id:seen.get(r.identity.name) ?? null}))
      expect(op).toBe('resolveFrameworkReference')
      const name=names[input.token-0x23000001],id=seen.size+2;seen.set(name,id)
      return {id,info:{...info(name),target_framework:{status:'not_declared',moniker:null}}}
    })
    const result=await resolveAutomaticDependencies(core(call),[loaded(1,'App')],null,new AbortController().signal,()=>true,retained,vi.fn())
    expect(result.downloaded).toBe(2);expect(result.failures).toEqual([])
    expect(retained.mock.calls).toHaveLength(2)
    expect(call.mock.calls.filter(c=>c[0]==='resolveFrameworkReference')).toHaveLength(2)
  })
  it('checks the selected folder before any download, also when online resolution is disabled',async()=>{
    const retained=vi.fn(async()=>{});let resolved=false
    const call=vi.fn(async(op,input)=>{
      if(op==='getReferences') return input.assembly===1 ? refs('System.Runtime').map(r=>({...r,resolved_id:resolved ? 2 : null})) : []
      expect(op).toBe('loadDependency');resolved=true;return loaded(2,'System.Runtime')
    })
    const result=await resolveAutomaticDependencies(core(call),[loaded(1,'App')],indexDependencyFolder([new File(['owned test bytes'],'System.Runtime.dll')]),new AbortController().signal,()=>false,retained,vi.fn())
    expect(result.downloaded).toBe(0);expect(retained).toHaveBeenCalledTimes(1)
    expect(call.mock.calls.some(c=>c[0]==='resolveFrameworkReference')).toBe(false)
  })
  it('does not repeat failed package downloads for each reference and retains the error across postMessage',async()=>{
    const error=Object.assign(new Error('NuGet returned HTTP 503'),{code:'dependency_download',detail:'HTTP 503'})
    const serialized=structuredClone(diagnostic(error))
    expect(serialized.code).toBe('dependency_download')
    const call=vi.fn(async(op)=>{if(op==='getReferences') return refs('System.Runtime','System.Collections','System.Console');throw serialized})
    const result=await resolveAutomaticDependencies(core(call),[loaded(1,'App')],null,new AbortController().signal,()=>true,vi.fn(),vi.fn())
    expect(result.failures).toEqual([serialized]);expect(call).toHaveBeenCalledTimes(2)
  })
  it('continues after one missing entry and stops cleanly when downloads are disabled mid-pass',async()=>{
    let enabled=true
    const call=vi.fn(async(op,input)=>{
      if(op==='getReferences') return refs('System.Missing','System.Runtime','System.Collections')
      if(input.token===0x23000001) throw new Error('Package entry unavailable')
      enabled=false;return loaded(2,'System.Runtime')
    })
    const result=await resolveAutomaticDependencies(core(call),[loaded(1,'App')],null,new AbortController().signal,()=>enabled,vi.fn(async()=>{}),vi.fn())
    expect(result.failures).toHaveLength(1);expect(result.downloaded).toBe(1)
    expect(call.mock.calls.filter(c=>c[0]==='resolveFrameworkReference')).toHaveLength(2)
  })
  it('does not continue after workspace cancellation',async()=>{
    const controller=new AbortController()
    const call=vi.fn(async(op)=>op==='getReferences' ? refs('System.Runtime','System.Console') : loaded(2,'System.Runtime'))
    await expect(resolveAutomaticDependencies(core(call),[loaded(1,'App')],null,controller.signal,()=>true,vi.fn(async()=>{controller.abort()}),vi.fn())).rejects.toThrow()
    expect(call.mock.calls.filter(c=>c[0]==='resolveFrameworkReference')).toHaveLength(1)
  })
  it('bounds large reference graphs at 64 automatic resolution attempts',async()=>{
    const call=vi.fn(async(op,input)=>op==='getReferences' ? (input.assembly===1 ? refs(...Array.from({length:65},(_,i)=>`System.Test${i}`)) : []) : loaded(input.token,`System.Test${input.token}`))
    const result=await resolveAutomaticDependencies(core(call),[loaded(1,'App')],null,new AbortController().signal,()=>true,vi.fn(async()=>{}),vi.fn())
    expect(result.limited).toBe(true);expect(result.downloaded).toBe(64)
  })
})
