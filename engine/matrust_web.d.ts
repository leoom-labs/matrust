/* tslint:disable */
/* eslint-disable */

/**
 * One workspace with its figures, like one desktop Command Window.
 */
export class Session {
    free(): void;
    [Symbol.dispose](): void;
    constructor();
    /**
     * Clear the workspace, figures and all engine state.
     */
    reset(): void;
    /**
     * JSON `{"output", "error", "figures": [{"id", "svg"}], "ms"}`.
     */
    run(code: string): string;
    /**
     * JSON `[{"name", "class", "size", "value"}]`, sorted by name.
     */
    workspace(): string;
}

/**
 * MatRust version, as in the desktop About box.
 */
export function version(): string;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_session_free: (a: number, b: number) => void;
    readonly session_new: () => number;
    readonly session_reset: (a: number) => void;
    readonly session_run: (a: number, b: number, c: number) => [number, number];
    readonly session_workspace: (a: number) => [number, number];
    readonly version: () => [number, number];
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
