export interface Figure { id: number; svg: string; }
export interface RunResult { output: string; error: string | null; figures: Figure[]; ms: number; }
export interface WorkspaceEntry { name: string; class: string; size: string | number[]; value: string; }
export interface LoadProgress { loaded: number; total: number | null; preview: boolean; }
type Method = 'run' | 'reset' | 'workspace' | 'version';
interface Pending { resolve: (value: unknown) => void; reject: (reason: Error) => void; timer: ReturnType<typeof setTimeout>; }
export class EngineClient {
  private worker?: Worker;
  private workerUrl?: string | URL;
  private loading?: Promise<void>;
  private abort?: AbortController;
  private pending = new Map<number, Pending>();
  private sequence = 0;
  private generation = 0;
  // Set once a worker has answered: later worker errors are crashes, not load failures.
  private loaded = false;
  constructor(private onProgress: (progress: LoadProgress | null) => void, private onPreview: (preview: boolean) => void) {}
  private spawn() {
    if (!this.workerUrl) return;
    const worker = this.workerUrl === '/engine/worker.js'
      ? new Worker('/engine/worker.js', { type: 'module' })
      : new Worker(new URL('./mock-worker.js', import.meta.url), { type: 'module' });
    this.worker = worker;
    worker.onmessage = ({ data }: MessageEvent<{ id: number; result: unknown }>) => {
      this.loaded = true;
      const request = this.pending.get(data.id);
      if (!request) return;
      clearTimeout(request.timer);
      this.pending.delete(data.id);
      request.resolve(data.result);
    };
    worker.onerror = event => {
      event.preventDefault();
      worker.terminate();
      if (this.worker !== worker) return;
      this.worker = undefined;
      // A crashed engine respawns on the next request; a failed load downloads again.
      if (!this.loaded) this.workerUrl = undefined;
      this.fail(new Error(this.loaded ? 'The engine stopped. The workspace was cleared.' : 'The engine worker could not load. Try Run again.'));
    };
    worker.onmessageerror = () => { this.stop('The engine returned an unreadable response. Workspace cleared.'); };
  }
  private fail(error: Error) {
    for (const request of this.pending.values()) { clearTimeout(request.timer); request.reject(error); }
    this.pending.clear();
  }
  private async initialize() {
    const generation = this.generation;
    const abort = new AbortController();
    this.abort = abort;
    this.onProgress({ loaded: 0, total: null, preview: false });
    try {
      const response = await fetch('/engine/worker.js', { signal: abort.signal });
      const html = response.headers.get('content-type')?.includes('text/html');
      if (response.status === 404 || (response.ok && html)) {
        this.workerUrl = new URL('./mock-worker.js', import.meta.url);
        this.onPreview(true);
        this.onProgress({ loaded: 0, total: null, preview: true });
      } else {
        if (!response.ok) throw new Error(`Engine download failed (${response.status}). Try again.`);
        await response.arrayBuffer();
        // Read the Wasm response progressively once. The worker's normal init
        // then uses the browser HTTP cache; no worker API changes are required.
        let measuredSize: number | null = null;
        try {
          const metadata = await fetch('/engine/size.json', { signal: abort.signal });
          if (metadata.ok && metadata.headers.get('content-type')?.includes('application/json')) {
            const size = await metadata.json() as { bytes?: number };
            if (typeof size.bytes === 'number' && Number.isSafeInteger(size.bytes) && size.bytes > 0) measuredSize = size.bytes;
          }
        } catch { /* Older engine bundles still work without size metadata. */ }
        this.onProgress({ loaded: 0, total: measuredSize, preview: false });
        const wasm = await fetch('/engine/matrust_web_bg.wasm', { signal: abort.signal });
        if (!wasm.ok || wasm.headers.get('content-type')?.includes('text/html')) throw new Error('Engine files are incomplete. Please try again later.');
        const header = Number(wasm.headers.get('content-length'));
        const total = measuredSize ?? (header > 0 && !wasm.headers.get('content-encoding') ? header : null);
        let loaded = 0;
        this.onProgress({ loaded, total, preview: false });
        if (wasm.body) {
          const reader = wasm.body.getReader();
          while (true) {
            const part = await reader.read();
            if (part.done) break;
            loaded += part.value.byteLength;
            this.onProgress({ loaded, total, preview: false });
          }
        } else { loaded = (await wasm.arrayBuffer()).byteLength; }
        this.onProgress({ loaded, total: loaded, preview: false });
        this.workerUrl = '/engine/worker.js';
        this.onPreview(false);
      }
      if (generation !== this.generation || abort.signal.aborted) throw new Error('Execution stopped. Workspace cleared.');
      this.spawn();
      // Confirms the module and Wasm loaded before any script runs.
      await this.post('version');
    } catch (error) {
      if (abort.signal.aborted) throw new Error('Execution stopped. Workspace cleared.');
      throw error;
    } finally {
      if (generation === this.generation) { this.onProgress(null); this.abort = undefined; }
    }
  }
  private async ready() {
    if (this.worker) return;
    if (this.loaded && this.workerUrl && !this.loading) { this.spawn(); return; }
    if (!this.loading) this.loading = this.initialize().finally(() => { this.loading = undefined; });
    await this.loading;
  }
  async request(type: Method, code?: string): Promise<unknown> {
    const generation = this.generation;
    await this.ready();
    if (generation !== this.generation || !this.worker) throw new Error('Execution stopped. Workspace cleared.');
    return this.post(type, code);
  }
  private post(type: Method, code?: string): Promise<unknown> {
    const worker = this.worker!;
    const id = ++this.sequence;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => this.stop('Stopped after 10 seconds. Workspace cleared; you can run again.'), 10000);
      this.pending.set(id, { resolve, reject, timer });
      worker.postMessage({ type, id, ...(code === undefined ? {} : { code }) });
    });
  }
  async run(code: string): Promise<RunResult> { return JSON.parse(String(await this.request('run', code))) as RunResult; }
  async reset(): Promise<void> {
    if (!this.worker && !this.loading) return;
    await this.request('reset');
  }
  async workspace(): Promise<WorkspaceEntry[]> { return JSON.parse(String(await this.request('workspace'))) as WorkspaceEntry[]; }
  stop(message = 'Execution stopped. Workspace cleared.') {
    this.generation++;
    this.abort?.abort();
    this.abort = undefined;
    this.worker?.terminate();
    this.worker = undefined;
    this.fail(new Error(message));
    this.onProgress(null);
    // An existing bundle can restart immediately. A cancelled download retries on Run.
    if (this.workerUrl) this.spawn();
  }
  dispose() {
    this.generation++;
    this.abort?.abort();
    this.worker?.terminate();
    this.worker = undefined;
    this.fail(new Error('Console closed.'));
  }
}
