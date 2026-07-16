# Release Checklist

This checklist is the baseline for merging release candidates and publishing builds. It is intentionally stricter than the minimum needed for local development.

## Release Inputs

- Confirm the target version in `Cargo.toml`.
- Confirm `CHANGELOG` has an entry for the target version.
- Confirm `README.md`, `nyancat.1`, and systemd files still match the current CLI and deployment model.
- Confirm `ARCHITECTURE.md` reflects any structural changes made during the release cycle.
- Confirm `ROADMAP.md` reflects any newly accepted engineering priorities or deferred work.
- Confirm the working tree is clean before final verification.

## Required Verification

Run the automated baseline from the repository root:

```bash
scripts/release_check.sh
```

The script covers:

- `cargo fmt --check`
- `cargo test --locked`, including CLI option coverage for `README.md` and `nyancat.1`
- `cargo clippy --locked --all-targets --all-features -- -D warnings`
- `cargo build --release --locked`
- `sh -n` syntax checks for release helper scripts
- `cargo package --list --allow-dirty --locked`, including expected release files and excluding local-only dotfiles
- `scripts/release_archive.sh` with a temporary dist directory, followed by archive content checks
- Smoke tests with exact golden comparisons, alternate-screen restore markers, CLI/telnet write-error checks, and `--help` option coverage

GitHub Actions also runs the release check on stable Rust, a separate MSRV build/test job for Rust 1.85.0, and macOS test/build/smoke coverage against the committed goldens.
The release workflow requires a tag matching `v${Cargo.toml version}`, builds Linux and macOS archives before publishing, and attaches a `SHA256SUMS` file. Failed drafts can be rebuilt, but an already published release is immutable and the workflow refuses to replace its live assets.

If you need to run the steps manually, use the commands in the sections below.

## Smoke Tests

Run the release binary after `cargo build --release`, or use `scripts/release_check.sh`:

```bash
env TERM=xterm-256color target/release/nyancat --frames 1 --no-title --no-clear --no-counter >/tmp/nyancat-rust-smoke.out
target/release/nyancat --telnet --skip-intro --frames 1 --no-title --no-clear --no-counter >/tmp/nyancat-rust-telnet-smoke.out
env TERM=xterm-256color target/release/nyancat --truecolor --frames 1 --no-title --no-clear --no-counter >/tmp/nyancat-rust-truecolor-smoke.out
env TERM=xterm-256color target/release/nyancat --frames 1 --width 40 --height 24 --no-title --no-clear --no-counter >/tmp/nyancat-rust-crop-smoke.out
env TERM=xterm-256color target/release/nyancat --benchmark --frames 3 --no-title --no-clear --no-counter >/tmp/nyancat-rust-benchmark-smoke.out 2>/tmp/nyancat-rust-benchmark-smoke.err
target/release/nyancat --wat
```

Output is verified by exact comparison against committed golden files in `tests/golden/`
(`normal.out`, `telnet.out`, `truecolor.out`, `crop.out`, `benchmark.out`). `scripts/release_check.sh`
runs each smoke command and `cmp`s the result against its golden, so a mismatch reports the first
differing byte instead of an opaque checksum.

When a render or output change is intentional, regenerate the goldens and review the diff before committing:

```bash
scripts/update_goldens.sh
git diff --stat tests/golden/
```

By default `scripts/update_goldens.sh` rebuilds `target/release/nyancat` first. Set `NYANCAT_BIN` only when you intentionally want to regenerate against a specific existing executable.

For reference, the current golden sizes are 4002 (normal), 3067 (telnet), 5175 (truecolor), 4083 (crop), and 11916 (benchmark) bytes.

The CLI error smoke must return a non-zero status and print:

```text
nyancat: unknown option: --wat
Try 'target/release/nyancat --help' for usage.
```

The benchmark smoke stderr must include:

```text
benchmark: frames=3
```

The automated smoke checks also verify key output markers:

- xterm output uses 256-color escape sequences and does not emit TrueColor sequences.
- TrueColor output uses 24-bit escape sequences and does not emit 256-color sequences.
- telnet output starts with negotiation bytes and uses CR NUL LF newline markers.
- `--no-counter` smoke paths do not print counter text.
- clear-screen startup enters and restores the alternate screen; SIGTERM smokes verify signal-path restore and status 143 even when stdout closes during cleanup.
- redirected help and benchmark diagnostics do not contain terminal styling.
- genuine local and telnet negotiation write failures return non-zero.

