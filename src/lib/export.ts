import {jsonText, tokenHex, type Declaration, type MethodAnalysis} from './types'
import type {ExportType, ExportMember, ExportReport, DownloadFile} from './export-types'
export type * from './export-types'
export const sourceNotice = '// Reconstructed by ILens from CIL and metadata. This is not the original source.\n'
// ASCII components plus token suffixes avoid platform, Unicode and case-folding collisions.
export function safeName(name: string, token?: number): string {
  const base = name.normalize('NFKC').replace(/[^a-zA-Z0-9_-]+/g, '_').replace(/^_+|_+$/g, '').slice(0, 80) || 'assembly'
  const safe = /^(con|prn|aux|nul|com[0-9]|lpt[0-9])$/i.test(base) ? `_${base}` : base
  return token === undefined ? safe : `${safe}_${token.toString(16).padStart(8, '0')}`
}
export function methodDownload(method: MethodAnalysis, member: Declaration, language: 'C#' | 'IL'): DownloadFile {
  if (method.token !== member.token) throw new Error('The selected method changed. Select it again before exporting.')
  if (language === 'C#' && method.quality === 'annotated_il') throw new Error('C# reconstruction is unavailable. Download the IL instead.')
  const extension = language === 'IL' ? 'il' : 'cs'
  const diagnostics = method.diagnostics.map(d => `// ${d.detail.replace(/[\r\n\u2028\u2029]/g, ' ')}`).join('\n')
  const text = `${sourceNotice}// Method ${tokenHex(method.token)} · ${method.quality}\n${diagnostics ? `${diagnostics}\n` : ''}\n${language === 'IL' ? method.il : method.csharp}\n`
  return {name: `${safeName(member.name, member.token)}.${extension}`, mime: 'text/plain;charset=utf-8', buffer: new TextEncoder().encode(text.replace(/\r\n?/g, '\n')).buffer}
}
export function downloadFile(file: DownloadFile) {
  const url = URL.createObjectURL(new Blob([file.buffer], {type: file.mime}))
  const anchor = document.createElement('a'); anchor.href = url; anchor.download = file.name
  document.body.append(anchor); anchor.click(); anchor.remove()
  // Allow the browser download task to acquire the Blob before revoking its URL.
  setTimeout(() => URL.revokeObjectURL(url), 1000)
}
const indent = (text: string) => text.split('\n').map(line => line ? `    ${line}` : '').join('\n')
const comment = (text: string) => text.replace(/[\r\n\u2028\u2029]/g, ' ')
export function renderType(type: ExportType, types: ExportType[], members: Map<number, ExportMember>, depth = 0): {text: string; partial: boolean} {
  if (depth > 48) throw new Error('Nested type export exceeds 48 levels.')
  if (type.kind === 'module') return {text: type.declaration, partial: type.members.length > 0}
  let partial = type.diagnostics.length > 0
  const children: string[] = []
  const accessorOwners = new Map<number, number>()
  for (const token of type.members) for (const [, accessor] of members.get(token)?.accessors ?? []) accessorOwners.set(accessor, token)
  for (const token of type.members) {
    const member = members.get(token)
    if (!member) {children.push(`// ${tokenHex(token)} could not be exported; see export-report.json.`); partial = true; continue}
    if (member.diagnostics.length || member.quality === 'annotated_il' || member.quality === 'error') partial = true
    if (type.kind === 'delegate' || accessorOwners.has(token) || member.quality === 'implicit') continue
    const prefix = `// ${tokenHex(token)}${member.diagnostics.length ? ` · ${comment(member.diagnostics.join(' '))}` : ''}\n`
    if (member.quality === 'error' || member.quality === 'annotated_il') {children.push(`${prefix}// ${comment(member.declaration || member.name)} — body retained in IL.`); continue}
    if (member.accessors.length) {
      const accessors: string[] = []
      let unavailable = member.diagnostics.length > 0
      for (const [kind, accessorToken] of member.accessors) {
        const accessor = members.get(accessorToken)
        if (!accessor || accessor.quality === 'annotated_il' || accessor.quality === 'error' || kind === 'unsupported') {unavailable = true; break}
        accessors.push(`${kind}${accessor.body ? `\n${accessor.body}` : ';'}`)
      }
      if (unavailable) {children.push(`${prefix}// ${comment(member.declaration)} — accessors retained in IL.`); partial = true}
      else children.push(`${prefix}${member.declaration}\n{\n${indent(accessors.join('\n'))}\n}`)
    } else if (token >>> 24 === 6) children.push(`${prefix}${member.declaration}${member.body ? `\n${member.body}` : ';'}`)
    else children.push(prefix + member.declaration)
  }
  for (const nested of types.filter(t => t.parent === type.token)) {const rendered = renderType(nested, types, members, depth + 1); children.push(rendered.text); partial ||= rendered.partial}
  return {text: type.kind === 'delegate' ? type.declaration : `${type.declaration}\n{\n${indent(children.join('\n\n'))}\n}\n`, partial}
}
export function compilationUnit(type: ExportType, rendered: string, partial: boolean): string {
  const namespace = type.namespace ? type.namespace.split('.').map(n => /^[a-zA-Z_][a-zA-Z0-9_]*$/.test(n) ? `@${n}` : safeName(n)).join('.') : ''
  const note = partial ? '// PARTIAL reconstruction. Read export-report.json and the independent IL files.\n' : '// C#-like reconstruction; compilation and semantic equivalence are not guaranteed.\n'
  return `${sourceNotice}${note}#nullable disable\n\n${namespace ? `namespace ${namespace}\n{\n${indent(rendered)}\n}\n` : rendered}`
}
export const frameworkChoices = ['net10.0', 'net9.0', 'net8.0', 'net7.0', 'net6.0', 'net5.0', 'netcoreapp3.1', 'netcoreapp3.0', 'netcoreapp2.2', 'netcoreapp2.1', 'netcoreapp2.0', 'netcoreapp1.1', 'netcoreapp1.0', 'netstandard2.1', 'netstandard2.0', 'netstandard1.6', 'netstandard1.5', 'netstandard1.4', 'netstandard1.3', 'netstandard1.2', 'netstandard1.1', 'netstandard1.0', 'net481', 'net48', 'net472', 'net471', 'net47', 'net462', 'net461', 'net46', 'net452', 'net451', 'net45', 'net40', 'net35', 'net30', 'net20']
export function inferFramework(moniker: string | null): string {
  if (!moniker) return ''
  const match = /^(\.NETCoreApp|\.NETStandard|\.NETFramework),Version=v(\d+\.\d+(?:\.\d+)?)$/.exec(moniker)
  if (!match) return ''
  const [, family, version] = match
  const target = family === '.NETFramework' ? `net${version.replaceAll('.', '')}` : family === '.NETStandard' ? `netstandard${version}` : `${Number(version.split('.')[0]) >= 5 ? 'net' : 'netcoreapp'}${version}`
  return frameworkChoices.includes(target) ? target : ''
}
export function xml(value: string): string {
  // Escape MSBuild expansion/list/wildcard syntax before XML entities.
  return value.replace(/[%$@;*?()]/g, c => `%${c.charCodeAt(0).toString(16).toUpperCase()}`).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;').replace(/'/g, '&apos;')
}
export const exportReadme = `ILens reconstructed sources\n\nThese files reconstruct C#-like code from CIL and metadata, not original source.\nRead export-report.json before using the output. Unsupported members remain in IL.\nPartial types have a .partial.cs suffix; no replacement implementations are invented.\nUse File > Export to Project for a solution and separate assembly projects.\nReferences are never fetched automatically. No NuGet package identities are guessed.\nEmbedded resources are opaque data; external resources and designer recovery are unsupported.\nSource exports describe the original assembly, not pending IL edits. Use modified assembly export for those.\nArchive timestamps are fixed. ILens adds no host paths or personal identifiers; original assembly content may contain its own metadata.\n`
export const reportText = (report: ExportReport) => jsonText(report) + '\n'
