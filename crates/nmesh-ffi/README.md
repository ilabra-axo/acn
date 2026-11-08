# nmesh-ffi (placeholder)

UniFFI bindings over `nmesh-core` for **Kotlin (Android / ARCore)** and Swift.

Not a workspace member yet, so it doesn't slow down builds. It will be added in roadmap Phase 4
(see `docs/ROADMAP.md`). Planned shape:

- `uniffi` (proc-macro mode), crate type `cdylib` + `staticlib`
- Exposes: `Session` (connect, send_text, send_structured, send_action, inbox as a callback
  interface), `ContractRegistry` (validate, pinned_ref), `resolve_renderer`
- Built with `cargo-ndk` for `aarch64-linux-android` / `x86_64-linux-android`
- Kotlin side renders Tier 0 declarative renderers with Jetpack Compose and spatial contracts
  with ARCore Geospatial / Cloud Anchors + SceneView (glTF)

Rationale: docs/adr/0002-rust-core.md.
