export interface ExportType {token: number; parent: number; name: string; namespace: string; kind: string; declaration: string; members: number[]; diagnostics: string[]}
export interface ExportMember {token: number; name: string; kind: string; declaration: string; body: string | null; il: string; quality: string; diagnostics: string[]; accessors: [string, number][]; metadata: Record<string, unknown>}
export interface ExportReport {outcome: 'complete' | 'partial'; build_ready: false; notices: string[]; members: {token: string; name: string; quality: string; diagnostics: string[]; path: string | null}[]; resources: {token: number; name: string; path: string | null; error?: string}[]; types: {token: number; path: string; diagnostics: string[]}[]}
export interface ExportOptions {scope: 'type' | 'assembly'; token: number; dependencies: boolean; format?: 'zip' | 'source'}
export interface ExportProgress {job: number; completed: number; total: number; phase: string; done: boolean}
export interface DownloadFile {name: string; mime: string; buffer: ArrayBuffer}
export interface ExportResult extends DownloadFile {report: ExportReport}
export interface ProjectExportOptions {assemblies: {id: number; framework: string}[]; solutionName: string; createSolution: boolean; dependencies: boolean; diagnostics: boolean}
export interface ProjectReport {assembly: number; name: string; project: string; outcome: 'complete' | 'partial'; report: string}
export interface ProjectExportReport extends ExportReport {projects: ProjectReport[]}
