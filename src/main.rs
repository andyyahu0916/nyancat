// All FFI and unsafe code is confined to sys.rs; the compiler enforces it.
#![deny(unsafe_code)]

mod animation;
mod cli;
mod render;
mod runtime;
#[allow(unsafe_code)]
mod sys;
mod telnet;
mod terminal;

use cli::{CliAction, parse_args, print_usage, print_version};
use render::{Palette, RenderState, RunOutcome, run};
use runtime::TerminalSession;
use std::env;
use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;
use std::time::Duration;
use telnet::negotiate_telnet;
use terminal::{TerminalType, detect_terminal_type, terminal_size};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let mut config = match parse_args(&args) {
        Ok(CliAction::Run(config)) => config,
        Ok(CliAction::Help { program }) => {
            print_usage(
                &program,
                io::stdout().is_terminal() && !no_color_requested(),
            );
            return ExitCode::SUCCESS;
        }
        Ok(CliAction::Version) => {
            print_version();
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            let program = args.first().map_or("nyancat", String::as_str);
            let _ = writeln!(io::stderr(), "nyancat: {error}");
            let _ = writeln!(io::stderr(), "Try '{program} --help' for usage.");
            return ExitCode::FAILURE;
        }
    };

    if config.benchmark {
        config.delay = Duration::ZERO;
        let warning = if io::stderr().is_terminal() && !no_color_requested() {
            "\x1b[1;33mWARNING:\x1b[0m"
        } else {
            "WARNING:"
        };
        let _ = writeln!(
            io::stderr(),
            "{warning} Benchmark mode enabled. Delay set to 0ms; use --frames for a completion report."
        );
    }

    if config.telnet && !config.skip_intro {
        config.show_intro = true;
    }
    let telnet_mode = config.telnet;

    let (term, mut terminal_size) = if config.telnet {
        let mut stdout = io::stdout().lock();
        let info = match negotiate_telnet(&mut stdout) {
            Ok(info) => info,
            Err(error) => {
                let clean_disconnect = is_clean_disconnect(&error, true);
                drop(stdout);
                if clean_disconnect {
                    return ExitCode::SUCCESS;
                }
                let _ = writeln!(io::stderr(), "nyancat: {error}");
                return ExitCode::FAILURE;
            }
        };
        (info.term, info.size.unwrap_or_default())
    } else {
        (env::var("TERM").ok(), terminal_size())
    };

    let mut terminal_type = detect_terminal_type(term.as_deref(), terminal_size);
    if config.truecolor {
        terminal_type = TerminalType::TrueColor;
    }
    // Honor NO_COLOR (https://no-color.org) outside telnet, where colour is the
    // remote client's concern rather than this process's environment.
    if !config.telnet && no_color_requested() {
        terminal_type = TerminalType::Vt220;
    }
    if terminal_type == TerminalType::Vt100Ascii {
        terminal_size = terminal_size.with_width(40);
    }

    let palette = Palette::new(terminal_type);
    let state = RenderState::new(&config, terminal_size);
    let mut terminal_session = match TerminalSession::new(config.clear_screen) {
        Ok(session) => session,
        Err(error) => {
            let _ = writeln!(
                io::stderr(),
                "nyancat: could not install signal handlers: {error}"
            );
            return ExitCode::FAILURE;
        }
    };

    let mut benchmark_report = None;
    let mut run_error = None;

    let exit_code = match run(config, state, palette) {
        Ok(RunOutcome::Finished {
            clear_screen,
            benchmark,
        }) => {
            terminal_session.set_clear_screen(clear_screen);
            benchmark_report = benchmark;
            ExitCode::SUCCESS
        }
        Err(error) => {
            if is_clean_disconnect(&error, telnet_mode) {
                ExitCode::SUCCESS
            } else {
                run_error = Some(error);
                ExitCode::FAILURE
            }
        }
    };

    drop(terminal_session);

    if let Some(error) = run_error {
        let _ = writeln!(io::stderr(), "nyancat: {error}");
    }
    if let Some(report) = benchmark_report {
        let _ = writeln!(io::stderr(), "{report}");
    }

    exit_code
}

fn no_color_requested() -> bool {
    env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty())
}

fn is_clean_disconnect(error: &io::Error, telnet_mode: bool) -> bool {
    error.kind() == io::ErrorKind::BrokenPipe
        || telnet_mode
            && matches!(
                error.kind(),
                io::ErrorKind::ConnectionReset
                    | io::ErrorKind::ConnectionAborted
                    | io::ErrorKind::NotConnected
                    | io::ErrorKind::UnexpectedEof
            )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broken_pipe_is_always_a_clean_disconnect() {
        let error = io::Error::new(io::ErrorKind::BrokenPipe, "closed pipe");

        assert!(is_clean_disconnect(&error, false));
        assert!(is_clean_disconnect(&error, true));
    }

    #[test]
    fn reset_connection_is_clean_only_in_telnet_mode() {
        let error = io::Error::new(io::ErrorKind::ConnectionReset, "reset connection");

        assert!(!is_clean_disconnect(&error, false));
        assert!(is_clean_disconnect(&error, true));
    }

    #[test]
    fn unrelated_errors_are_never_clean_disconnects() {
        let error = io::Error::other("unrelated output failure");

        assert!(!is_clean_disconnect(&error, false));
        assert!(!is_clean_disconnect(&error, true));
    }
}
