import type {Identity, Overview} from './types'
import type {ExportType} from './export-types'
import {frameworkChoices, sourceNotice, xml} from './export'

// Preserve familiar filenames. Normalize only unsafe components; disambiguate
// collisions, rather than appending metadata tokens to every source filename.
export function projectName(name: string): string {
  const value = name.normalize('NFKC').replace(/[^a-zA-Z0-9_.-]+/g, '_').replace(/^[._-]+|[. ]+$/g, '').slice(0, 80).replace(/[. ]+$/g, '') || 'Assembly'
  return /^(con|prn|aux|nul|com[0-9]|lpt[0-9])(?:\.|$)/i.test(value) ? `_${value}` : value
}
export function createProjectPaths() {
  const used = new Set<string>()
  return (parts: string[], extension = '') => {
    if (!/^(?:\.[a-zA-Z0-9]+)?$/.test(extension) || parts.length > 48) throw new Error('Invalid project path.')
    const base = parts.map(projectName).join('/')
    let path = base + extension; let suffix = 2
    while (used.has(path.toLowerCase())) path = `${base}.${suffix++}${extension}`
    used.add(path.toLowerCase()); return path
  }
}
export function rootNamespace(types: ExportType[]): string {
  const namespaces = types.filter(t => t.parent === 0 && t.kind !== 'module').map(t => t.namespace.split('.'))
  const result = namespaces[0]?.slice() ?? []
  for (const ns of namespaces) {let i = 0; while (i < result.length && result[i] === ns[i]) i++; result.length = i}
  return result.join('.')
}
export function typePath(type: ExportType, root: string, allocate: ReturnType<typeof createProjectPaths>): string {
  const ns = type.namespace === root ? '' : root && type.namespace.startsWith(root + '.') ? type.namespace.slice(root.length + 1) : type.namespace
  return allocate([...(ns ? ns.split('.') : []), type.name], '.cs')
}
export interface ProjectDescriptor {id: number; name: string; directory: string; file: string; guid: string; framework: string; overview: Overview}
// Solution identifiers are deterministic counters within this export, never
// host/user identifiers or assembly-controlled strings.
export function projectGuid(index: number): string {return `{494C454E-5300-4000-8000-${(index + 1).toString(16).padStart(12, '0').toUpperCase()}}`}
export function solutionFile(projects: ProjectDescriptor[]): string {
  const lines = ['Microsoft Visual Studio Solution File, Format Version 12.00', '# Visual Studio Version 17', 'VisualStudioVersion = 17.0.31903.59', 'MinimumVisualStudioVersion = 10.0.40219.1']
  for (const p of projects) lines.push(`Project("{FAE04EC0-301F-11D3-BF4B-00C04F79EFBC}") = "${p.directory}", "${p.directory}\\${p.file}", "${p.guid}"`, 'EndProject')
  lines.push('Global', '\tGlobalSection(SolutionConfigurationPlatforms) = preSolution', '\t\tDebug|Any CPU = Debug|Any CPU', '\t\tRelease|Any CPU = Release|Any CPU', '\tEndGlobalSection', '\tGlobalSection(ProjectConfigurationPlatforms) = postSolution')
  for (const p of projects) for (const c of ['Debug', 'Release']) lines.push(`\t\t${p.guid}.${c}|Any CPU.ActiveCfg = ${c}|Any CPU`, `\t\t${p.guid}.${c}|Any CPU.Build.0 = ${c}|Any CPU`)
  return lines.concat('\tEndGlobalSection', 'EndGlobal', '').join('\r\n')
}
const csharpString = (value: string) => JSON.stringify(value).replace(/[\u2028\u2029]/g, c => `\\u${c.charCodeAt(0).toString(16)}`)
export function assemblyInfoFile(identity: Identity): string {
  if (!/^\d+\.\d+\.\d+\.\d+$/.test(identity.version)) throw new Error('Invalid assembly version in project export.')
  return `${sourceNotice}// Identity attributes recovered from metadata. Other attributes are listed in the export report.\n[assembly: System.Reflection.AssemblyVersion(${csharpString(identity.version)})]\n${identity.culture && identity.culture !== 'neutral' ? `[assembly: System.Reflection.AssemblyCulture(${csharpString(identity.culture)})]\n` : ''}`
}
export function projectSettings(overview: Overview, types: ExportType[]) {
  const pe = overview.pe
  const entry = typeof pe.entry_point === 'number' ? pe.entry_point : 0
  const type = types.find(t => t.members.includes(entry))
  const validName = type && type.kind !== 'module' && type.parent === 0 && [...type.namespace.split('.').filter(Boolean), type.name].every(n => /^[a-zA-Z_][a-zA-Z0-9_]*$/.test(n))
  return {
    output: entry ? pe.subsystem === 2 ? 'WinExe' : 'Exe' : 'Library',
    platform: pe.machine === 0x8664 ? 'x64' : pe.machine === 0xaa64 ? 'ARM64' : Number(pe.cli_flags) & 2 ? 'x86' : 'AnyCPU',
    startup: entry && validName ? [type.namespace, type.name].filter(Boolean).join('.') : '',
    entry,
  }
}
export interface ProjectReference {identity: Identity; path?: string; project?: string}
export function sdkProjectFile(p: ProjectDescriptor, types: ExportType[], paths: string[], references: ProjectReference[], resources: {name: string; path: string}[]): string {
  if (!frameworkChoices.includes(p.framework)) throw new Error('Select a target framework for every exported assembly.')
  const settings = projectSettings(p.overview, types)
  const identity = p.overview.info.identity
  const properties = {TargetFramework: p.framework, OutputType: settings.output, AssemblyName: projectName(identity.name), RootNamespace: rootNamespace(types), PlatformTarget: settings.platform, ...(settings.startup ? {StartupObject: settings.startup} : {}), EnableDefaultItems: 'false', GenerateAssemblyInfo: 'false', GenerateTargetFrameworkAttribute: 'true', AllowUnsafeBlocks: 'false', CheckForOverflowUnderflow: 'false', DebugType: 'none', DebugSymbols: 'false', Deterministic: 'true', LangVersion: 'latest', ImplicitUsings: 'disable', Nullable: 'disable'}
  const items = paths.map(path => `    <Compile Include="${xml(path)}" />`)
  for (const r of references) {
    if (r.project) items.push(`    <ProjectReference Include="${xml(r.project)}" />`)
    else {
      const name = `${r.identity.name}, Version=${r.identity.version}, Culture=${r.identity.culture || 'neutral'}, PublicKeyToken=${r.identity.public_key_token || 'null'}`
      items.push(`    <Reference Include="${xml(name)}">${r.path ? `<HintPath>${xml(r.path)}</HintPath>` : ''}</Reference>`)
    }
  }
  for (const r of resources) items.push(`    <EmbeddedResource Include="${xml(r.path)}"><LogicalName>${xml(r.name)}</LogicalName><WithCulture>false</WithCulture></EmbeddedResource>`)
  return `<Project Sdk="Microsoft.NET.Sdk">\n  <PropertyGroup>\n${Object.entries(properties).map(([key, value]) => `    <${key}>${xml(value)}</${key}>`).join('\n')}\n  </PropertyGroup>\n  <ItemGroup>\n${items.join('\n')}\n  </ItemGroup>\n</Project>\n`
}
export const projectReadme = `ILens exported solution\n\nExtract this archive, then open the .sln file in Visual Studio or the project folder in your IDE.\nEach selected assembly has its own project. Selected dependencies use ProjectReference; other references retain their metadata identity. No packages are guessed or downloaded by ILens.\n\nSources are reconstructed from CIL, not recovered original source. Read _ilens/export-report.json and each project's _ilens/export-report.json. Unsupported methods retain independent IL and diagnostic comments, not invented implementations. Compilation and equivalent behavior are not guaranteed.\n\nEmbedded resources keep their original logical names and raw bytes. BAML/XAML, .resx conversion, designers, signing, arbitrary attributes and original build settings are not reconstructed. Output type, platform and supported entry-point type are taken from PE/CLI metadata.\n\nSource export currently describes original assembly bytes. Download and reopen a modified assembly before exporting its project.\nArchive timestamps and solution IDs contain no host or user identifiers. Original assembly content may contain its own metadata.\n`
