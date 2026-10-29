# Offscreen frame compositor

Part 1 of the splitscreen series (#3541). The renderer exposes its offscreen frame and can hand
its window surface to a compositor on the same GPU device. The compositor clears uncovered
pixels to black and copies each BGRA view to its assigned position, clipping at the window edge.
Returning the surface through `Renderer::take_back` lets the next renderer draw on the window
again. The normal game path does not create a compositor.

A standalone example exercises this without game resources, profiles, audio or launcher data:

```sh
cargo run --profile fast -p horizon-client-render --example compositor
cargo run --profile fast -p horizon-client-render --example compositor -- --smoke
```

The window shows a red view on the left and a blue one on the right. Resize it to check clipping
and uncovered black regions. The smoke run presents 20 frames, switches vsync off, tests zero and
oversized dimensions, resizes the window, then exits. Unsupported surface copy formats return an
error. Unit coverage checks clipping while a view still has its old size during a resize.
