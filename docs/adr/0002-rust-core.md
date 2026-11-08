# ADR 0002: Rust core with UniFFI bindings

**Status:** accepted

## Context
Clients are planned for terminal, desktop/web and Android, with ARCore later. ARCore is
Kotlin/Java (or Unity). MLS must be implemented exactly once and correctly.

## Decision
- All protocol, contract and trust logic lives in Rust (`nmesh-proto`, `nmesh-contracts`,
  `nmesh-core`).
- Mobile and AR get it through UniFFI (`nmesh-ffi`), the same approach `rust-nostr` and MDK use.
- UIs stay native (Compose / SceneView on Android, a web stack on desktop).
- Contracts are plain JSON files (`contracts/registry/`) so non-Rust code can read them too.

## Dependency pins
`mdk-core 0.8` depends on `nostr 0.44`, so the workspace pins `nostr`, `nostr-sdk` and
`nostr-relay-builder` to `0.44`. Mixing 0.44 and 0.45 would give two incompatible `Event`
types. **Upgrade these together** when MDK moves.

`rmcp` is built with `reqwest-tls-no-provider`, and the binaries install the `ring` rustls
provider at startup. That keeps a single crypto provider in the process and avoids an
`aws-lc-rs` version conflict.

## Consequences
- One implementation of the security-critical code.
- FFI surface design needs care: keep it coarse-grained, and use callbacks for inbound messages.
- Web needs either WASM (`nostr-sdk` supports it) or a thin TS client for the browser-only
  pieces (e.g. NIP-07).
