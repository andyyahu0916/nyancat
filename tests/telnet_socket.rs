#![cfg(unix)]

use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, Shutdown, TcpListener, TcpStream};
use std::os::fd::OwnedFd;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const IAC: u8 = 255;
const SB: u8 = 250;
const WILL: u8 = 251;
const SE: u8 = 240;
const TTYPE: u8 = 24;
const NAWS: u8 = 31;
const FRAME_PREFIX: &[u8] = b"\x1b[u";
const TELNET_NEWLINE: &[u8] = b"\r\0\n";
const TEST_TIMEOUT: Duration = Duration::from_secs(10);
const READ_POLL_INTERVAL: Duration = Duration::from_millis(100);

struct ChildGuard {
    child: Child,
}

impl ChildGuard {
    fn wait_before(&mut self, deadline: Instant) -> io::Result<(ExitStatus, Vec<u8>)> {
        loop {
            if let Some(status) = self.child.try_wait()? {
                let mut stderr = Vec::new();
                if let Some(mut pipe) = self.child.stderr.take() {
                    pipe.read_to_end(&mut stderr)?;
                }
                return Ok((status, stderr));
            }

            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return Err(timeout_error(
                    "timed out waiting for the telnet child to exit",
                ));
            };
            thread::sleep(remaining.min(Duration::from_millis(10)));
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn spawn_telnet() -> io::Result<(ChildGuard, TcpStream)> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let client = TcpStream::connect(listener.local_addr()?)?;
    let (server, _) = listener.accept()?;

    client.set_nodelay(true)?;
    client.set_read_timeout(Some(READ_POLL_INTERVAL))?;
    client.set_write_timeout(Some(TEST_TIMEOUT))?;
    server.set_nodelay(true)?;

    // Rust 1.85's Stdio converts from OwnedFd, not directly from TcpStream.
    let child_stdin: OwnedFd = server.try_clone()?.into();
    let child_stdout: OwnedFd = server.into();
    let mut command = Command::new(env!("CARGO_BIN_EXE_nyancat"));
    command.args([
        "--telnet",
        "--skip-intro",
        "--delay",
        "100",
        "--no-title",
        "--no-clear",
        "--no-counter",
    ]);

    let child = command
        .stdin(Stdio::from(child_stdin))
        .stdout(Stdio::from(child_stdout))
        .stderr(Stdio::piped())
        .spawn()?;

    Ok((ChildGuard { child }, client))
}

fn terminal_type(term: &[u8]) -> Vec<u8> {
    let mut bytes = vec![IAC, WILL, TTYPE, IAC, SB, TTYPE, 0];
    bytes.extend_from_slice(term);
    bytes.extend_from_slice(&[IAC, SE]);
    bytes
}

fn window_size(width: u16, height: u16) -> Vec<u8> {
    let [width_hi, width_lo] = width.to_be_bytes();
    let [height_hi, height_lo] = height.to_be_bytes();
    vec![
        IAC, WILL, NAWS, IAC, SB, NAWS, width_hi, width_lo, height_hi, height_lo, IAC, SE,
    ]
}

fn initial_handshake() -> Vec<u8> {
    let mut bytes = terminal_type(b"xterm");
    bytes.extend(window_size(80, 24));
    bytes
}

fn read_until_contains(
    stream: &mut TcpStream,
    output: &mut Vec<u8>,
    needle: &[u8],
    deadline: Instant,
) -> io::Result<()> {
    while !contains(output, needle) {
        if !read_chunk_before(stream, output, deadline)? {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "telnet session closed before the expected output arrived",
            ));
        }
    }
    Ok(())
}

fn read_until_completed_frame_with_rows(
    stream: &mut TcpStream,
    output: &mut Vec<u8>,
    expected_rows: usize,
    deadline: Instant,
) -> io::Result<()> {
    while !completed_frame_row_counts(output).contains(&expected_rows) {
        if !read_chunk_before(stream, output, deadline)? {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "telnet session closed before the resized frame completed",
            ));
        }
    }
    Ok(())
}

