use std::ffi::c_void;
use std::io;
use std::time::Duration;

const SIGHUP: i32 = 1;
const SIGINT: i32 = 2;
const SIGPIPE: i32 = 13;
const SIGTERM: i32 = 15;
const SIGWINCH: i32 = 28;
const POLLIN: i16 = 0x001;
const POLLERR: i16 = 0x008;
const POLLHUP: i16 = 0x010;
const POLLNVAL: i16 = 0x020;
const SIG_DFL: usize = 0;
const SIG_IGN: usize = 1;
const SIG_ERR: usize = usize::MAX;

// Linux defines nfds_t as unsigned long; Android and the supported BSD-family
// targets define it as unsigned int. Keep the raw declaration ABI-accurate
// without adding a runtime dependency solely for this alias.
#[cfg(target_os = "linux")]
type PollNfds = usize;

#[cfg(any(
    target_os = "android",
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]
type PollNfds = u32;

#[cfg(any(target_os = "linux", target_os = "android"))]
const TIOCGWINSZ: usize = 0x5413;

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]
const TIOCGWINSZ: usize = 0x4008_7468;

pub type SignalHandler = extern "C" fn(i32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PollReadiness {
    Ready,
    Timeout,
    Interrupted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StdinRead {
    Bytes(usize),
    Eof,
    Interrupted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Signal {
    Hangup,
    Interrupt,
    Pipe,
    Terminate,
    WindowChanged,
}

impl Signal {
    pub const fn number(self) -> i32 {
        match self {
            Self::Hangup => SIGHUP,
            Self::Interrupt => SIGINT,
            Self::Pipe => SIGPIPE,
            Self::Terminate => SIGTERM,
            Self::WindowChanged => SIGWINCH,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PollTimeout(i32);

impl PollTimeout {
    pub fn from_duration(duration: Duration) -> Self {
        Self(duration.as_millis().min(i32::MAX as u128) as i32)
    }

    const fn as_raw(self) -> i32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Fd(i32);

impl Fd {
    const STDIN: Self = Self(0);
    const STDOUT: Self = Self(1);

    const fn as_raw(self) -> i32 {
        self.0
    }
}

#[repr(C)]
struct Winsize {
    ws_row: u16,
    ws_col: u16,
    ws_xpixel: u16,
    ws_ypixel: u16,
}

#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

unsafe extern "C" {
    fn ioctl(fd: i32, request: usize, ...) -> i32;
    fn poll(fds: *mut PollFd, nfds: PollNfds, timeout: i32) -> i32;
    fn signal(signum: i32, handler: usize) -> usize;
    fn raise(signum: i32) -> i32;
    fn write(fd: i32, buf: *const c_void, count: usize) -> isize;
    fn read(fd: i32, buf: *mut c_void, count: usize) -> isize;
    fn _exit(status: i32) -> !;
}

pub fn install_signal_handler(signal_kind: Signal, handler: SignalHandler) -> io::Result<()> {
    let previous = unsafe { signal(signal_kind.number(), handler as *const () as usize) };
    if previous == SIG_ERR {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn stdout_terminal_size() -> Option<(i32, i32)> {
    let mut winsize = Winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };

    let rc = unsafe { ioctl(Fd::STDOUT.as_raw(), TIOCGWINSZ, &mut winsize) };
    if rc == 0 && winsize.ws_col > 0 && winsize.ws_row > 0 {
        Some((winsize.ws_col as i32, winsize.ws_row as i32))
    } else {
        None
    }
}

pub fn stdin_readiness(timeout: PollTimeout) -> io::Result<PollReadiness> {
    let mut fds = PollFd {
        fd: Fd::STDIN.as_raw(),
        events: POLLIN,
        revents: 0,
    };

    let rc = unsafe { poll(&mut fds, 1, timeout.as_raw()) };
    if rc > 0 {
        if (fds.revents & POLLNVAL) != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "stdin is not a valid poll fd",
            ));
        }
        if (fds.revents & (POLLIN | POLLHUP | POLLERR)) != 0 {
            Ok(PollReadiness::Ready)
        } else {
            Ok(PollReadiness::Timeout)
        }
    } else if rc == 0 {
        Ok(PollReadiness::Timeout)
    } else {
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::Interrupted {
            Ok(PollReadiness::Interrupted)
        } else {
            Err(err)
        }
    }
}

pub fn read_stdin(buffer: &mut [u8]) -> io::Result<StdinRead> {
    let bytes_read = unsafe { read(Fd::STDIN.as_raw(), buffer.as_mut_ptr().cast(), buffer.len()) };
    if bytes_read > 0 {
        Ok(StdinRead::Bytes(bytes_read as usize))
    } else if bytes_read == 0 {
        Ok(StdinRead::Eof)
    } else {
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::Interrupted {
            Ok(StdinRead::Interrupted)
        } else {
            Err(err)
        }
    }
}

pub fn write_stdout_raw(data: &[u8]) {
    let mut remaining = data;
    while !remaining.is_empty() {
        let written = unsafe {
            write(
                Fd::STDOUT.as_raw(),
                remaining.as_ptr().cast(),
                remaining.len(),
            )
        };
        if written <= 0 {
            break;
        }
        remaining = &remaining[written as usize..];
    }
}

pub fn ignore_signal(signal_kind: Signal) -> bool {
    let previous = unsafe { signal(signal_kind.number(), SIG_IGN) };
    previous != SIG_ERR
}

/// Restores the default action and queues the signal again. The caller must
/// return from its handler so the currently blocked signal can be delivered.
pub fn reraise_with_default(signal_number: i32) -> bool {
    let previous = unsafe { signal(signal_number, SIG_DFL) };
    previous != SIG_ERR && unsafe { raise(signal_number) } == 0
}

pub fn exit(status: i32) -> ! {
    unsafe { _exit(status) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_numbers_match_existing_unix_values() {
        assert_eq!(Signal::Hangup.number(), SIGHUP);
        assert_eq!(Signal::Interrupt.number(), SIGINT);
        assert_eq!(Signal::Pipe.number(), SIGPIPE);
        assert_eq!(Signal::Terminate.number(), SIGTERM);
        assert_eq!(Signal::WindowChanged.number(), SIGWINCH);
    }

    #[test]
    fn poll_timeout_clamps_to_platform_limit() {
        assert_eq!(
            PollTimeout::from_duration(Duration::from_millis(250)).as_raw(),
            250
        );
        assert_eq!(
            PollTimeout::from_duration(Duration::from_millis(i32::MAX as u64 + 1)).as_raw(),
            i32::MAX
        );
    }
}
