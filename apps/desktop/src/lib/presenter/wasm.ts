// Loader for the real ACN validator compiled to WebAssembly (crates/acn-wasm).
// JSON-in/JSON-out ABI: acn_alloc / acn_invoke / acn_free. Rebuild with `pnpm wasm`.

interface Exports {
  memory: WebAssembly.Memory;
  acn_alloc(len: number): number;
  acn_free(ptr: number, len: number): void;
  acn_invoke(ptr: number, len: number): bigint;
}

let exports: Exports | null = null;
const enc = new TextEncoder();
const dec = new TextDecoder();

export async function loadAcn(url = "/acn.wasm"): Promise<void> {
  if (exports) return;
  const res = await fetch(url);
  if (!res.ok) throw new Error(`could not load ${url}: ${res.status}`);
  const { instance } = await WebAssembly.instantiate(await res.arrayBuffer(), {});
  exports = instance.exports as unknown as Exports;
}

/** Call an acn-wasm op. Throws on `{err}`. */
export function acn<T = any>(op: string, args: Record<string, unknown> = {}): T {
  if (!exports) throw new Error("acn.wasm not loaded");
  const input = enc.encode(JSON.stringify({ op, ...args }));
  const ptr = exports.acn_alloc(input.length);
  new Uint8Array(exports.memory.buffer, ptr, input.length).set(input);
  const packed = exports.acn_invoke(ptr, input.length);
  exports.acn_free(ptr, input.length);
  const rptr = Number(packed >> 32n);
  const rlen = Number(packed & 0xffffffffn);
  const text = dec.decode(new Uint8Array(exports.memory.buffer, rptr, rlen).slice());
  exports.acn_free(rptr, rlen);
  const out = JSON.parse(text);
  if ("err" in out) throw new Error(`acn ${op}: ${out.err}`);
  return out.ok as T;
}

export function randomHex32(): string {
  const b = new Uint8Array(32);
  crypto.getRandomValues(b);
  return Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
}
