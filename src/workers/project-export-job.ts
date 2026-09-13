'use client'
import type {Decompiler} from '../wasm/decompiler'
import {compilationUnit, inferFramework, renderType} from '../lib/export'
import {assemblyInfoFile, createProjectPaths, projectGuid, projectName, projectReadme, projectSettings, rootNamespace, sdkProjectFile, solutionFile, typePath, type ProjectDescriptor, type ProjectReference} from '../lib/project-export'
import {diagnostic, jsonText, tokenHex, type EditsInfo} from '../lib/types'
import type {ExportMember, ExportProgress, ExportReport, ExportResult, ExportType, ProjectExportOptions, ProjectExportReport} from '../lib/export-types'
import {createZipArchive} from '../lib/zip'

interface Plan {revision: number; types: ExportType[]}
interface CurrentProject {
  descriptor: ProjectDescriptor; plan: Plan; roots: ExportType[]; tokens: number[]; members: Map<number, ExportMember>; cursor: number; total: number; retained: number
  paths: ReturnType<typeof createProjectPaths>; report: ExportReport; references: ProjectReference[]; copies: {id: number; path: string}[]
}
class ProjectExportJob {
  private archive = createZipArchive()
  private projects: ProjectDescriptor[]
  private current: CurrentProject | null = null
  private index = 0
  private cancelled = false
  private revision: number
  private report: ProjectExportReport = {outcome: 'complete', build_ready: false, notices: ['Reconstructed projects require review; compilation is not guaranteed. See each project report for missing constructs.'], members: [], resources: [], types: [], projects: []}
  constructor(private core: Decompiler, readonly id: number, private options: ProjectExportOptions) {
    if (!options.assemblies.length || options.assemblies.length > 16 || new Set(options.assemblies.map(a => a.id)).size !== options.assemblies.length) throw new Error('Select between 1 and 16 distinct loaded assemblies.')
    const names = createProjectPaths()
    this.projects = options.assemblies.map((a, index) => {
      const overview = core.get_assembly_info(a.id) as ProjectDescriptor['overview']
      const directory = names([overview.info.identity.name])
      // Validate the framework/template before starting any expensive work.
      const descriptor = {id: a.id, name: overview.info.identity.name, directory, file: `${directory}.csproj`, guid: projectGuid(index), framework: a.framework, overview}
      sdkProjectFile(descriptor, [], [], [], [])
      return descriptor
    })
    this.revision = (core.get_edits(this.projects[0].id) as EditsInfo).revision
  }
  progress(): ExportProgress {
    const fraction = this.current ? this.current.cursor / this.current.total : 0
    return {job: this.id, completed: Math.floor((this.index + fraction) * 1000), total: this.projects.length * 1000, phase: this.current ? `Exporting ${this.current.descriptor.name} · ${this.current.cursor} / ${this.current.total}` : this.index === this.projects.length ? 'Projects ready' : `Preparing ${this.projects[this.index].name}`, done: this.index === this.projects.length}
  }
  private check() {
    if (this.cancelled || (this.core.get_edits(this.projects[0].id) as EditsInfo).revision !== this.revision) throw {code: 'cancelled', message: 'Project export cancelled.', detail: 'The workspace changed or the export was cancelled.'}
  }
  private prepare(): CurrentProject {
    const descriptor = this.projects[this.index]
    const plan = this.core.export_plan(descriptor.id, 0) as Plan
    if (plan.revision !== this.revision) throw new Error('The workspace changed during project export.')
    const roots = plan.types.filter(t => t.parent === 0 && t.kind !== 'module')
    const tokens = plan.types.flatMap(t => t.members)
    const report: ExportReport = {outcome: 'complete', build_ready: false, notices: ['Original build settings, arbitrary attributes, signing and designer output are not reconstructed.', 'Embedded resources are raw data with their original logical names; no managed objects are deserialized.'], members: [], types: [], resources: []}
    if (descriptor.framework !== inferFramework(descriptor.overview.info.target_framework.moniker)) {report.outcome = 'partial'; report.notices.push('The target framework was selected manually. Review compatibility.')}
    if (projectName(descriptor.name) !== descriptor.name) {report.outcome = 'partial'; report.notices.push('The assembly output name required filesystem-safe mapping; review assembly identity and reference binding.')}
    const settings = projectSettings(descriptor.overview, plan.types)
    if (settings.entry && !settings.startup) {report.outcome = 'partial'; report.notices.push('The entry-point declaring type cannot be represented as StartupObject. Review executable startup configuration.')}
    const edits = this.core.get_edits(descriptor.id) as EditsInfo
    if (edits.methods.length) report.notices.push('Pending IL edits are not included. Download and reopen the modified assembly to export its sources.')
    const paths = createProjectPaths()
    paths(['Properties', 'AssemblyInfo'], '.cs')
    const references: ProjectReference[] = []; const copies: CurrentProject['copies'] = []
    for (const ref of descriptor.overview.references) {
      const peer = this.projects.find(p => p.id === ref.resolved_id)
      if (peer && peer.id !== descriptor.id) references.push({identity: ref.identity, project: `../${peer.directory}/${peer.file}`})
      else if (this.options.dependencies && ref.resolved_id !== null && ref.resolved_id !== descriptor.id) {
        const path = paths(['References', ref.identity.name], '.dll')
        references.push({identity: ref.identity, path}); copies.push({id: ref.resolved_id, path})
      } else if (ref.resolved_id !== descriptor.id) {
        references.push({identity: ref.identity})
        report.notices.push(`Reference without a bundled file: ${ref.identity.name}, ${ref.identity.version}. It must be supplied by the target SDK or the developer.`)
      }
    }
    return {descriptor, plan, roots, tokens, members: new Map(), cursor: 0, total: tokens.length + roots.length + descriptor.overview.resources.length + copies.length + 1, retained: 0, paths, report, references, copies}
  }
  private add(project: CurrentProject, path: string, data: string | Uint8Array) {this.archive.add(`${project.descriptor.directory}/${path}`, data)}
  step(): ExportProgress {
    this.check()
    try {
      if (this.index === this.projects.length) return this.progress()
      if (!this.current) {this.current = this.prepare(); return this.progress()}
      const p = this.current; let i = p.cursor
      if (i < p.tokens.length) this.member(p, p.tokens[i])
      else if ((i -= p.tokens.length) < p.roots.length) {
        const type = p.roots[i]; const rendered = renderType(type, p.plan.types, p.members)
        const path = typePath(type, rootNamespace(p.roots), p.paths)
        this.add(p, path, compilationUnit(type, rendered.text, rendered.partial))
        p.report.types.push({token: type.token, path, diagnostics: type.diagnostics})
        if (rendered.partial) p.report.outcome = 'partial'
      } else if ((i -= p.roots.length) < p.descriptor.overview.resources.length) this.resource(p, i)
      else if ((i -= p.descriptor.overview.resources.length) < p.copies.length) this.add(p, p.copies[i].path, this.core.export_dependency(p.copies[i].id, this.revision))
      else this.finishProject(p)
      p.cursor++
      if (p.cursor === p.total) {p.members.clear(); this.current = null; this.index++}
      return this.progress()
    } catch (error) {this.cancel(); throw error}
  }
  private member(p: CurrentProject, token: number) {
    let member: ExportMember
    try {member = this.core.export_member(p.descriptor.id, token, this.revision) as ExportMember}
    catch (error) {
      if (error instanceof WebAssembly.RuntimeError || diagnostic(error).code === 'cancelled') throw error
      member = {token, name: tokenHex(token), kind: 'member', declaration: '', body: null, il: '', quality: 'error', diagnostics: [diagnostic(error).detail], accessors: [], metadata: {error: diagnostic(error)}}
    }
    p.retained += jsonText(member).length * 2
    if (p.retained > 32 * 1024 * 1024) throw new Error('Project members exceed the 32 MiB working budget. Export individual types instead.')
    p.members.set(token, member)
    const partial = member.quality === 'error' || member.quality === 'annotated_il' || member.diagnostics.length > 0
    const path = member.il && (this.options.diagnostics || partial) ? `_ilens/il/${token.toString(16).padStart(8, '0')}.il` : null
    if (path) this.add(p, path, member.il + '\n')
    if (this.options.diagnostics || partial) this.add(p, `_ilens/metadata/${token.toString(16).padStart(8, '0')}.json`, jsonText(member.metadata) + '\n')
    p.report.members.push({token: tokenHex(token), name: member.name, quality: member.quality, diagnostics: member.diagnostics, path})
    if (partial) p.report.outcome = 'partial'
  }
  private resource(p: CurrentProject, index: number) {
    const resource = p.descriptor.overview.resources[index]
    let data: Uint8Array
    try {
      data = this.core.export_resource(p.descriptor.id, resource.token, this.revision)
    } catch (error) {
      if (error instanceof WebAssembly.RuntimeError || ['cancelled', 'size_limit'].includes(diagnostic(error).code)) throw error
      p.report.outcome = 'partial'; p.report.resources.push({token: resource.token, name: resource.name, path: null, error: diagnostic(error).detail})
      return
    }
    const path = p.paths(['Resources', resource.name])
    this.add(p, path, data); p.report.resources.push({token: resource.token, name: resource.name, path})
  }
  private finishProject(p: CurrentProject) {
    this.add(p, 'Properties/AssemblyInfo.cs', assemblyInfoFile(p.descriptor.overview.info.identity))
    this.add(p, p.descriptor.file, sdkProjectFile(p.descriptor, p.plan.types, [...p.report.types.map(t => t.path), 'Properties/AssemblyInfo.cs'], p.references, p.report.resources.filter(r => r.path).map(r => ({name: r.name, path: r.path!}))))
    this.add(p, '_ilens/assembly.json', jsonText(p.descriptor.overview) + '\n')
    this.add(p, '_ilens/export-report.json', jsonText(p.report) + '\n')
    if (p.report.outcome === 'partial') this.report.outcome = 'partial'
    this.report.projects.push({assembly: p.descriptor.id, name: p.descriptor.name, project: `${p.descriptor.directory}/${p.descriptor.file}`, outcome: p.report.outcome, report: `${p.descriptor.directory}/_ilens/export-report.json`})
  }
  finish(): ExportResult {
    this.check()
    if (this.index !== this.projects.length) throw new Error('Project export has not finished.')
    try {
      const name = projectName(this.options.solutionName.replace(/\.sln$/i, ''))
      if (this.options.createSolution) this.archive.add(`${name}.sln`, solutionFile(this.projects))
      this.archive.add('_ilens/export-report.json', jsonText(this.report) + '\n')
      this.archive.add('README.txt', projectReadme)
      return {name: `${name}.zip`, mime: 'application/zip', buffer: this.archive.finish(), report: this.report}
    } finally {this.cancel()}
  }
  cancel() {this.cancelled = true; this.current?.members.clear(); this.current = null; this.archive.clear()}
}
export function createProjectExportJob(core: Decompiler, id: number, options: ProjectExportOptions) {return new ProjectExportJob(core, id, options)}
