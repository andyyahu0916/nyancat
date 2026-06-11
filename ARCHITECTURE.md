# Architecture

This document records the current module boundaries and the design constraints that keep the Rust edition maintainable, testable, and performance-conscious.

## Design Goals

- Keep terminal animation output compatible with the historical nyancat behavior.
- Keep the render path allocation-light and predictable.
- Keep Unix FFI and unsafe calls behind a small safe API.
- Keep command-line parsing, telnet negotiation, terminal detection, rendering, and runtime cleanup independently testable.
- Prefer small typed boundaries over broad framework-style abstractions.

## Startup Flow

`main.rs` owns process orchestration:

1. Parse arguments with `cli::parse_args`.
2. Apply benchmark and telnet startup policy.
3. Install terminal/session cleanup through `runtime::TerminalSession`.
4. Negotiate telnet metadata when `--telnet` is active.
5. Detect terminal type and create `render::Palette`.
6. Create `render::RenderState` and run `render::run`.
7. Restore the terminal before printing deferred benchmark reports or runtime errors.

`main.rs` should stay thin. New behavior should usually live in the module that owns the relevant domain.

## Module Boundaries

| Module | Ownership |
| :--- | :--- |
| `animation.rs` | Raw frame data, frame dimensions, `FrameSymbol`, and frame symbol accessors. |
| `cli.rs` | `Config`, crop option types, CLI actions, CLI errors, and option parsing. |
| `terminal.rs` | Positive terminal size type, terminal size detection adapter, and terminal type classification. |
| `telnet.rs` | Telnet parser, negotiation state, byte-source abstraction, negotiated terminal metadata, and mid-session input draining (`SessionInput`). |
| `render.rs` | Render orchestration, `RenderState`, frame composition, intro output, resize handling, and `RunOutcome`. |
| `render/palette.rs` | Terminal-specific palette tables and O(1) frame-symbol lookup. |
| `render/frame_buffer.rs` | Reusable frame byte buffer, telnet newline conversion, and frame prefix helpers. |
| `render/render_loop.rs` | Frame index advancement, frame-limit tracking, and target-delay sleeping. |
| `render/benchmark.rs` | Benchmark frame accounting and stable report formatting. |
| `runtime.rs` | Terminal restore sequences, signal handlers, resize flag, and session RAII guard. |
| `sys.rs` | Unix FFI declarations and safe wrappers for signals, typed poll/read outcomes, write, ioctl, and `_exit`. |
| `main.rs` | Process-level composition and exit-code decisions. |

## Data Flow

CLI arguments become `cli::Config`. `main.rs` combines `Config`, terminal metadata, and `terminal::TerminalType` into:

- `render::Palette`
- `render::RenderState`
- `render::run(config, state, palette)`

`render::run` repeatedly:

1. Updates terminal size after resize signals (local SIGWINCH) or, in telnet mode, after a mid-session NAWS update drained from the client between frames.
2. Clears and prefixes a reusable `render::frame_buffer::FrameBuffer`.
3. Calls `Renderer::render_frame`.
4. Writes the buffer to stdout and flushes.
5. Advances `render::render_loop::RenderLoop` state or returns `RunOutcome`.

Frame data remains private to `animation.rs`. Rendering obtains symbols through `frame_symbol(frame, row, col)`, which returns `FrameSymbol`. Palette lookup remains an O(1) array index via `FrameSymbol::as_byte()`.

Block-mode rows end with `\x1b[K` (erase-to-line-end). Cells are two columns wide, so an odd terminal width leaves a one-column gap on the right that the counter's `\x1b[J` would otherwise fill only on the last row, producing a one-cell protrusion at the bottom-right corner. This per-row fill is a **fork-specific divergence** from the historical C renderer; it is visually a no-op at even widths and is excluded from ASCII modes, which have no background to fill.

Terminal dimensions enter the core as `terminal::TerminalSize`, which stores non-zero `u16` values capped at 10000 columns/rows and exposes signed accessors for crop arithmetic. Syscall and telnet inputs that report zero, invalid, or extreme dimensions are rejected at the adapter boundary and fall back to defaults when appropriate, so untrusted NAWS/ioctl metadata cannot force pathological render loops.

`terminal::detect_terminal_type` is a faithful port of the historical `TERM`-matching chain, with one deliberate **fork-specific divergence**: any `TERM` containing `256color` that the historical chain would otherwise leave unclassified (notably `screen-256color` and `tmux-256color`) maps to the 256-color palette instead of the upstream 16-color fallback. The check is placed last, so every explicit historical mapping — including `rxvt-256color` vs. `rxvt` — is unchanged.

