#!/usr/bin/env python3
"""Exercise local PTY resize handling without third-party dependencies."""

import errno
import fcntl
import os
from pathlib import Path
import pty
import select
import signal
import struct
import sys
import termios
import time


INITIAL_ROWS = 24
INITIAL_COLUMNS = 80
RESIZED_ROWS = 10
RESIZED_COLUMNS = 40
FIRST_FRAME_LINES = INITIAL_ROWS - 1
RESIZED_FRAME_LINES = RESIZED_ROWS - 1
TIMEOUT_SECONDS = 5.0

ALTERNATE_SCREEN_ENTRY = b"\x1b[?1049h\x1b[H\x1b[2J\x1b[?25l"
FRAME_PREFIX = b"\x1b[H"
RESIZE_CLEAR = b"\x1b[2J\x1b[H"
RESTORE_SEQUENCE = b"\x1b[?25h\x1b[0m\x1b[?1049l"


class SmokeFailure(RuntimeError):
    """A failed PTY smoke-test assertion."""


def set_window_size(fd, rows, columns):
    size = struct.pack("HHHH", rows, columns, 0, 0)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, size)


def read_until(fd, output, condition, description):
    deadline = time.monotonic() + TIMEOUT_SECONDS
    while not condition(output):
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise SmokeFailure(f"timed out waiting for {description}")

        readable, _, _ = select.select([fd], [], [], remaining)
        if not readable:
            continue

        try:
            chunk = os.read(fd, 65536)
        except OSError as error:
            if error.errno == errno.EIO:
                raise SmokeFailure(f"PTY closed before {description}") from error
            raise
        if not chunk:
            raise SmokeFailure(f"PTY closed before {description}")
        output.extend(chunk)


def initial_frame_complete(output):
    if not output.startswith(ALTERNATE_SCREEN_ENTRY):
        return False
    frame_output = output[len(ALTERNATE_SCREEN_ENTRY) :]
    return frame_output.startswith(FRAME_PREFIX) and frame_output.count(b"\n") >= FIRST_FRAME_LINES


def resized_frame_complete(output):
    marker = output.find(RESIZE_CLEAR)
    if marker < 0:
        return False
    resized_output = output[marker + len(RESIZE_CLEAR) :]
    return resized_output.count(b"\n") >= RESIZED_FRAME_LINES


def drain_output(fd, output):
    deadline = time.monotonic() + TIMEOUT_SECONDS
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise SmokeFailure("timed out waiting for nyancat to exit after the resize")

        readable, _, _ = select.select([fd], [], [], remaining)
        if not readable:
            continue

        try:
            chunk = os.read(fd, 65536)
        except OSError as error:
            # Linux PTY masters commonly report EIO when the final slave closes;
            # macOS returns an empty read instead. Both mean clean end-of-stream.
            if error.errno == errno.EIO:
                return
            raise
        if not chunk:
            return
        output.extend(chunk)


def wait_for_child(pid):
    while True:
        try:
            _, status = os.waitpid(pid, 0)
            return status
        except InterruptedError:
            continue


def status_code(status):
    if os.WIFEXITED(status):
        return os.WEXITSTATUS(status)
    if os.WIFSIGNALED(status):
        return 128 + os.WTERMSIG(status)
    return status


def output_tail(output):
    return repr(bytes(output[-500:]))


def verify_output(output, child_status):
    code = status_code(child_status)
    expected_code = 128 + signal.SIGTERM
    if code != expected_code:
        raise SmokeFailure(
            f"nyancat exited with status {code}, expected {expected_code}; "
            f"output tail: {output_tail(output)}"
        )

    marker_count = output.count(RESIZE_CLEAR)
    if marker_count != 1:
        raise SmokeFailure(
            f"expected one resize clear marker, found {marker_count}; "
            f"output tail: {output_tail(output)}"
        )

    before_resize, after_resize = bytes(output).split(RESIZE_CLEAR, 1)
    if not before_resize.startswith(ALTERNATE_SCREEN_ENTRY):
        raise SmokeFailure("alternate-screen entry was missing before the first frame")

    initial_output = before_resize[len(ALTERNATE_SCREEN_ENTRY) :]
    initial_parts = initial_output.split(FRAME_PREFIX)
    if initial_parts[0] or len(initial_parts) == 1:
        raise SmokeFailure("could not identify the initial 80x24 frame boundaries")

    initial_row_counts = [part.count(b"\n") for part in initial_parts[1:]]
    if any(rows != FIRST_FRAME_LINES for rows in initial_row_counts):
        raise SmokeFailure(
            f"initial 80x24 frames contained {initial_row_counts} lines, "
            f"expected {FIRST_FRAME_LINES} each"
        )

    if RESTORE_SEQUENCE not in after_resize:
        raise SmokeFailure("terminal restore sequence was missing after the resized frame")

    resized_output, _ = after_resize.split(RESTORE_SEQUENCE, 1)
    resized_frame = resized_output.split(FRAME_PREFIX, 1)[0]
    resized_lines = resized_frame.count(b"\n")
    if resized_lines != RESIZED_FRAME_LINES:
        raise SmokeFailure(
            f"resized 40x10 frame contained {resized_lines} lines, "
            f"expected {RESIZED_FRAME_LINES}"
        )


def run(binary):
    release_read, release_write = os.pipe()
    pid, master = pty.fork()

    if pid == 0:
        try:
            os.close(release_write)
            token = os.read(release_read, 1)
            os.close(release_read)
            if token != b"1":
                os._exit(125)

            environment = os.environ.copy()
            environment["TERM"] = "xterm-256color"
            environment["NO_COLOR"] = "1"
            arguments = [
                str(binary),
                "--delay",
                "1000",
                "--no-title",
                "--no-counter",
            ]
            os.execve(str(binary), arguments, environment)
        except BaseException:
            os._exit(126)

    os.close(release_read)
    output = bytearray()
    child_status = None

    try:
        attributes = termios.tcgetattr(master)
        attributes[1] &= ~termios.OPOST
        termios.tcsetattr(master, termios.TCSANOW, attributes)

        # The child waits above so its first TIOCGWINSZ cannot race this setup.
        set_window_size(master, INITIAL_ROWS, INITIAL_COLUMNS)
        os.write(release_write, b"1")
        os.close(release_write)
        release_write = None

        read_until(
            master,
            output,
            initial_frame_complete,
            "a complete initial 80x24 frame",
        )

        # Because pty.fork() created a controlling terminal, changing its size
        # makes the kernel deliver SIGWINCH to nyancat's foreground process.
        set_window_size(master, RESIZED_ROWS, RESIZED_COLUMNS)
        read_until(
            master,
            output,
            resized_frame_complete,
            "a complete resized 40x10 frame",
        )
        os.kill(pid, signal.SIGTERM)
        drain_output(master, output)
        child_status = wait_for_child(pid)
        verify_output(output, child_status)
    finally:
        if release_write is not None:
            os.close(release_write)
        os.close(master)
        if child_status is None:
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            wait_for_child(pid)


def main():
    if len(sys.argv) > 2:
        raise SmokeFailure(f"usage: {Path(sys.argv[0]).name} [nyancat-binary]")

    binary = Path(sys.argv[1] if len(sys.argv) == 2 else "target/release/nyancat").resolve()
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise SmokeFailure(f"nyancat binary is not executable: {binary}")

    run(binary)
    print("PTY resize smoke passed (80x24 -> 40x10 via SIGWINCH)")


if __name__ == "__main__":
    try:
        main()
    except SmokeFailure as error:
        print(f"PTY resize smoke failed: {error}", file=sys.stderr)
        sys.exit(1)
