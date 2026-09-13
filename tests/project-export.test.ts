import {describe, expect, it} from 'vitest'
import {assemblyInfoFile, createProjectPaths, projectGuid, projectName, projectSettings, rootNamespace, sdkProjectFile, solutionFile, typePath, type ProjectDescriptor} from '../src/lib/project-export'
import type {ExportType} from '../src/lib/export-types'
import type {Overview} from '../src/lib/types'
import {inferFramework} from '../src/lib/export'
const type = {token: 0x02000002, parent: 0, name: 'Main', namespace: 'Example.App', kind: 'class', members: [0x06000001], declaration: 'public class Main', diagnostics: []} satisfies ExportType
const overview = {info: {identity: {name: 'Example.App', version: '1.2.3.4', culture: '', public_key_token: ''}}, pe: {entry_point: 0x06000001, subsystem: 3, machine: 0x14c, cli_flags: 1}, streams: [], tables: [], references: [], resources: []} as unknown as Overview
const descriptor: ProjectDescriptor = {id: 1, name: 'Example.App', directory: 'Example.App', file: 'Example.App.csproj', guid: projectGuid(0), framework: 'net10.0', overview}
describe('project export layout and build metadata', () => {
  it('uses conventional namespace/type paths and changes only colliding names', () => {
    const allocate = createProjectPaths()
    expect(rootNamespace([type, {...type, namespace: 'Example.App.Models'}])).toBe('Example.App')
    expect(typePath(type, 'Example.App', allocate)).toBe('Main.cs')
    expect(typePath({...type, namespace: 'Example.App.Models', name: 'Item'}, 'Example.App', allocate)).toBe('Models/Item.cs')
    expect(typePath({...type, name: 'main'}, 'Example.App', allocate)).toBe('main.2.cs')
    expect(projectName('ILens.Dependency')).toBe('ILens.Dependency')
    for (const raw of ['../../CON.txt', 'x\\y:$(Attack)', '\n"inject', '..']) expect(projectName(raw)).toMatch(/^[a-zA-Z0-9_][a-zA-Z0-9_.-]*$/)
    expect(projectName('CON.txt')).toBe('_CON.txt')
    expect(inferFramework('.NETFramework,Version=v4.0')).toBe('net40')
    expect(inferFramework('.NETCoreApp,Version=v3.1')).toBe('netcoreapp3.1')
    expect(inferFramework('.NETStandard,Version=v1.6')).toBe('netstandard1.6')
    expect(inferFramework('.NETFramework,Version=v4.0,Profile=Client')).toBe('')
  })
  it('preserves executable output, platform, assembly version, and identity-matched project references', () => {
    const xml = sdkProjectFile(descriptor, [type], ['Main.cs'], [{identity: {...overview.info.identity, name: 'Dependency'}, project: '../Dependency/Dependency.csproj'}], [{name: 'Example.App.message.txt', path: 'Resources/Example.App.message.txt'}])
    expect(xml).toContain('<OutputType>Exe</OutputType>'); expect(xml).toContain('<StartupObject>Example.App.Main</StartupObject>'); expect(xml).toContain('<PlatformTarget>AnyCPU</PlatformTarget>')
    expect(xml).toContain('<ProjectReference Include="../Dependency/Dependency.csproj" />')
    expect(xml).toContain('<LogicalName>Example.App.message.txt</LogicalName>')
    expect(assemblyInfoFile(overview.info.identity)).toContain('AssemblyVersion("1.2.3.4")')
    expect(projectSettings({...overview, pe: {...overview.pe, machine: 0x8664, subsystem: 2}}, [type])).toMatchObject({output: 'WinExe', platform: 'x64'})
    expect(projectSettings({...overview, pe: {...overview.pe, entry_point: 0}}, [type]).output).toBe('Library')
  })
  it('creates solution build configurations without user paths or random machine IDs', () => {
    const text = solutionFile([descriptor, {...descriptor, id: 2, name: 'Dependency', directory: 'Dependency', file: 'Dependency.csproj', guid: projectGuid(1)}])
    expect(text).toContain('"Example.App\\Example.App.csproj"')
    expect(text).toContain(`${projectGuid(1)}.Release|Any CPU.Build.0 = Release|Any CPU`)
    expect(text).not.toMatch(/\/Users\/|C:\\|\/home\//)
    const hostile = sdkProjectFile({...descriptor, overview: {...overview, info: {...overview.info, identity: {...overview.info.identity, name: '../../$(Attack)'}}}}, [], [], [], [])
    expect(hostile).not.toContain('$(Attack)'); expect(hostile).not.toContain('<AssemblyName>../')
  })
})
