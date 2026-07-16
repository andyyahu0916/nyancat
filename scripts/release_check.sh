#!/usr/bin/env sh
set -eu

BIN=${NYANCAT_BIN:-target/release/nyancat}
TMP=${TMPDIR:-/tmp}
GOLDEN_DIR=${GOLDEN_DIR:-tests/golden}

work_dir=$(mktemp -d "$TMP/nyancat-rust-release-check.XXXXXX")
normal_out="$work_dir/normal.out"
clear_out="$work_dir/clear.out"
signal_out="$work_dir/signal.out"
telnet_out="$work_dir/telnet.out"
truecolor_out="$work_dir/truecolor.out"
crop_out="$work_dir/crop.out"
benchmark_out="$work_dir/benchmark.out"
benchmark_err="$work_dir/benchmark.err"
cli_err="$work_dir/cli-error.err"
frames_err="$work_dir/frames-error.err"
flag_value_err="$work_dir/flag-value-error.err"
crop_err="$work_dir/crop-error.err"
write_err="$work_dir/write-error.err"
telnet_write_err="$work_dir/telnet-write-error.err"
help_out="$work_dir/help.out"
package_list="$work_dir/package-list.out"
archive_log="$work_dir/release-archive.log"
archive_list="$work_dir/release-archive-list.out"
archive_dir="$work_dir/archive"

cleanup() {
    if [ -n "${signal_pid:-}" ]; then
        kill "$signal_pid" 2> /dev/null || true
    fi
    if [ -n "${pipe_signal_pid:-}" ]; then
        kill -CONT "$pipe_signal_pid" 2> /dev/null || true
        kill -KILL "$pipe_signal_pid" 2> /dev/null || true
    fi
    if [ -n "${pipe_reader_pid:-}" ]; then
        kill "$pipe_reader_pid" 2> /dev/null || true
    fi
    rm -rf "$work_dir"
}

trap cleanup EXIT HUP INT TERM

check_golden() {
    file=$1
    golden=$2
    label=$3
    if [ ! -f "$golden" ]; then
        echo "missing golden $golden for $label; regenerate with scripts/update_goldens.sh" >&2
        exit 1
    fi
    if ! cmp -s "$golden" "$file"; then
        echo "golden mismatch for $label:" >&2
        cmp "$golden" "$file" >&2 || true
        echo "if this output change is intentional, regenerate with scripts/update_goldens.sh and review the git diff" >&2
        exit 1
    fi
}

check_contains() {
    file=$1
    pattern=$2
    label=$3
    if ! LC_ALL=C grep -F "$pattern" "$file" > /dev/null; then
        echo "missing $label in $file" >&2
        exit 1
    fi
}

check_absent() {
    file=$1
    pattern=$2
    label=$3
    if LC_ALL=C grep -F "$pattern" "$file" > /dev/null; then
        echo "unexpected $label in $file" >&2
        exit 1
    fi
}

check_char_count() {
    file=$1
    chars=$2
    expected=$3
    label=$4
    actual=$(LC_ALL=C tr -cd "$chars" < "$file" | wc -c | tr -d ' ')
    if [ "$actual" != "$expected" ]; then
        echo "character count mismatch for $label in $file: expected $expected, got $actual" >&2
        exit 1
    fi
}

echo "== cargo fmt --check =="
cargo fmt --check

echo "== cargo test =="
cargo test --locked

echo "== cargo clippy =="
cargo clippy --locked --all-targets --all-features -- -D warnings

echo "== cargo build --release =="
cargo build --release --locked

echo "== shell script syntax =="
sh -n scripts/benchmark_matrix.sh
sh -n scripts/benchmark_callgrind.sh
sh -n scripts/release_archive.sh
sh -n scripts/update_goldens.sh

echo "== cargo package list =="
cargo package --list --allow-dirty --locked > "$package_list"
check_contains "$package_list" "Cargo.toml" "package manifest"
check_contains "$package_list" "README.md" "package readme"
check_contains "$package_list" "src/main.rs" "package source"
check_contains "$package_list" "scripts/benchmark_matrix.sh" "package benchmark matrix"
check_contains "$package_list" "scripts/benchmark_callgrind.sh" "package callgrind benchmark"
check_contains "$package_list" "scripts/release_archive.sh" "package release archive helper"
check_contains "$package_list" "scripts/release_check.sh" "package release check"
check_contains "$package_list" "scripts/update_goldens.sh" "package goldens updater"
check_contains "$package_list" "tests/golden/normal.out" "package normal golden"
check_contains "$package_list" "tests/golden/telnet.out" "package telnet golden"
check_contains "$package_list" "tests/golden/truecolor.out" "package truecolor golden"
check_contains "$package_list" "tests/golden/crop.out" "package crop golden"
check_contains "$package_list" "tests/golden/benchmark.out" "package benchmark golden"
check_contains "$package_list" "nyancat.1" "package manpage"
check_contains "$package_list" "systemd/nyancat.socket" "package systemd socket"
check_absent "$package_list" ".codex" "codex state file"
check_absent "$package_list" ".cargo/config.toml" "local cargo config"
check_absent "$package_list" ".github/workflows/ci.yml" "CI workflow"
check_absent "$package_list" ".github/workflows/release.yml" "release workflow"

