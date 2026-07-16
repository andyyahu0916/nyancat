# 🐱 Nyancat - Rust Edition

> A fast, colorful Nyancat animation for your terminal, written in Rust.

[![License](https://img.shields.io/badge/license-NCSA-blue.svg)](LICENSE)
[![CI](https://github.com/andyyahu/nyancat-rust/actions/workflows/ci.yml/badge.svg)](https://github.com/andyyahu/nyancat-rust/actions/workflows/ci.yml)

![Nyancat](https://nyancat.dakko.us/nyancat.png)

## ✨ Features

- **High-performance animation** - Written in Rust for speed and efficiency
- **Terminal rendering** - Works in any ANSI-compatible terminal
- **TrueColor support** - Optional 24-bit high-definition rendering mode
- **Benchmark mode** - Zero-delay rendering for performance testing
- **Telnet mode** - Share Nyancat over the network through socket activation or inetd; the animation reflows live when the client window resizes
- **Unix-focused** - CI-verified on Linux and macOS, with BSD-family FFI support
- **Minimal dependencies** - No external crates required

## 🚀 Quick Start

### Build and Run

```bash
# Build the release version
cargo build --release

# Run the animation
./target/release/nyancat
```

Or run directly with Cargo:

```bash
cargo run --release
```

### Serve over telnet

The included systemd socket listens on TCP port 23 on all interfaces by default, and telnet traffic is unencrypted. Before enabling it, bind `ListenStream` to a trusted interface (such as `127.0.0.1:23`) or restrict TCP/23 with your firewall.

```bash
# The -t flag speaks telnet over stdin/stdout; it does not open a listening port.
# Use the included systemd socket files, xinetd, or openbsd-inetd for network access.
sudo cp target/release/nyancat /usr/bin/nyancat
sudo cp systemd/nyancat.socket systemd/nyancat@.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now nyancat.socket
```

Then connect with:

```bash
telnet localhost 23
```

Example systemd service files are included in the `systemd/` directory.
The supplied service expects the executable at `/usr/bin/nyancat`; adjust
`ExecStart` if you install it elsewhere.

## 📦 Installation

### From a release artifact

Download the archive matching your platform and architecture from [GitHub Releases](https://github.com/andyyahu/nyancat-rust/releases). Archives are named `nyancat-vX.Y.Z-<rust-host>.tar.gz`; the executable is under `bin/nyancat` and the man page under `share/man/man1/nyancat.1`. Verify the download against the release's `SHA256SUMS` file before installing it.

### From source

```bash
# Install to $CARGO_HOME/bin (usually ~/.cargo/bin)
cargo install --path . --locked
```

For a local build without installing, run `cargo build --release --locked`; the binary is written to `target/release/nyancat`.

## ⚡ Performance

The Rust edition includes a repeatable benchmark mode for local measurements. Benchmark mode disables frame delay and prints a stable key-value report to stderr when a frame limit is reached.

```bash
cargo build --release
env TERM=xterm-256color target/release/nyancat --benchmark --frames 100000 --no-title --no-clear --no-counter >/dev/null
```

To run the standard benchmark matrix:

```bash
scripts/benchmark_matrix.sh 100000 5
```

The matrix script rebuilds the default release binary before measuring; set `NYANCAT_BIN` only when comparing a specific existing executable.

Example report format:

```text
benchmark: frames=100000 elapsed_s=0.123456 fps=810005.18 bytes=400200000 avg_frame_bytes=4002.00 max_frame_bytes=4002 throughput_mib_s=3091.33
```

Reported values depend on hardware, terminal mode, build profile, and output destination. Redirect stdout to `/dev/null` when measuring rendering throughput instead of terminal drawing speed.

See the benchmark section in [`RELEASE_CHECKLIST.md`](RELEASE_CHECKLIST.md#benchmarking) for recent local snapshots.

## 💻 Usage

```bash
# Run the animation
nyancat

# Run with telnet negotiation on stdin/stdout
nyancat -t

# Run in benchmark mode (0ms delay)
nyancat -b

# Run in high-definition TrueColor mode
nyancat -T
```

### Command Line Options

| Flag | Long Option | Description |
| :--- | :--- | :--- |
| `-i` | `--intro` | Show introduction at startup |
| `-I` | `--skip-intro` | Skip the introduction in telnet mode |
| `-t` | `--telnet` | Speak Telnet over stdin/stdout; does not open a port |
| `-T` | `--truecolor` | Enable 24-bit TrueColor rendering |
| `-n` | `--no-counter` | Do not display the timer |
| `-s` | `--no-title` | Do not set titlebar text |
| `-e` | `--no-clear` | Do not clear screen between frames |
| `-b` | `--benchmark` | Run with 0ms delay (Warning: high CPU) |
| `-d` | `--delay` | Set delay (10ms - 1000ms) |
| `-f` | `--frames` | Quit after a positive number of frames |
| `-r` | `--min-rows` | Crop the animation from the top (-10000 to 10000) |
| `-R` | `--max-rows` | Crop the animation from the bottom (-10000 to 10000) |
| `-c` | `--min-cols` | Crop the animation from the left (-10000 to 10000) |
| `-C` | `--max-cols` | Crop the animation from the right (-10000 to 10000) |
| `-W` | `--width` | Set animation width (1 to 10000) |
| `-H` | `--height` | Set animation height (1 to 10000) |
| `-h` | `--help` | Show help message |
| `-V` | `--version` | Show version information |

### Environment

- `NO_COLOR` — when set to a non-empty value, nyancat renders in monochrome ASCII and omits styling from help and diagnostics (honoring [no-color.org](https://no-color.org)). Render-mode selection does not apply to telnet sessions, where color is the connecting client's concern.

## 🔧 Development

### Prerequisites

- Rust 1.85+ (2024 edition)

### Build

```bash
cargo build --release
```

### Verification

```bash
scripts/release_check.sh
scripts/benchmark_matrix.sh 100000 5
scripts/release_archive.sh
```

### Project structure

- `src/main.rs` - Startup orchestration
- `src/animation.rs` - Frame data
- `src/cli.rs` - Command-line parsing and configuration
- `src/render.rs` - Render orchestration and frame composition
- `src/render/` - Palette lookup, frame buffer, render loop timing, and benchmark accounting
- `src/telnet.rs` - Telnet negotiation
- `src/terminal.rs` - Terminal size and type detection
- `src/runtime.rs` - Exit and signal handling
- `src/sys.rs` - Unix FFI bindings
- `scripts/` - Release verification and benchmark helpers
- `systemd/` - Systemd service files for telnet server integration

### Engineering docs

- [`ARCHITECTURE.md`](ARCHITECTURE.md) documents module boundaries, data flow, runtime policy, and extension guidelines.
- [`RELEASE_CHECKLIST.md`](RELEASE_CHECKLIST.md) defines the verification baseline and benchmark snapshot record for release candidates.
- [`ROADMAP.md`](ROADMAP.md) tracks engineering priorities and completed rustification milestones.

## Credits

- **Original Nyancat animation**: [prguitarman](http://www.prguitarman.com/index.php?id=348)
- **Original implementation**: [Kevin Lange (klange)](https://github.com/klange/nyancat)
- **Original project website**: [nyancat.dakko.us](https://nyancat.dakko.us/)
- **Rust rewrite**: This project

## 📜 License

Licensed under the [NCSA License](LICENSE).
