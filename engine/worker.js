// MatRust Web Worker: new Worker(new URL("worker.js", base), { type: "module" }).
// The page posts {type: "run" | "reset" | "workspace" | "version", id, code?}
// and receives {id, result}. A run cannot be interrupted from inside: to stop
// a runaway run, terminate this worker and start a new one.
import init, { Session, version } from "./matrust_web.js";

const ready = init();
let session = null;

// A failed message still answers {id, result}: a run gets its usual JSON with
// the error, other types get null. `error` repeats the message for any type.
function fail(id, type, message) {
    const result =
        type === "run" ? JSON.stringify({ output: "", error: message, figures: [], ms: 0 }) : null;
    self.postMessage({ id, result, error: message });
}

self.onmessage = async (event) => {
    const { type, id, code } = event.data ?? {};
    try {
        await ready;
    } catch (error) {
        fail(id, type, `MatRust could not load: ${error}`);
        return;
    }
    let result = null;
    try {
        session ??= new Session();
        switch (type) {
            case "run":
                result = session.run(String(code ?? ""));
                break;
            case "reset":
                session.reset();
                break;
            case "workspace":
                result = session.workspace();
                break;
            case "version":
                result = version();
                break;
            default:
                fail(id, type, `Unknown message type: ${type}`);
                return;
        }
    } catch (error) {
        // A Rust panic aborts the call (RuntimeError: unreachable) and leaves the
        // Wasm instance in an unknown state. Rethrow: the page's worker.onerror
        // terminates this worker and starts a fresh one on the next run.
        session = null;
        throw error;
    }
    self.postMessage({ id, result });
};