## Benchmarking

When performance changes are intentional, verify them with the deterministic instruction-count benchmark (immune to CPU frequency scaling and scheduler noise), and optionally refresh the wall-clock snapshot below.

```bash
scripts/benchmark_callgrind.sh 1000   # objective: retired-instruction counts via valgrind (preferred)
scripts/benchmark_matrix.sh 100000 5  # supplementary: wall-clock FPS, environment-dependent
```

Record:

- Commit SHA
- Rust version
- OS and kernel
- CPU model and topology
- Build profile
- Output destination
- Runs per mode
- Full benchmark reports

### Current Snapshot

Benchmark snapshots are local measurements, not portable guarantees. Hardware, CPU governor, kernel, Rust version, terminal mode, build profile, and output destination can all change the numbers.

For comparable render-throughput measurements, build in release mode and redirect stdout to `/dev/null`. `scripts/benchmark_matrix.sh` rebuilds the default release binary, runs each mode multiple times, verifies deterministic byte stats, and reports the sample with median elapsed time. Set `NYANCAT_BIN` only when benchmarking a specific existing executable.

#### 2026-05-25

- Commit: `18243a4`
- Rust: `rustc 1.95.0 (59807616e 2026-04-14)`
- OS: `Linux 7.0.10-1-cachyos x86_64 GNU/Linux`
- CPU: `Intel(R) Core(TM) Ultra 9 185H`
- CPU topology: 22 logical CPUs, 16 cores, 2 threads per core
- Build: `cargo build --release`
- Output: stdout redirected to `/dev/null`
- Frames: 100,000
- Runs per mode: 5

| Mode | Command suffix | Median elapsed | Median FPS | Bytes | Avg frame bytes | Max frame bytes | Median throughput | Elapsed range |
| :--- | :--- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Xterm 256-color | `env TERM=xterm-256color ... --benchmark --frames 100000 --no-title --no-clear --no-counter` | 1.202529s | 83,158.09 | 401,883,158 | 4,018.83 | 4,152 | 318.72 MiB/s | 1.131927s-1.223136s |
| TrueColor | `env TERM=xterm-256color ... --benchmark --truecolor --frames 100000 --no-title --no-clear --no-counter` | 1.066388s | 93,774.48 | 520,949,705 | 5,209.50 | 5,424 | 465.89 MiB/s | 1.040087s-1.080270s |
| VT100 40x24 | `env TERM=vt100 ... --benchmark --frames 100000 --width 40 --height 24 --no-title --no-clear --no-counter` | 1.258103s | 79,484.77 | 308,616,555 | 3,086.17 | 3,170 | 233.94 MiB/s | 1.217675s-1.282198s |

## Packaging Checks

- Confirm `Cargo.lock` is committed.
- Confirm `LICENSE` is present.
- Confirm `Cargo.toml` has description, repository, homepage, readme, license, keywords, and categories metadata.
- Confirm the repository does not track `.cargo/config.toml`; release archives must not be compiled with local `target-cpu=native` rustflags.
- Confirm `nyancat.1` documents all public CLI options; `cargo test` also checks README/manpage option names against `OPTION_SPECS`.
- Confirm `systemd/nyancat.socket` and `systemd/nyancat@.service` still reference the intended binary path and socket behavior.
- Confirm `cargo package --list --locked` contains the expected user docs, release scripts, golden smoke fixtures, source files, manpage, and systemd files.
- Confirm the package list excludes local-only files such as `.codex`, `.cargo/config.toml`, and GitHub Actions workflow metadata.
- Confirm release artifacts are built from a clean checkout or a clean working tree; `scripts/release_check.sh` builds a temporary archive and verifies its contents.
- Confirm the GitHub Release contains both platform archives and `SHA256SUMS`.

To build a local binary release archive:

```bash
scripts/release_archive.sh
tar -tzf target/dist/nyancat-vX.Y.Z-<host>.tar.gz
```

The archive contains:

- `bin/nyancat`
- `share/man/man1/nyancat.1`
- `systemd/nyancat.socket` and `systemd/nyancat@.service`
- user docs, release docs, license, changelog, and Cargo manifest files

## Tagging

Before tagging:

```bash
git status --short
git log -1 --oneline
```

After verification, tag with the package version:

```bash
git tag -a vX.Y.Z -m "Release vX.Y.Z"
```

The tag must exactly equal `v` followed by the `Cargo.toml` version. Only push tags after the release artifact and documentation have been reviewed.
