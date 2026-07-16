use crate::sys;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};

static CLEAR_SCREEN_ON_EXIT: AtomicBool = AtomicBool::new(true);
static RESIZE_PENDING: AtomicBool = AtomicBool::new(false);

fn set_clear_screen_on_exit(clear_screen: bool) {
    CLEAR_SCREEN_ON_EXIT.store(clear_screen, Ordering::Relaxed);
}

pub(crate) fn take_resize_pending() -> bool {
    RESIZE_PENDING.swap(false, Ordering::Relaxed)
}

pub(crate) struct TerminalSession {
    clear_screen: bool,
}

impl TerminalSession {
    pub(crate) fn new(clear_screen: bool) -> io::Result<Self> {
        set_clear_screen_on_exit(clear_screen);
        install_signal_handlers()?;
        Ok(Self { clear_screen })
    }

    pub(crate) fn set_clear_screen(&mut self, clear_screen: bool) {
        self.clear_screen = clear_screen;
        set_clear_screen_on_exit(clear_screen);
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = restore_terminal(self.clear_screen);
    }
}

fn install_signal_handlers() -> io::Result<()> {
    sys::install_signal_handler(sys::Signal::Hangup, handle_exit_signal)?;
    sys::install_signal_handler(sys::Signal::Interrupt, handle_exit_signal)?;
    sys::install_signal_handler(sys::Signal::Pipe, handle_exit_signal)?;
    sys::install_signal_handler(sys::Signal::Terminate, handle_exit_signal)?;
    sys::install_signal_handler(sys::Signal::WindowChanged, handle_resize_signal)
}

extern "C" fn handle_exit_signal(signal_number: i32) {
    let is_broken_pipe = signal_number == sys::Signal::Pipe.number();
    // A restore write to a closed stdout can itself raise SIGPIPE. Ignore it
    // before handling another exit signal so SIGTERM/SIGINT/SIGHUP keep their
    // original status instead of taking the nested SIGPIPE success path.
    if is_broken_pipe || sys::ignore_signal(sys::Signal::Pipe) {
        sys::write_stdout_raw(restore_sequence(
            CLEAR_SCREEN_ON_EXIT.load(Ordering::Relaxed),
        ));
    }
    if is_broken_pipe {
        sys::exit(0);
    }
    if !sys::reraise_with_default(signal_number) {
        sys::exit(128 + signal_number);
    }
}

extern "C" fn handle_resize_signal(_: i32) {
    RESIZE_PENDING.store(true, Ordering::Relaxed);
}

fn restore_terminal(clear_screen: bool) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    stdout.write_all(restore_sequence(clear_screen))?;
    stdout.flush()
}

fn restore_sequence(clear_screen: bool) -> &'static [u8] {
    if clear_screen {
        // Show the cursor, reset attributes, and leave the alternate screen
        // buffer, which restores the terminal contents present before startup.
        b"\x1b[?25h\x1b[0m\x1b[?1049l"
    } else {
        b"\x1b[0m\n"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_sequence_leaves_alternate_screen_when_clearing() {
        assert_eq!(restore_sequence(true), b"\x1b[?25h\x1b[0m\x1b[?1049l");
    }

    #[test]
    fn restore_sequence_preserves_screen_when_requested() {
        assert_eq!(restore_sequence(false), b"\x1b[0m\n");
    }
}
