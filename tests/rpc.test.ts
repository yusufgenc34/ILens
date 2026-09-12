import { describe, it, expect, vi, afterEach } from 'vitest'
import { DecompilerClient, type WorkerPort } from '../src/lib/rpc'
import type { Response, WorkerMessage } from '../src/lib/types'
class FakeWorker implements WorkerPort {
  sent: WorkerMessage[] = []
  onmessage: WorkerPort['onmessage'] = null
  onerror: WorkerPort['onerror'] = null
  onmessageerror: WorkerPort['onmessageerror'] = null
  terminated = false
  postMessage(message: WorkerMessage) {this.sent.push(message)}
  terminate() {this.terminated = true}
  respond(response: Response) {this.onmessage?.({data: response} as MessageEvent<Response>)}
}
afterEach(() => vi.useRealTimers())
describe('Worker RPC lifecycle', () => {
  it('discards a trapped WASM worker and rejects its remaining requests', async () => {
    const worker = new FakeWorker(); const client = new DecompilerClient(worker)
    const fatal = vi.fn(); client.onFatal = fatal
    const first = client.call('getTree', {assembly: 1}); const second = client.call('getTree', {assembly: 2})
    const rejected = [expect(first).rejects.toMatchObject({code: 'worker_error'}), expect(second).rejects.toMatchObject({code: 'worker_error'})]
    worker.respond({id: 1, kind: 'fatal', error: {code: 'worker_error', message: 'WASM stopped', detail: 'Runtime trap'}})
    await Promise.all(rejected)
    expect(worker.terminated).toBe(true); expect(fatal).toHaveBeenCalledOnce()
  })
  it('routes out-of-order responses by request ID', async () => {
    const worker = new FakeWorker(); const client = new DecompilerClient(worker)
    const first = client.call('getTree', {assembly: 1}); const second = client.call('getTree', {assembly: 2})
    worker.respond({id: 2, kind: 'success', result: ['second']}); worker.respond({id: 1, kind: 'success', result: ['first']})
    expect(await first).toEqual(['first']); expect(await second).toEqual(['second']); client.terminate()
  })
  it('discards stale responses after cancellation', async () => {
    const worker = new FakeWorker(); const client = new DecompilerClient(worker); const controller = new AbortController()
    const pending = client.call('getTree', {assembly: 1}, {signal: controller.signal})
    const rejected = expect(pending).rejects.toMatchObject({code: 'cancelled'})
    controller.abort(); await rejected
    worker.respond({id: 1, kind: 'success', result: ['stale']})
    expect(worker.sent[1]).toMatchObject({op: 'cancel', target: 1}); client.terminate()
  })
  it('terminates a blocked worker and rejects every pending request', async () => {
    vi.useFakeTimers(); const worker = new FakeWorker(); const client = new DecompilerClient(worker)
    const pending = client.call('getTree', {assembly: 1}); const rejected = expect(pending).rejects.toMatchObject({code: 'size_limit'})
    await vi.advanceTimersByTimeAsync(30_001); await rejected; expect(worker.terminated).toBe(true)
  })
  it('returns structured parser errors without poisoning subsequent requests', async () => {
    const worker = new FakeWorker(); const client = new DecompilerClient(worker)
    const bad = client.call('load', {buffer: new ArrayBuffer(8)}); const rejected = expect(bad).rejects.toMatchObject({code: 'invalid_pe'})
    worker.respond({id: 1, kind: 'error', error: {code: 'invalid_pe', message: 'Invalid PE', detail: 'Missing MZ'}}); await rejected
    const good = client.call('getTree', {assembly: 2}); worker.respond({id: 2, kind: 'success', result: []}); expect(await good).toEqual([]); client.terminate()
  })
})