echo "== release archive =="
DIST_DIR="$archive_dir" scripts/release_archive.sh > "$archive_log"
archive_path=$(sed -n 's/^release archive: //p' "$archive_log")
if [ ! -f "$archive_path" ]; then
    echo "release archive was not created: $archive_path" >&2
    exit 1
fi

tar -tzf "$archive_path" > "$archive_list"
check_contains "$archive_list" "/bin/nyancat" "archive binary"
check_contains "$archive_list" "/share/man/man1/nyancat.1" "archive manpage"
check_contains "$archive_list" "/systemd/nyancat.socket" "archive systemd socket"
check_contains "$archive_list" "/systemd/nyancat@.service" "archive systemd service"
check_contains "$archive_list" "/docs/RELEASE_CHECKLIST.md" "archive release checklist"
check_contains "$archive_list" "/README.md" "archive README"
check_contains "$archive_list" "/LICENSE" "archive license"
check_absent "$archive_list" ".staging" "archive staging directory"

echo "== smoke tests =="
env NO_COLOR= TERM=xterm-256color "$BIN" --frames 1 --no-title --no-clear --no-counter > "$normal_out"
env NO_COLOR= TERM=xterm-256color "$BIN" --frames 1 --no-title --no-counter > "$clear_out"
"$BIN" --telnet --skip-intro --frames 1 --no-title --no-clear --no-counter > "$telnet_out"
env NO_COLOR= TERM=xterm-256color "$BIN" --truecolor --frames 1 --no-title --no-clear --no-counter > "$truecolor_out"
env NO_COLOR= TERM=xterm-256color "$BIN" --frames 1 --width 40 --height 24 --no-title --no-clear --no-counter > "$crop_out"
env NO_COLOR= TERM=xterm-256color "$BIN" --benchmark --frames 3 --no-title --no-clear --no-counter > "$benchmark_out" 2> "$benchmark_err"
"$BIN" --help > "$help_out"

check_golden "$normal_out" "$GOLDEN_DIR/normal.out" "normal output"
check_golden "$telnet_out" "$GOLDEN_DIR/telnet.out" "telnet output"
check_golden "$truecolor_out" "$GOLDEN_DIR/truecolor.out" "truecolor output"
check_golden "$crop_out" "$GOLDEN_DIR/crop.out" "crop output"
check_golden "$benchmark_out" "$GOLDEN_DIR/benchmark.out" "benchmark output"

esc=$(printf '\033')
telnet_iac_wont_echo=$(printf '\377\374\001')

check_contains "$normal_out" "${esc}[s${esc}[u${esc}[48;5;17m" "xterm frame prefix"
check_contains "$normal_out" "${esc}[48;5;196m" "xterm rainbow red"
check_absent "$normal_out" "${esc}[48;2;" "truecolor escape sequence"
check_absent "$normal_out" "You have nyaned for" "counter text"

check_contains "$clear_out" "${esc}[?1049h${esc}[H${esc}[2J${esc}[?25l" "alternate-screen entry"
check_contains "$clear_out" "${esc}[?25h${esc}[0m${esc}[?1049l" "alternate-screen restore"

env NO_COLOR= TERM=xterm-256color "$BIN" --no-title --no-counter > "$signal_out" &
signal_pid=$!
sleep 1
kill -TERM "$signal_pid"
set +e
wait "$signal_pid"
signal_status=$?
set -e
signal_pid=
if [ "$signal_status" -ne 143 ]; then
    echo "SIGTERM smoke exited $signal_status instead of 143" >&2
    exit 1
fi
check_contains "$signal_out" "${esc}[?25h${esc}[0m${esc}[?1049l" "signal-path terminal restore"

# Closing stdout while another exit signal is pending must not let a nested
# SIGPIPE replace the original signal's status. Stop the writer before closing
# the FIFO reader so the SIGTERM is deterministically delivered first.
pipe_signal_dir="$work_dir/pipe-signal"
mkdir "$pipe_signal_dir"
mkfifo "$pipe_signal_dir/stdout"
cat "$pipe_signal_dir/stdout" > /dev/null &
pipe_reader_pid=$!
env NO_COLOR= TERM=xterm-256color "$BIN" --no-title --no-counter > "$pipe_signal_dir/stdout" &
pipe_signal_pid=$!
sleep 1
kill -STOP "$pipe_signal_pid"
kill -TERM "$pipe_reader_pid"
set +e
wait "$pipe_reader_pid"
set -e
pipe_reader_pid=
kill -TERM "$pipe_signal_pid"
kill -CONT "$pipe_signal_pid"
set +e
wait "$pipe_signal_pid"
pipe_signal_status=$?
set -e
pipe_signal_pid=
if [ "$pipe_signal_status" -ne 143 ]; then
    echo "closed-pipe SIGTERM smoke exited $pipe_signal_status instead of 143" >&2
    exit 1
