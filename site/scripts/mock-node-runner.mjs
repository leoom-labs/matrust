// Node adapter to verify the browser mock without changing its worker contract.
import { parentPort } from 'node:worker_threads';
globalThis.self = { postMessage: message => parentPort.postMessage(message), onmessage:null };
await import('../src/try/mock-worker.js');
parentPort.on('message', data => self.onmessage({data}));
