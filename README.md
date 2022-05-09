<h1 align="center">
  🦅🦁 glyphon
</h1>
<div align="center">
  Fast, simple 2D text rendering for wgpu
</div>
<br />
<div align="center">
  <img src="https://img.shields.io/badge/Min%20Rust-1.54-green.svg" alt="Minimum Rust Version">
  <a href="https://crates.io/crates/glyphon"><img src="https://img.shields.io/crates/v/glow.svg?label=glyphon" alt="crates.io"></a>
  <a href="https://docs.rs/glyphon"><img src="https://docs.rs/glyphon/badge.svg" alt="docs.rs"></a>
  <a href="https://github.com/grovesNL/glyphon/actions"><img src="https://github.com/grovesNL/glyphon/workflows/CI/badge.svg?branch=main" alt="Build Status" /></a>
</div>

## What is this?

This crate provides a simple way to render 2D text with [`wgpu`](https://github.com/gfx-rs/wgpu/) by:

- rasterizing glyphs (with [`fontdue`](https://github.com/mooman219/fontdue/))
- packing the glyphs into texture atlas (with [`etagere`](https://github.com/nical/etagere/))
- calculate layout for text (with [`fontdue`](https://github.com/mooman219/fontdue/))
- sampling from the texture atlas to render text (with [`wgpu`](https://github.com/gfx-rs/wgpu/))

To avoid extra render passes, rendering uses existing render passes(following the middleware pattern described in [`wgpu`'s Encapsulating Graphics Work wiki page](https://github.com/gfx-rs/wgpu/wiki/Encapsulating-Graphics-Work).

## License

This project is licensed under either [Apache License, Version 2.0](LICENSE-APACHE), [zlib License](LICENSE-ZLIB), or [MIT License](LICENSE-MIT), at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this project by you, as defined in the Apache 2.0 license, shall be triple licensed as above, without any additional terms or conditions.