CLI option metadata lives in `cli::OPTION_SPECS`. Parsing, value arity, and generated `--help` output all use that table so public option drift is caught in one module.

The geometric crop options (`--min-rows`, `--max-rows`, `--min-cols`, `--max-cols`, `--width`, `--height`) are range-checked at the CLI boundary: sizes to `1..=10000` and offsets to `-10000..=10000`. This is a **fork-specific safety bound** not present in the historical C implementation. The limits sit far beyond any real terminal, so they do not affect normal use, but they stop extreme values from overflowing crop arithmetic — both the centered-range computation in `cli.rs` and the rainbow-tail negation in the render path — and from forcing multi-billion-iteration render loops. As a result the render core only ever receives crop values that are already known to be sane, consistent with the goal of validating untrusted input at the boundary rather than in the hot path.

## Runtime And Signals

Normal execution restores the terminal through `TerminalSession` drop. Signal paths cannot rely on normal unwinding, so they use raw async-signal-compatible output and `sys::exit`. `SIGHUP`, `SIGINT`, `SIGPIPE`, and `SIGTERM` all route to the same restore-and-exit handler, so `kill`, a closed terminal, or Ctrl-C leave the terminal in a clean state.

In clear-screen mode the animation runs on the **alternate screen buffer** (`\x1b[?1049h` at startup, `\x1b[?1049l` on restore), so the terminal's prior contents and scrollback are restored on exit instead of being cleared. The `--no-clear` path is unchanged (save/restore cursor only), which is why the golden smokes — all run with `--no-clear` — are unaffected.

The resize signal path only sets an atomic flag. The render loop consumes that flag, recalculates crop bounds in normal code, and (in clear-screen mode) clears the screen for that one frame so a now-narrower or shorter animation cannot leave stale cells from the previous terminal size along the right edge or below the cat.

## Telnet Flow

Telnet support is intentionally synchronous:

- `TelnetParser` converts bytes into typed parser events.
- `TelnetNegotiation` handles typed command / option state transitions and output bytes.
- `ByteSource` lets tests drive negotiation with scripted input.
- `TimeoutReader` is the production stdin/poll source.
- `SessionInput` drains client bytes between frames after negotiation: mid-session NAWS updates reflow the animation (with the same one-shot screen clear as a local resize), and EOF or a failed read ends the session cleanly. Reads are capped per frame so a flooding client cannot starve rendering. This is a **fork-specific divergence** — the historical C implementation never reads the connection after negotiation, so client resizes were ignored and client bytes accumulated unread.

Do not introduce async unless the deployment model changes. The current tool speaks telnet over stdin/stdout for socket activation, inetd, or similar supervisors.

## Error Policy

Use `io::Result` where the domain is already I/O-bound. Add an app-level error enum only if errors need shared semantics across runtime, telnet, rendering, and CLI boundaries.

Current process policy:

- CLI errors print a stable user-facing message and exit failure.
- Broken pipe is treated as successful termination (a downstream reader closing the pipe, e.g. `| head`, is normal).
- Other runtime I/O errors print after terminal restore and exit failure, so callers and scripts can detect a genuine failure.

## Performance Policy

The render path may use typed wrappers when they compile down to simple value passing and array indexing. Avoid abstractions that add per-cell allocation, dynamic dispatch, or avoidable format work.

Before making performance claims:

1. Build with `cargo build --release`.
2. Run `--benchmark --frames ...` with stdout redirected to `/dev/null`.
3. Record environment and results in the benchmark section of `RELEASE_CHECKLIST.md`.

## Test Policy

The baseline for behavior changes is:

- `cargo fmt --check`
- `cargo test`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo build --release`
- Smoke paths from `RELEASE_CHECKLIST.md`

For render or terminal-output changes, compare smoke output against the committed goldens and inspect whether changed bytes are intentional.

## Extension Guidelines

- Add CLI options through `OPTION_SPECS` first, then implement behavior.
- Keep `--help` text generated from `OPTION_SPECS`; do not add hand-maintained option lists inside `cli.rs`.
- Keep raw frame strings inside `animation.rs`.
- Keep unsafe code inside `sys.rs`. This is compiler-enforced: the crate root carries `#![deny(unsafe_code)]` and only `mod sys` is `#[allow(unsafe_code)]`.
- Keep terminal cleanup behavior centralized in `runtime.rs`.
- Keep telnet parser/state logic independently testable.
- Update `ARCHITECTURE.md` when a module takes on a new responsibility.
- Update `ROADMAP.md` when a deferred technical direction becomes accepted project scope.
