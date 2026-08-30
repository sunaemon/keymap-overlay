# Display Model Contract

`display-model.schema.json` is generated from the canonical Rust semantic model
in `keymap-overlay-generator`. The native renderers consume this versioned JSON
shape directly or through the shared Rust type.

Run `make generate-contracts` after intentionally changing the model, and run
`make check-contracts` to verify that the committed schema and fixtures still
match the Rust source. Contract version 2 evolves additively; a breaking change
requires a new version and migration coverage in every renderer.

The fixtures cover the base layer, a composed transparent layer, and an encoder.
The shared base and held-layer fixtures drive the simulated AppKit, GNOME Shell,
Qt Quick, and WPF E2E paths, so the renderer boundary is exercised with the same
models validated here.
