'use client'
import {ui} from '../lib/ui'
import type {DetectionReport} from '../lib/types'
export const verdictLabels: Record<DetectionReport['verdict'], string> = {packed: 'Packed', protected: 'Protected', obfuscated: 'Obfuscated', suspicious: 'Suspicious indicators', no_findings: 'No findings'}
// Plain-text summary used where the report cannot be rendered, such as load errors.
export function describeReport(report: DetectionReport) {
  const found = report.detections.map(d => `${d.name} (${d.kind}, ${d.confidence})`).join(', ')
  return `Protection scan: ${verdictLabels[report.verdict]}${found ? ` · ${found}` : ''}${report.signals.length ? ` · ${report.signals.length} indicators: ${report.signals.map(s => s.detail).join('; ')}` : ''}`
}
export default function ProtectionView({report,onDeobfuscate}: {report: DetectionReport | null; onDeobfuscate?:()=>void}) {
  if (!report) return <p role="status" className="p-4 text-[12px] text-muted-foreground">Scanning for packers and protectors…</p>
  return <div className="protection-view h-full overflow-auto px-5 py-4 text-[12px] [scrollbar-width:thin]" role="region" aria-label="Protection scan report">
    <div className="mb-1 flex items-center gap-3"><h2 className="text-[15px] font-semibold">{verdictLabels[report.verdict]}</h2><span className="rounded-[4px] bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground">{report.managed ? 'Managed assembly' : 'Native image'}</span></div>
    <p className="mb-4 max-w-170 text-muted-foreground leading-relaxed">Static scan of sections, metadata, resources and method bodies. Signatures identify markers, not verified tool provenance; heuristics only indicate that a file may be packed or protected. No code is executed or unpacked.</p>
    {onDeobfuscate && <button className={ui.button} onClick={onDeobfuscate}>Deobfuscate assembly…</button>}
    <h3 className="mb-2 mt-4 font-medium">Detections</h3>
    {report.detections.length ? <ul className="mb-5 space-y-2">{report.detections.map(d => <li key={d.name} className="rounded-[6px] border border-solid border-border p-3"><div className="flex flex-wrap items-center gap-2"><strong className="font-medium">{d.name}</strong><span className="rounded-[4px] bg-muted px-1.5 text-[11px] text-muted-foreground">{d.kind}</span><span className={d.confidence === 'signature' ? 'text-[11px] text-warning-foreground' : 'text-[11px] text-muted-foreground'}>{d.confidence}</span></div><ul className="mt-1.5 list-disc pl-5 text-muted-foreground">{d.evidence.map((e, i) => <li key={i} className="[overflow-wrap:anywhere]">{e}</li>)}</ul></li>)}</ul> : <p className="mb-5 text-muted-foreground">No known packer, protector or obfuscator signature matched.</p>}
    <h3 className="mb-2 font-medium">Indicators</h3>
    {report.signals.length ? <ul className="list-disc space-y-1 pl-5 text-muted-foreground">{report.signals.map((s, i) => <li key={i} className="[overflow-wrap:anywhere]"><code className="mr-1.5 font-mono text-[11px] text-foreground">{s.kind}</code>{s.detail}</li>)}</ul> : <p className="text-muted-foreground">No generic packing indicators.</p>}
  </div>
}
