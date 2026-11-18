# Profiling the game with Tracy

`m2-alpha` has an optional `tracy` feature, off by default. Without it the
build has no profiler code and no extra dependencies.

```
cargo build --release -p m2-alpha --features tracy
```

With the feature on, the game starts a Tracy client and sends:

- a frame mark at the start of each frame (`App::frame`);
- one zone for every `spans::time` span, under the same name the `pace:`
  log lines use (`ticks`, `game`, `scripts`, `scripts: lua`, `stream`,
  `draw`, `present`, `hud`, ...), so a log line and a trace can be read
  side by side;
- the scopes wgpu and egui already emit through the `profiling` crate.

`spans::Lap` sections are summed for the `pace:` lines only; they end
after the fact, so they are not Tracy zones.

Run the game as usual, then connect the Tracy profiler (or record headless
with `tracy-capture -o out.tracy`). The client speaks the protocol of
Tracy 0.14 (`tracy-client` 0.18, the version `profiling` 1.0 uses); use a
profiler of that version: a 0.15 profiler refuses the connection. The
client is built with `timer-fallback`, since on a machine without an
invariant TSC (a VM) it would otherwise end the game at start, and with
`ondemand`, so nothing is recorded or kept in memory until a profiler
connects. Tracy's sampling and context-switch data need an elevated process; zones and frame marks do not.

Provenance: invented (a development aid, nothing of the original game's).

# Timing the CPU side without a GPU

`m2-alpha` and `m2-render` have an optional `noop` feature, off by
default, which builds in wgpu's no-GPU backend. Run with it selected and
the game does all its CPU work (simulation, culling, the draw queue,
wgpu's validation and command recording) while nothing reaches a GPU, so
the CPU's share of a frame can be measured on a machine whose GPU is busy
or absent:

```
cargo build --release -p m2-alpha --features noop
WGPU_BACKEND=noop WGPU_NOOP_BACKEND=1 m2-alpha --install <game> --bench 150 --size 2560x1440 --screenshot out.ppm
```

The `bench:` lines' `cpu ms a frame` parts and `encode cpu ms a frame`
stages are real; the `gpu ms a frame` stages read zero, and the picture
is empty. Without `WGPU_BACKEND=noop` the build runs on the GPU as usual.

The bench's CPU parts: `tick` (the game's step), `stream` (the scene
loading and placing what is near), `scene` (the scene putting up its
draws), `queue` (sorting them and handing each to the backend), `submit`
(encoding the frame's passes, wgpu's `finish` and the submit) and `wait`
(for the frame before last).

Provenance: invented (a development aid).
