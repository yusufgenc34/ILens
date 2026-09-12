'use client'
import {TriangleAlert} from 'lucide-react'
import {tokenHex, type AssemblyInfo} from '../lib/types'

export const obfuscationLabels = {markers_found: 'Markers detected', possible: 'Possible obfuscation', no_markers: 'No markers found', inconclusive: 'Inspection incomplete'}
export default function AssemblyWarnings({info, onNavigate}: {info: AssemblyInfo; onNavigate: (token: number) => void}) {
  const report = info.obfuscation
  if (!report.evidence.length && !info.diagnostics.length && report.scan_complete) return null
  const tools = [...new Set(report.evidence.flatMap(e => e.tool ? [e.tool] : []))]
  return <section className="assembly-warnings" aria-label="Assembly warnings" key={info.identity.name}>
    <details>
      <summary><TriangleAlert size={16} className="warning-icon" aria-hidden="true" /><strong>{report.status === 'markers_found' ? 'Obfuscation markers detected' : report.status === 'possible' ? 'Possible obfuscation' : 'Assembly inspection warnings'}</strong>{tools.length > 0 && <span>{tools.join(', ')}</span>}<small>View details</small></summary>
      <div className="warning-details">
        <p className="warning-assembly">{info.identity.name}</p>
        {report.evidence.map((e, i) => <div className="warning-evidence" key={`${e.kind}-${i}`}><span className="evidence-badge">{e.confidence === 'marker' ? 'Metadata marker' : 'Heuristic'}</span><p>{e.detail}</p>{e.token !== null && <button className="text-button" onClick={() => onNavigate(e.token!)}>Inspect {tokenHex(e.token)}</button>}</div>)}
        {info.diagnostics.map((message, i) => <p key={i}>{message}</p>)}
        {!report.scan_complete && <p>Some attributes could not be inspected or exceeded the scan limit.</p>}
        {report.evidence.length > 0 && <p className="warning-note">Obfuscation can limit C# reconstruction. Markers can be removed or imitated; these findings do not prove which tool processed the assembly. The original IL remains available when it can be decoded.</p>}
      </div>
    </details>
  </section>
}
