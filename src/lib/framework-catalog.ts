'use client'
import type {AssemblyInfo, Identity, FrameworkPackage} from './types'
const frameworkVersions: Record<string,string> = {'2.0':'net20','3.5':'net35','4.0':'net40','4.5':'net45','4.5.1':'net451','4.5.2':'net452','4.6':'net46','4.6.1':'net461','4.6.2':'net462','4.7':'net47','4.7.1':'net471','4.7.2':'net472','4.8':'net48','4.8.1':'net481'}
// Only fixed Microsoft package IDs/versions are eligible for automatic downloads.
// Assembly names, URLs and package IDs supplied by input binaries never become URLs.
export function frameworkPackage(info: AssemblyInfo, identity: Identity): FrameworkPackage | null {
  if (!['b03f5f7f11d50a3a','b77a5c561934e089','cc7b13ffcd2ddd51','31bf3856ad364e35','7cec85d7bea7798e'].includes(identity.public_key_token.toLowerCase()) || identity.culture.toLowerCase() !== 'neutral') return null
  if (!/^(?:mscorlib|netstandard|System(?:\.[A-Za-z0-9_]+)*|Microsoft\.(?:CSharp|VisualBasic(?:\.Core)?|Win32\.[A-Za-z0-9_.]+)|WindowsBase|PresentationCore|PresentationFramework)$/.test(identity.name)) return null
  const declared = info.target_framework.status === 'declared' ? info.target_framework.moniker : null
  // Older .NET Framework builds often omit TargetFrameworkAttribute; their CLR
  // metadata version picks the matching reference pack (2.0–3.5 or 4.x).
  const runtime = info.runtime ?? ''
  const inferred = runtime.startsWith('v2.') ? '.NETFramework,Version=v3.5' : runtime.startsWith('v4.') && /^[234]\./.test(identity.version) ? '.NETFramework,Version=v4.8' : null
  const moniker = declared ?? inferred
  if (!moniker) return null
  const core = /^\.NETCoreApp,Version=v(3\.[01]|[5-9]\.0|10\.0)$/.exec(moniker)
  if (core) return {id:'microsoft.netcore.app.ref',version:`${core[1]}.0`,directory:`ref/${core[1].startsWith('3.') ? 'netcoreapp' : 'net'}${core[1]}/`}
  // Client/other profiles are subsets of the full framework directory, which also
  // carries server-only assemblies (System.Web.*) that profile apps still reference.
  const framework = /^\.NETFramework,Version=v([0-9.]+)(?:,Profile=[A-Za-z0-9]+)?$/.exec(moniker)
  const version = framework && (frameworkVersions[framework[1]] ? framework[1] : framework[1].startsWith('4.') ? '4.8' : null)
  if (version) return {id:`microsoft.netframework.referenceassemblies.${frameworkVersions[version]}`,version:'1.0.3',directory:`build/.NETFramework/v${version}/`}
  if (moniker === '.NETStandard,Version=v2.0') return {id:'netstandard.library',version:'2.0.3',directory:'build/netstandard2.0/ref/'}
  if (moniker === '.NETStandard,Version=v2.1') return {id:'netstandard.library.ref',version:'2.1.0',directory:'ref/netstandard2.1/'}
  return null
}