fi

check_contains "$truecolor_out" "${esc}[s${esc}[u${esc}[48;2;0;49;105m" "truecolor frame prefix"
check_contains "$truecolor_out" "${esc}[48;2;255;25;0m" "truecolor rainbow red"
check_absent "$truecolor_out" "${esc}[48;5;" "256-color escape sequence"

check_contains "$crop_out" "${esc}[s${esc}[u${esc}[48;5;17m" "cropped frame prefix"
check_absent "$benchmark_out" "You have nyaned for" "benchmark counter text"
check_absent "$benchmark_err" "$esc" "terminal styling on redirected benchmark stderr"
check_absent "$help_out" "$esc" "terminal styling on redirected help output"

check_contains "$telnet_out" "$telnet_iac_wont_echo" "telnet negotiation prefix"
check_contains "$telnet_out" "${esc}[s${esc}[u${esc}[104m" "telnet ANSI frame prefix"
check_char_count "$normal_out" '\000' 0 "normal NUL bytes"
check_char_count "$telnet_out" '\000' 23 "telnet NUL newline bytes"
check_char_count "$telnet_out" '\015' 23 "telnet CR newline bytes"

nocolor_out="$work_dir/nocolor.out"
env NO_COLOR=1 TERM=xterm-256color "$BIN" --frames 1 --no-title --no-clear --no-counter > "$nocolor_out"
check_absent "$nocolor_out" "${esc}[48" "color background under NO_COLOR"
check_contains "$nocolor_out" "::" "NO_COLOR ascii output"

if "$BIN" --wat > "$cli_err" 2>&1; then
    echo "expected CLI error smoke to fail" >&2
    exit 1
fi

if "$BIN" --frames=-1 > "$frames_err" 2>&1; then
    echo "expected negative frame count smoke to fail" >&2
    exit 1
fi

if "$BIN" --no-counter=false > "$flag_value_err" 2>&1; then
    echo "expected flag value smoke to fail" >&2
    exit 1
fi

if "$BIN" --width=10001 > "$crop_err" 2>&1; then
    echo "expected crop bounds smoke to fail" >&2
    exit 1
fi

grep -F "nyancat: unknown option: --wat" "$cli_err" > /dev/null
grep -F "Try '$BIN --help' for usage." "$cli_err" > /dev/null
grep -F "nyancat: value for --frames must be positive: -1" "$frames_err" > /dev/null
grep -F "nyancat: unexpected value for --no-counter: false" "$flag_value_err" > /dev/null
grep -F "nyancat: value for --width out of range: 10001 (expected 1-10000)" "$crop_err" > /dev/null
grep -F "benchmark: frames=3" "$benchmark_err" > /dev/null
grep -F "usage: $BIN" "$help_out" > /dev/null
grep -F -- "-i, --intro" "$help_out" > /dev/null
grep -F -- "-I, --skip-intro" "$help_out" > /dev/null
grep -F -- "-t, --telnet" "$help_out" > /dev/null
grep -F -- "-T, --truecolor" "$help_out" > /dev/null
grep -F -- "-n, --no-counter" "$help_out" > /dev/null
grep -F -- "-s, --no-title" "$help_out" > /dev/null
grep -F -- "-e, --no-clear" "$help_out" > /dev/null
grep -F -- "-b, --benchmark" "$help_out" > /dev/null
grep -F -- "-d, --delay <ms>" "$help_out" > /dev/null
grep -F -- "-f, --frames <frames>" "$help_out" > /dev/null
grep -F -- "-r, --min-rows <row>" "$help_out" > /dev/null
grep -F -- "-R, --max-rows <row>" "$help_out" > /dev/null
grep -F -- "-c, --min-cols <col>" "$help_out" > /dev/null
grep -F -- "-C, --max-cols <col>" "$help_out" > /dev/null
grep -F -- "-W, --width <width>" "$help_out" > /dev/null
grep -F -- "-H, --height <height>" "$help_out" > /dev/null
grep -F -- "-h, --help" "$help_out" > /dev/null
grep -F -- "-V, --version" "$help_out" > /dev/null

# A genuine (non-broken-pipe) write failure must exit non-zero and report the error.
# /dev/full always fails writes with ENOSPC; it only exists on Linux, so guard on it.
if [ -c /dev/full ]; then
    if env NO_COLOR= TERM=xterm-256color "$BIN" --frames 1 --no-title --no-clear --no-counter > /dev/full 2> "$write_err"; then
        echo "expected write failure smoke to exit non-zero" >&2
        exit 1
    fi
    grep -F "nyancat: " "$write_err" > /dev/null

    if "$BIN" --telnet < /dev/null > /dev/full 2> "$telnet_write_err"; then
        echo "expected telnet negotiation write failure to exit non-zero" >&2
        exit 1
    fi
    grep -F "nyancat: " "$telnet_write_err" > /dev/null
fi

echo "release check passed"
