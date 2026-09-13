'use client'
import {TriangleAlert} from 'lucide-react'
import {tokenHex, type AssemblyInfo} from '../lib/types'

export const obfuscationLabels = {markers_found: 'Markers detected', possible: 'Possible obfuscation', no_markers: 'No markers found', inconclusive: 'Inspection incomplete'}
export default function AssemblyWarnings({info, onNavigate}: {info: AssemblyInfo; onNavigate: (token: number) => void}) {
  const report = info.obfuscation
  if (!report.evidence.length && !info.diagnostics.length && report.scan_complete) return null
  const tools = [...new Set(report.evidence.flatMap(e => e.tool ? [e.tool] : []))]
  return <section className="assembly-warnings flex-none border-b border-solid border-b-warning-border bg-warning-background text-[12px] [&_summary]:flex [&_summary]:items-center [&_summary]:flex-wrap [&_summary]:gap-2.5 [&_summary]:py-[11px] [&_summary]:px-4.5 [&_summary]:text-warning-foreground [&_summary]:cursor-pointer [&_summary]:list-none [&_summary::-webkit-details-marker]:hidden [&_summary_strong]:font-[550] [&_summary_span]:text-warning-foreground [&_summary_span]:opacity-85 [&_summary_small]:ml-auto [&_summary_small]:text-[11px] [&_summary_small]:underline [&_summary_small]:underline-offset-[3px] [&_summary:focus-visible]:[outline:2px_solid_var(--ring)] [&_summary:focus-visible]:[outline-offset:-2px]" aria-label="Assembly warnings" key={info.identity.name}>
    <details>
      <summary><TriangleAlert size={16} className="warning-icon shrink-0 text-warning-icon animate-warning-pulse motion-reduce:animate-none" aria-hidden="true" /><strong>{report.status === 'markers_found' ? 'Obfuscation markers detected' : report.status === 'possible' ? 'Possible obfuscation' : 'Assembly inspection warnings'}</strong>{tools.length > 0 && <span>{tools.join(', ')}</span>}<small>View details</small></summary>
      <div className="warning-details bg-information-background border-t border-solid border-t-information-border max-h-57.5 overflow-auto py-3 px-4.5 leading-[1.7] [overflow-wrap:anywhere] [&_p]:mt-1 [&_p]:mr-0 [&_p]:mb-2.5 [&_p]:ml-0">
        <p className="warning-assembly font-mono font-medium">{info.identity.name}</p>
        {report.evidence.map((e, i) => <div className="warning-evidence py-2.5 px-0 border-b border-solid border-b-information-border" key={`${e.kind}-${i}`}><span className="evidence-badge inline-block text-information-accent border border-solid border-information-border py-px px-1.5 rounded-[4px] text-[10px]">{e.confidence === 'marker' ? 'Metadata marker' : 'Heuristic'}</span><p>{e.detail}</p>{e.token !== null && <button className="inline-flex gap-1.5 items-center border-0 bg-transparent p-0 text-[12px] text-foreground hover:underline hover:underline-offset-1" onClick={() => onNavigate(e.token!)}>Inspect {tokenHex(e.token)}</button>}</div>)}
        {info.diagnostics.map((message, i) => <p key={i}>{message}</p>)}
        {!report.scan_complete && <p>Some attributes could not be inspected or exceeded the scan limit.</p>}
        {report.evidence.length > 0 && <p className="warning-note text-muted-foreground pt-2 max-w-250">Obfuscation can limit C# reconstruction. Markers can be removed or imitated; these findings do not prove which tool processed the assembly. The original IL remains available when it can be decoded.</p>}
      </div>
    </details>
  </section>
}
