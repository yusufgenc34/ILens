import type {Diagnostic, Operation, Operations, Request, Response, WorkerMessage} from './types'
export interface WorkerPort {
  postMessage(message: WorkerMessage, transfer?: Transferable[]): void
  terminate(): void
  onmessage: ((event: MessageEvent<Response>) => void) | null
  onerror: ((event: ErrorEvent) => void) | null
  onmessageerror: ((event: MessageEvent) => void) | null
}
interface Pending {resolve: (value: unknown) => void; reject: (error: Diagnostic) => void; cleanup: () => void; send: () => void}
const cancelled = (): Diagnostic => ({code: 'cancelled', message: 'Operation cancelled.', detail: 'The response was discarded.'})
export class DecompilerClient {
  private sequence = 0
  private pending = new Map<number, Pending>()
  private queue: number[] = []
  private active: number | null = null
  private timer: ReturnType<typeof setTimeout> | undefined
  private closed = false
  onProgress?: (message: string) => void
  onFatal?: (error: Diagnostic) => void
  constructor(private worker: WorkerPort) {
    worker.onmessage = ({data}) => {
      if (data.kind === 'fatal') {this.fail(data.error); return}
      const pending = this.pending.get(data.id)
      if (data.kind === 'progress') {if (pending) this.onProgress?.(data.message); return}
      if (pending) {
        this.pending.delete(data.id); pending.cleanup()
        if (data.kind === 'error') pending.reject(data.error)
        else pending.resolve(data.result)
      }
      if (data.id === this.active) {clearTimeout(this.timer); this.active = null; this.pump()}
    }
    worker.onerror = event => {event.preventDefault(); this.fail({code: 'worker_error', message: 'The isolated analysis worker stopped. Reopen your assemblies to continue.', detail: event.message})}
    worker.onmessageerror = () => this.fail({code: 'worker_error', message: 'The worker returned an unreadable response.', detail: 'Message deserialization failed.'})
  }
  call<K extends Operation>(op: K, input: Operations[K]['input'], options: {signal?: AbortSignal; transfer?: Transferable[]} = {}): Promise<Operations[K]['output']> {
    if (this.closed) return Promise.reject({code: 'worker_error', message: 'The workspace is closed.', detail: 'Worker terminated.'})
    if (this.pending.size >= 128) return Promise.reject({code: 'size_limit', message: 'Too many pending operations.', detail: 'The client queue is limited to 128 requests.'})
    if (options.signal?.aborted) return Promise.reject(cancelled())
    const id = ++this.sequence
    return new Promise((resolve, reject) => {
      const abort = () => {
        const entry = this.pending.get(id)
        if (!entry) return
        this.pending.delete(id); entry.cleanup(); this.queue = this.queue.filter(queued => queued !== id)
        // Keep the active watchdog until the worker acknowledges cancellation.
        // A synchronous WASM call cannot receive cancellation while it is running.
        if (this.active === id) this.worker.postMessage({id: ++this.sequence, op: 'cancel', target: id})
        reject(cancelled())
      }
      this.pending.set(id, {resolve: value => resolve(value as Operations[K]['output']), reject, cleanup: () => options.signal?.removeEventListener('abort', abort), send: () => this.worker.postMessage({id, op, ...input} as Request, options.transfer ?? [])})
      options.signal?.addEventListener('abort', abort, {once: true})
      this.queue.push(id); this.pump()
    })
  }
  private pump() {
    if (this.active !== null || this.closed) return
    while (this.queue.length) {
      const id = this.queue.shift()!; const pending = this.pending.get(id)
      if (!pending) continue
      this.active = id
      // Only execution time counts. Queued navigation and export steps do not
      // consume each other's deadline, and cancellation cannot disable isolation.
      this.timer = setTimeout(() => this.fail({code: 'size_limit', message: 'Analysis exceeded 30 seconds. The worker was stopped; reopen your assemblies to continue.', detail: 'Hard watchdog terminated the WASM worker and released its memory.'}), 30_000)
      try {pending.send()}
      catch (error) {clearTimeout(this.timer); this.active = null; this.pending.delete(id); pending.cleanup(); pending.reject({code: 'worker_error', message: 'The operation could not be sent.', detail: String(error)}); continue}
      return
    }
  }
  private fail(error: Diagnostic) {this.terminate(error); this.onFatal?.(error)}
  terminate(error = cancelled()) {
    if (this.closed) return
    this.closed = true; this.worker.terminate(); clearTimeout(this.timer); this.active = null; this.queue = []
    for (const entry of this.pending.values()) {entry.cleanup(); entry.reject(error)}
    this.pending.clear()
  }
}
