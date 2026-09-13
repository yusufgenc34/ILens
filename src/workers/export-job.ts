'use client'
import type {Decompiler} from '../wasm/decompiler'
import {compilationUnit, exportReadme, renderType, reportText, safeName, type ExportMember, type ExportOptions, type ExportProgress, type ExportReport, type ExportResult, type ExportType} from '../lib/export'
import {diagnostic, jsonText, tokenHex, type EditsInfo, type Overview} from '../lib/types'
import {createZipArchive} from '../lib/zip'
interface Plan {revision: number; types: ExportType[]}
class ExportJob {
  private plan: Plan
  private overview: Overview
  private tokens: number[]
  private roots: ExportType[]
  private members = new Map<number, ExportMember>()
  private archive = createZipArchive()
  private cursor = 0
  private retained = 0
  private cancelled = false
  private singleSource: {text: string; name: string; partial: boolean} | null = null
  private dependencies: {name: string; id: number; path: string}[]
  private report: ExportReport = {outcome: 'complete', build_ready: false, notices: ['C#-like reconstruction, not original source or a guarantee of compilation.', 'Source output describes original bytes; pending IL edits are exported separately.', 'Custom-attribute previews are bounded; not all metadata can be reproduced as C# attributes.'], members: [], resources: [], types: []}
  constructor(private core: Decompiler, readonly id: number, private assembly: number, private options: ExportOptions) {
    this.plan = core.export_plan(assembly, options.scope === 'type' ? options.token : 0) as Plan
    this.overview = core.get_assembly_info(assembly) as Overview
    this.tokens = this.plan.types.flatMap(t => t.members)
    this.roots = this.plan.types.filter(t => !this.plan.types.some(parent => parent.token === t.parent))
    this.dependencies = options.dependencies ? this.overview.references.filter(r => r.resolved_id !== null).map(r => ({name: r.identity.name, id: r.resolved_id!, path: `references/${safeName(r.identity.name, r.token)}.dll`})) : []
    for (const reference of this.overview.references.filter(r => r.status === 'unresolved')) this.report.notices.push(`Reference unavailable: ${reference.identity.name}, ${reference.identity.version}.`)
  }
  get total() {return this.tokens.length + this.roots.length + this.overview.resources.length + this.dependencies.length}
  progress(): ExportProgress {return {job: this.id, completed: this.cursor, total: this.total, phase: this.cursor < this.tokens.length ? 'Reconstructing members' : 'Building source archive', done: this.cursor >= this.total}}
  private check() {
    if (this.cancelled) throw {code: 'cancelled', message: 'Export cancelled.', detail: 'Export buffers were released.'}
    if ((this.core.get_edits(this.assembly) as EditsInfo).revision !== this.plan.revision) throw {code: 'cancelled', message: 'The workspace changed during export.', detail: 'Start the export again with the current workspace.'}
  }
  step(): ExportProgress {
    this.check()
    try {
      let index = this.cursor
      if (index < this.tokens.length) this.member(this.tokens[index])
      else if ((index -= this.tokens.length) < this.roots.length) this.type(this.roots[index])
      else if ((index -= this.roots.length) < this.overview.resources.length) this.resource(index)
      else if ((index -= this.overview.resources.length) < this.dependencies.length) {
        const dependency = this.dependencies[index]
        this.archive.add(dependency.path, this.core.export_dependency(dependency.id, this.plan.revision))
      }
      if (this.cursor < this.total) this.cursor++
      return this.progress()
    } catch (error) {this.cancel(); throw error}
  }
  private member(token: number) {
    let member: ExportMember
    try {member = this.core.export_member(this.assembly, token, this.plan.revision) as ExportMember}
    catch (error) {
      if (error instanceof WebAssembly.RuntimeError || diagnostic(error).code === 'cancelled') throw error
      member = {token, name: tokenHex(token), kind: 'member', declaration: '', body: null, il: '', quality: 'error', diagnostics: [diagnostic(error).detail], accessors: [], metadata: {error: diagnostic(error)}}
    }
    const text = jsonText(member)
    this.retained += text.length * 2
    if (this.retained > 32 * 1024 * 1024) throw new Error('Reconstructed members exceed the 32 MiB working budget. Export individual types instead.')
    this.members.set(token, member)
    const path = member.il ? `il/${safeName(member.name, token)}.il` : null
    if (path) this.archive.add(path, member.il + '\n')
    this.archive.add(`metadata/${token.toString(16).padStart(8, '0')}.json`, jsonText(member.metadata) + '\n')
    this.report.members.push({token: tokenHex(token), name: member.name, quality: member.quality, diagnostics: member.diagnostics, path})
    if (member.quality === 'error' || member.quality === 'annotated_il' || member.diagnostics.length) this.report.outcome = 'partial'
  }
  private type(type: ExportType) {
    const rendered = renderType(type, this.plan.types, this.members)
    const namespace = type.namespace ? safeName(type.namespace) : 'global'
    const path = `src/${namespace}/${safeName(type.name, type.token)}${rendered.partial ? '.partial' : ''}.cs`
    const text = compilationUnit(type, rendered.text, rendered.partial)
    this.archive.add(path, text)
    if (this.options.scope === 'type' && type.token === this.options.token) this.singleSource = {text, name: `${safeName(type.name, type.token)}.cs`, partial: rendered.partial}
    this.report.types.push({token: type.token, path, diagnostics: type.diagnostics})
    if (rendered.partial) this.report.outcome = 'partial'
  }
  private resource(index: number) {
    const resource = this.overview.resources[index]
    try {
      const bytes = this.core.export_resource(this.assembly, resource.token, this.plan.revision)
      const path = `resources/${safeName(resource.name, resource.token)}.bin`
      this.archive.add(path, bytes); this.report.resources.push({token: resource.token, name: resource.name, path})
    } catch (error) {
      if (error instanceof WebAssembly.RuntimeError || diagnostic(error).code === 'cancelled') throw error
      this.report.outcome = 'partial'; this.report.resources.push({token: resource.token, name: resource.name, path: null, error: diagnostic(error).detail})
    }
  }
  finish(): ExportResult {
    this.check()
    if (this.cursor < this.total) throw new Error('The export job has not finished.')
    try {
      if (this.options.format === 'source') {
        if (!this.singleSource || this.singleSource.partial) throw new Error('This type is only partially reconstructed. Use Export type sources to keep the independent IL and diagnostics.')
        return {name: this.singleSource.name, mime: 'text/plain;charset=utf-8', buffer: new TextEncoder().encode(this.singleSource.text).buffer, report: this.report}
      }
      this.archive.add('README.txt', exportReadme)
      this.archive.add('assembly.json', jsonText(this.overview) + '\n')
      this.archive.add('export-report.json', reportText(this.report))
      this.members.clear(); this.retained = 0
      const buffer = this.archive.finish()
      return {name: `${safeName(this.overview.info.identity.name)}.${this.options.scope === 'type' ? `type-${this.options.token.toString(16)}.` : ''}sources.zip`, mime: 'application/zip', buffer, report: this.report}
    } finally {this.cancel()}
  }
  cancel() {this.cancelled = true; this.members.clear(); this.archive.clear(); this.retained = 0; this.singleSource = null}
}

// Lowercase factory exports keep worker helpers out of Rari component discovery.
export function createExportJob(core: Decompiler, id: number, assembly: number, options: ExportOptions) {return new ExportJob(core, id, assembly, options)}
