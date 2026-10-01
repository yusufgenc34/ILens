import {expect,it} from 'vitest'
import {companionAssembly} from '../src/lib/companion-assembly'
import {indexDependencyFolder} from '../src/lib/dependency-folder'
it('uses explicitly provided matching DLLs before the folder and refuses ambiguous versions',()=>{
 const exe=new File(['native'],'App.exe'),dll=new File(['managed'],'APP.dll'),other=new File(['other'],'App.dll')
 expect(companionAssembly(exe,[exe,dll],indexDependencyFolder([other]))).toBe(dll)
 expect(companionAssembly(exe,[],indexDependencyFolder([dll]))).toBe(dll)
 expect(()=>companionAssembly(exe,[],indexDependencyFolder([dll,other]))).toThrow()
 expect(companionAssembly(exe,[],null)).toBeNull()
 expect(companionAssembly(dll,[other],null)).toBeNull()
})
