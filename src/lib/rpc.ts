import type { Diagnostic, Operation, Operations, Request, Response, WorkerMessage } from './types'
export interface WorkerPort {
  postMessage(message: WorkerMessage, transfer?: Transferable[]): void
  terminate(): void
  onmessage: ((event: MessageEvent<Response>) => void) | null
  onerror: ((event: ErrorEvent) => void) | null
  onmessageerror: ((event: MessageEvent) => void) | null
}
interface Pending { resolve: (value: unknown) => void; reject: (error: Diagnostic) => void; cleanup: () => void }
const cancelled = (): Diagnostic => ({code: 'cancelled', message: 'Operation cancelled.', detail: 'The response was discarded.'})
export class DecompilerClient {
  private sequence = 0
  private pending = new Map<number, Pending>()
  private closed = false
  onProgress?: (message: string) => void
  onFatal?: (error: Diagnostic) => void
  constructor(private worker: WorkerPort) {
    worker.onmessage = ({data}) => {
      if (data.kind === 'fatal') { this.fail(data.error); return }
      const pending = this.pending.get(data.id)
      if (!pending) return
      if (data.kind === 'progress') { this.onProgress?.(data.message); return }
      this.pending.delete(data.id)
      pending.cleanup()
      if (data.kind === 'error') pending.reject(data.error)
      else pending.resolve(data.result)
    }
    worker.onerror = event => { event.preventDefault(); this.fail({code: 'worker_error', message: 'The isolated analysis worker stopped. Reopen your assemblies to continue.', detail: event.message}) }
    worker.onmessageerror = () => this.fail({code: 'worker_error', message: 'The worker returned an unreadable response.', detail: 'Message deserialization failed.'})
  }
  call<K extends Operation>(op: K, input: Operations[K]['input'], options: {signal?: AbortSignal; transfer?: Transferable[]} = {}): Promise<Operations[K]['output']> {
    if (this.closed) return Promise.reject({code: 'worker_error', message: 'The workspace is closed.', detail: 'Worker terminated.'})
    if (options.signal?.aborted) return Promise.reject(cancelled())
    const id = ++this.sequence
    return new Promise((resolve, reject) => {
      const abort = () => {
        const entry = this.pending.get(id)
        if (!entry) return
        this.pending.delete(id); entry.cleanup()
        this.worker.postMessage({id: ++this.sequence, op: 'cancel', target: id})
        reject(cancelled())
      }
      // A synchronous WASM call cannot process cancel messages. The watchdog provides hard isolation.
      const timer = setTimeout(() => this.fail({code: 'size_limit', message: 'Analysis exceeded 30 seconds. The worker was stopped; reopen your assemblies to continue.', detail: 'Hard watchdog terminated the WASM worker and released its memory.'}), 30_000)
      const cleanup = () => { clearTimeout(timer); options.signal?.removeEventListener('abort', abort) }
      this.pending.set(id, {resolve: value => resolve(value as Operations[K]['output']), reject, cleanup})
      options.signal?.addEventListener('abort', abort, {once: true})
      try { this.worker.postMessage({id, op, ...input} as Request, options.transfer ?? []) }
      catch (error) { this.pending.delete(id); cleanup(); reject(error) }
    })
  }
  private fail(error: Diagnostic) { this.terminate(error); this.onFatal?.(error) }
  terminate(error = cancelled()) {
    if (this.closed) return
    this.closed = true; this.worker.terminate()
    for (const entry of this.pending.values()) { entry.cleanup(); entry.reject(error) }
    this.pending.clear()
  }
}