fn drain_before(stream: &mut TcpStream, output: &mut Vec<u8>, deadline: Instant) -> io::Result<()> {
    while read_chunk_before(stream, output, deadline)? {}
    Ok(())
}

fn read_chunk_before(
    stream: &mut TcpStream,
    output: &mut Vec<u8>,
    deadline: Instant,
) -> io::Result<bool> {
    let mut buffer = [0; 4096];
    loop {
        if Instant::now() >= deadline {
            return Err(timeout_error("timed out reading from the telnet child"));
        }

        match stream.read(&mut buffer) {
            Ok(0) => return Ok(false),
            Ok(count) => {
                output.extend_from_slice(&buffer[..count]);
                return Ok(true);
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::Interrupted
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::WouldBlock
                ) => {}
            Err(error) => return Err(error),
        }
    }
}

fn timeout_error(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, message)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn occurrence_offsets(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
    haystack
        .windows(needle.len())
        .enumerate()
        .filter_map(|(offset, window)| (window == needle).then_some(offset))
        .collect()
}

fn count_occurrences(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

fn completed_frame_row_counts(output: &[u8]) -> Vec<usize> {
    let starts = occurrence_offsets(output, FRAME_PREFIX);
    starts
        .windows(2)
        .map(|frame| {
            let [start, end] = frame else {
                unreachable!("two-element frame window")
            };
            count_occurrences(&output[*start..*end], TELNET_NEWLINE)
        })
        .collect()
}

#[test]
fn mid_session_naws_reflows_over_tcp() -> io::Result<()> {
    let deadline = Instant::now() + TEST_TIMEOUT;
    let (mut child, mut client) = spawn_telnet()?;
    client.write_all(&initial_handshake())?;

    let mut output = Vec::new();
    read_until_contains(&mut client, &mut output, FRAME_PREFIX, deadline)?;

    // Send this only after rendering starts. If it accompanied the initial
    // handshake, the negotiation reader could prefetch and then discard it.
    client.write_all(&window_size(40, 10))?;
    read_until_completed_frame_with_rows(&mut client, &mut output, 9, deadline)?;
    client.shutdown(Shutdown::Write)?;
    drain_before(&mut client, &mut output, deadline)?;

    let (status, stderr) = child.wait_before(deadline)?;
    assert!(
        status.success(),
        "telnet child exited with {status}: {}",
        String::from_utf8_lossy(&stderr)
    );
    assert!(
        stderr.is_empty(),
        "unexpected telnet stderr: {}",
        String::from_utf8_lossy(&stderr)
    );

    let row_counts = completed_frame_row_counts(&output);
    assert_eq!(row_counts[0], 23, "initial 80x24 frame was not rendered");
    assert!(
        row_counts.contains(&9),
        "40x10 NAWS update did not reflow a later frame: {row_counts:?}"
    );

    Ok(())
}

#[test]
fn client_half_close_ends_unbounded_session_cleanly() -> io::Result<()> {
    let deadline = Instant::now() + TEST_TIMEOUT;
    let (mut child, mut client) = spawn_telnet()?;
    client.write_all(&initial_handshake())?;

    let mut output = Vec::new();
    read_until_contains(&mut client, &mut output, FRAME_PREFIX, deadline)?;
    client.shutdown(Shutdown::Write)?;
    drain_before(&mut client, &mut output, deadline)?;

    let (status, stderr) = child.wait_before(deadline)?;
    assert!(
        status.success(),
        "telnet child exited with {status}: {}",
        String::from_utf8_lossy(&stderr)
    );
    assert!(
        stderr.is_empty(),
        "unexpected telnet stderr: {}",
        String::from_utf8_lossy(&stderr)
    );
    assert!(
        contains(&output, FRAME_PREFIX),
        "client disconnected before receiving a frame"
    );

    Ok(())
}
