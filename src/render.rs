mod benchmark;
mod frame_buffer;
mod palette;
mod render_loop;

use crate::animation::{FRAME_HEIGHT, FRAME_WIDTH, FrameSymbol, frame_row};
use crate::cli::{AxisCrop, AxisRange, Config};
use crate::runtime::take_resize_pending;
use crate::telnet::{SessionEvent, SessionInput};
use crate::terminal::{TerminalSize, terminal_size};
pub(crate) use benchmark::BenchmarkReport;
use benchmark::BenchmarkTracker;
use frame_buffer::FrameBuffer;
pub(crate) use palette::Palette;
use render_loop::RenderLoop;
use std::io::{self, Write};
use std::thread;
use std::time::{Duration, Instant};

// Vertical band (frame rows 24..=42) where the off-screen rainbow tail is drawn.
const RAINBOW_BAND_TOP: i32 = 23;
const RAINBOW_BAND_BOTTOM: i32 = 43;

pub(crate) struct RenderState {
    terminal_size: TerminalSize,
    row_crop: AxisCrop,
    col_crop: AxisCrop,
    min_row: i32,
    max_row: i32,
    min_col: i32,
    max_col: i32,
}

impl RenderState {
    pub(crate) fn new(config: &Config, terminal_size: TerminalSize) -> Self {
        let mut state = Self {
            terminal_size,
            row_crop: config.crop.rows,
            col_crop: config.crop.cols,
            min_row: 0,
            max_row: 0,
            min_col: 0,
            max_col: 0,
        };
        state.recalculate_bounds();
        state
    }

    fn update_terminal_size(&mut self, size: TerminalSize) {
        self.terminal_size = size;
        if self.row_crop.is_terminal_dependent() || self.col_crop.is_terminal_dependent() {
            self.recalculate_bounds();
        }
    }

    fn recalculate_bounds(&mut self) {
        let rows = self.row_crop.resolve(self.automatic_row_range());
        let cols = self.col_crop.resolve(self.automatic_col_range());

        self.min_row = rows.min;
        self.max_row = rows.max;
        self.min_col = cols.min;
        self.max_col = cols.max;
    }

    fn automatic_col_range(&self) -> AxisRange {
        let terminal_width = self.terminal_size.width();
        AxisRange::new(
            (FRAME_WIDTH as i32 - terminal_width / 2) / 2,
            (FRAME_WIDTH as i32 + terminal_width / 2) / 2,
        )
    }

    fn automatic_row_range(&self) -> AxisRange {
        let terminal_height = self.terminal_size.height();
        AxisRange::new(
            (FRAME_HEIGHT as i32 - (terminal_height - 1)) / 2,
            (FRAME_HEIGHT as i32 + (terminal_height - 1)) / 2,
        )
    }
}

pub(crate) enum RunOutcome {
    /// The run ended cleanly: the frame limit was reached, or the telnet
    /// client disconnected.
    Finished {
        clear_screen: bool,
        benchmark: Option<BenchmarkReport>,
    },
}

struct Renderer<'a> {
    config: &'a Config,
    palette: &'a Palette,
}

impl<'a> Renderer<'a> {
    fn new(config: &'a Config, palette: &'a Palette) -> Self {
        Self { config, palette }
    }

    fn render_frame(
        &self,
        out: &mut FrameBuffer,
        state: &RenderState,
        frame_index: usize,
        elapsed_seconds: u64,
    ) {
        match self.palette.output {
            Some(output) => self.render_block_frame(out, state, frame_index, output),
            None => self.render_ascii_frame(out, state, frame_index),
        }

        self.render_counter(out, state, elapsed_seconds);
    }

    fn render_block_frame(
        &self,
        out: &mut FrameBuffer,
        state: &RenderState,
        frame_index: usize,
        output: &'static [u8],
    ) {
        let mut last: Option<FrameSymbol> = None;
        let mut run = 0usize;

        for y in state.min_row..state.max_row {
            // Row-invariant work hoisted out of the per-cell path: whether this
            // row is in the rainbow band, and its in-frame byte slice (one frame
            // lookup + bounds check per row instead of per cell).
            let in_band = y > RAINBOW_BAND_TOP && y < RAINBOW_BAND_BOTTOM;
            let row = Self::row_for(frame_index, y);

            for x in state.min_col..state.max_col {
                let color = Self::cell_color(frame_index, in_band, row, y, x);
                if last != Some(color) {
                    // Flush the finished run of identical cells in one fill, then
                    // emit the new color's escape once (cells coalesce into runs,
                    // so the per-cell output becomes a per-run memset).
                    out.push_repeated(output, run);
                    run = 0;
                    let escape = self.palette.color(color);
                    if !escape.is_empty() {
                        out.push_bytes(escape);
                    }
                    last = Some(color);
                }
                run += 1;
            }
            out.push_repeated(output, run);
            run = 0;
            // Fill to the line end with the row's current background. An odd
            // terminal width leaves a one-column gap (cells are two chars wide),
            // which the counter's \x1b[J would otherwise fill on the last row
            // only, leaving a one-cell protrusion at the bottom-right corner.
            out.push_bytes(b"\x1b[K");
            out.push_newlines(self.config.telnet, 1);
        }
    }

    fn render_ascii_frame(&self, out: &mut FrameBuffer, state: &RenderState, frame_index: usize) {
        for y in state.min_row..state.max_row {
            let in_band = y > RAINBOW_BAND_TOP && y < RAINBOW_BAND_BOTTOM;
            let row = Self::row_for(frame_index, y);

            for x in state.min_col..state.max_col {
                let color = Self::cell_color(frame_index, in_band, row, y, x);
                out.push_bytes(self.palette.color(color));
            }
            out.push_newlines(self.config.telnet, 1);
        }
    }

    /// The in-frame byte row for `y`, or `None` when `y` falls outside the frame.
    #[inline]
    fn row_for(frame_index: usize, y: i32) -> Option<&'static [u8]> {
        if (0..FRAME_HEIGHT as i32).contains(&y) {
            Some(frame_row(frame_index, y as usize))
        } else {
            None
        }
    }

    #[inline]
    fn cell_color(
        frame_index: usize,
        in_band: bool,
        row: Option<&[u8]>,
        y: i32,
        x: i32,
    ) -> FrameSymbol {
        if x < 0 {
            if in_band {
                Self::rainbow(frame_index, y, x)
            } else {
                FrameSymbol::BACKGROUND
            }
        } else if let Some(row) = row {
            // x >= FRAME_WIDTH falls off the slice and reads as background.
            row.get(x as usize)
                .map_or(FrameSymbol::BACKGROUND, |&byte| {
                    FrameSymbol::from_byte(byte)
                })
        } else {
            FrameSymbol::BACKGROUND
        }
    }

    // Off-screen rainbow tail: a two-phase square wave that flips every couple of
    // frames. Cold path (only x < 0 in the band), so kept out of the hot loop.
    fn rainbow(frame_index: usize, y: i32, x: i32) -> FrameSymbol {
        const RAINBOW: &[FrameSymbol] = &[
            FrameSymbol::BACKGROUND,
            FrameSymbol::BACKGROUND,
            FrameSymbol::RED,
            FrameSymbol::RED,
            FrameSymbol::ORANGE,
            FrameSymbol::ORANGE,
            FrameSymbol::ORANGE,
            FrameSymbol::YELLOW,
            FrameSymbol::YELLOW,
            FrameSymbol::YELLOW,
            FrameSymbol::GREEN,
            FrameSymbol::GREEN,
            FrameSymbol::GREEN,
            FrameSymbol::BLUE,
            FrameSymbol::BLUE,
            FrameSymbol::INDIGO,
            FrameSymbol::INDIGO,
            FrameSymbol::INDIGO,
            FrameSymbol::BACKGROUND,
            FrameSymbol::BACKGROUND,
        ];
        const STRIPE_PERIOD: i32 = 16;
        const STRIPE_HALF: i32 = 8;

        let mut mod_x = ((-x + 2) % STRIPE_PERIOD) / STRIPE_HALF;
        if (frame_index / 2) % 2 == 1 {
            mod_x = 1 - mod_x;
        }
        let index = (mod_x + y - RAINBOW_BAND_TOP) as usize;
        RAINBOW
            .get(index)
            .copied()
            .unwrap_or(FrameSymbol::BACKGROUND)
    }

    fn render_counter(&self, out: &mut FrameBuffer, state: &RenderState, elapsed_seconds: u64) {
        if self.config.show_counter {
            const PREFIX: &[u8] = b"You have nyaned for ";
            const SUFFIX: &[u8] = b" seconds!";

            // ASCII palettes (Vt220/Vt100, and NO_COLOR) carry no background, so
            // the counter omits its color escapes there and stays colorless.
            let colored = self.palette.output.is_some();
            // Centering width is derived from the surrounding text length, not a
            // hardcoded constant, so the layout stays correct if the text changes.
            let text_len = (PREFIX.len() + SUFFIX.len()) as i32 + decimal_digits(elapsed_seconds);
            let width = (state.terminal_size.width() - text_len) / 2;
            out.push_spaces(width);
            if colored {
                out.push_bytes(b"\x1b[1;37m");
            }
            out.push_bytes(PREFIX);
            let _ = write!(out, "{elapsed_seconds}");
            out.push_bytes(SUFFIX);
            out.push_bytes(b"\x1b[J");
            if colored {
                out.push_bytes(b"\x1b[0m");
            }
        }
    }
}

fn decimal_digits(mut value: u64) -> i32 {
    let mut digits = 1;
    while value >= 10 {
        value /= 10;
        digits += 1;
    }
    digits
}

pub(crate) fn run(
    config: Config,
    mut state: RenderState,
    palette: Palette,
) -> io::Result<RunOutcome> {
    let mut stdout = io::stdout().lock();

    if config.set_title {
        stdout.write_all(b"\x1bkNyanyanyanyanyanyanya...\x1b\\")?;
        stdout.write_all(b"\x1b]1;Nyanyanyanyanyanyanya...\x07")?;
        stdout.write_all(b"\x1b]2;Nyanyanyanyanyanyanya...\x07")?;
    }

    if config.clear_screen {
        // Enter the alternate screen buffer so the animation runs on its own
        // screen and the terminal's prior contents/scrollback are restored on
        // exit (see runtime::restore_sequence).
        stdout.write_all(b"\x1b[?1049h\x1b[H\x1b[2J\x1b[?25l")?;
    } else {
        stdout.write_all(b"\x1b[s")?;
    }

    if config.show_intro {
        show_intro(&mut stdout, config.telnet, config.clear_screen)?;
    }

    let renderer = Renderer::new(&config, &palette);
    let mut render_loop = RenderLoop::new(config.delay);
    let mut buffer = FrameBuffer::with_capacity(32 * 1024);
    let mut benchmark = config.benchmark.then(BenchmarkTracker::new);
    let mut telnet_input = config.telnet.then(SessionInput::new);
    let mut telnet_resized = false;

    loop {
        let frame_start = Instant::now();

        let resized = if config.telnet {
            std::mem::take(&mut telnet_resized)
        } else if take_resize_pending() {
            state.update_terminal_size(terminal_size());
            true
        } else {
            false
        };

        buffer.clear();
        if resized && config.clear_screen {
            // The previous frame was drawn for the old terminal size. Clear the
            // screen so a now-narrower (or shorter) frame cannot leave stale
            // cells along the right edge or below the animation.
            buffer.push_bytes(b"\x1b[2J");
        }
        buffer.push_frame_prefix(config.clear_screen);

        renderer.render_frame(
            &mut buffer,
            &state,
            render_loop.frame_index(),
            render_loop.elapsed_seconds(),
        );
        stdout.write_all(buffer.as_bytes())?;
        stdout.flush()?;
        if let Some(benchmark) = &mut benchmark {
            benchmark.record_frame(buffer.as_bytes().len());
        }

        if render_loop.finish_frame(frame_start, config.frame_limit) {
            return Ok(RunOutcome::Finished {
                clear_screen: config.clear_screen,
                benchmark: benchmark.map(BenchmarkTracker::finish),
            });
        }

        // After the frame is out, drain any client input that arrived during
        // it. Placed after the frame-limit check so finite runs (smoke tests,
        // goldens) never depend on stdin state.
        if let Some(input) = &mut telnet_input {
            match input.poll() {
                Some(SessionEvent::Resize(size)) => {
                    state.update_terminal_size(size);
                    telnet_resized = true;
                }
                Some(SessionEvent::Disconnected) => {
                    return Ok(RunOutcome::Finished {
                        clear_screen: config.clear_screen,
                        benchmark: benchmark.map(BenchmarkTracker::finish),
                    });
                }
                None => {}
            }
        }
    }
}

fn show_intro(out: &mut impl Write, telnet: bool, clear_screen: bool) -> io::Result<()> {
    let countdown_clock = 5;

    for k in 0..countdown_clock {
        let mut buffer = FrameBuffer::with_capacity(1024);
        buffer.push_newlines(telnet, 3);
        buffer.push_bytes(b"                             \x1b[1mNyancat Telnet Server\x1b[0m");
        buffer.push_newlines(telnet, 2);
        buffer.push_bytes(
            b"                   written and run by \x1b[1;32mK. Lange\x1b[1;34m @_klange\x1b[0m",
        );
        buffer.push_newlines(telnet, 2);
        buffer.push_bytes(b"        If things don't look right, try:");
        buffer.push_newlines(telnet, 1);
        buffer.push_bytes(b"                TERM=fallback telnet ...");
        buffer.push_newlines(telnet, 2);
        buffer.push_bytes(b"        Or on Windows:");
        buffer.push_newlines(telnet, 1);
        buffer.push_bytes(b"                telnet -t vtnt ...");
        buffer.push_newlines(telnet, 2);
        buffer.push_bytes(b"        Problems? Check the website:");
        buffer.push_newlines(telnet, 1);
        buffer.push_bytes(b"                \x1b[1;34mhttp://nyancat.dakko.us\x1b[0m");
        buffer.push_newlines(telnet, 2);
        buffer.push_bytes(b"        This is a telnet server, remember your escape keys!");
        buffer.push_newlines(telnet, 1);
        buffer.push_bytes(b"                \x1b[1;31m^]quit\x1b[0m to exit");
        buffer.push_newlines(telnet, 2);
        let _ = writeln!(
            buffer,
            "        Starting in {}...                ",
            countdown_clock - k
        );

        out.write_all(buffer.as_bytes())?;
        out.flush()?;
        thread::sleep(Duration::from_millis(400));
        if clear_screen {
            out.write_all(b"\x1b[H")?;
        } else {
            out.write_all(b"\x1b[u")?;
        }
    }

    if clear_screen {
        out.write_all(b"\x1b[H\x1b[2J\x1b[?25l")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::TerminalType;

    fn bytes_contain(bytes: &[u8], needle: &[u8]) -> bool {
        bytes.windows(needle.len()).any(|window| window == needle)
    }

    fn render_test_frame_for_terminal(
        config: &Config,
        terminal_type: TerminalType,
        terminal_size: TerminalSize,
        elapsed_seconds: u64,
    ) -> Vec<u8> {
        let state = RenderState::new(config, terminal_size);
        let palette = Palette::new(terminal_type);
        let renderer = Renderer::new(config, &palette);
        let mut out = FrameBuffer::with_capacity(32 * 1024);

        renderer.render_frame(&mut out, &state, 0, elapsed_seconds);

        out.into_bytes()
    }

    fn render_test_frame(config: &Config, elapsed_seconds: u64) -> Vec<u8> {
        render_test_frame_for_terminal(
            config,
            TerminalType::Vt100Ascii,
            TerminalSize::new(80, 24),
            elapsed_seconds,
        )
    }

    #[test]
    fn xterm_frame_uses_256_color_sequences() {
        let config = Config {
            show_counter: false,
            ..Config::default()
        };
        let out = render_test_frame_for_terminal(
            &config,
            TerminalType::Xterm256,
            TerminalSize::new(80, 24),
            0,
        );

        assert!(bytes_contain(&out, b"\x1b[48;5;17m"));
        assert!(bytes_contain(&out, b"\x1b[48;5;196m"));
        assert!(!bytes_contain(&out, b"\x1b[48;2;"));
    }

    #[test]
    fn truecolor_frame_uses_24_bit_color_sequences() {
        let config = Config {
            show_counter: false,
            ..Config::default()
        };
        let out = render_test_frame_for_terminal(
            &config,
            TerminalType::TrueColor,
            TerminalSize::new(80, 24),
            0,
        );

        assert!(bytes_contain(&out, b"\x1b[48;2;0;49;105m"));
        assert!(bytes_contain(&out, b"\x1b[48;2;255;25;0m"));
        assert!(!bytes_contain(&out, b"\x1b[48;5;"));
    }

    #[test]
    fn vt100_ascii_frame_uses_plain_text_symbols() {
        let config = Config {
            show_counter: false,
            ..Config::default()
        };
        let out = render_test_frame_for_terminal(
            &config,
            TerminalType::Vt100Ascii,
            TerminalSize::new(40, 24),
            0,
        );

        assert!(bytes_contain(&out, b"@"));
        assert!(bytes_contain(&out, b"#"));
        assert!(!bytes_contain(&out, b"\x1b["));
    }

    #[test]
    fn counter_uses_supplied_elapsed_seconds() {
        let config = Config::default();
        let out = render_test_frame(&config, 42);

        assert!(bytes_contain(&out, b"You have nyaned for 42 seconds!"));
    }

    #[test]
    fn counter_can_be_disabled() {
        let config = Config {
            show_counter: false,
            ..Config::default()
        };
        let out = render_test_frame(&config, 42);

        assert!(!bytes_contain(&out, b"You have nyaned for"));
    }

    #[test]
    fn telnet_mode_uses_telnet_newlines() {
        let config = Config {
            telnet: true,
            show_counter: false,
            ..Config::default()
        };
        let out = render_test_frame(&config, 0);

        assert!(bytes_contain(&out, b"\r\0\n"));
        assert!(!bytes_contain(&out, b"\n\n"));
    }

    #[test]
    fn decimal_digits_counts_without_formatting() {
        assert_eq!(decimal_digits(0), 1);
        assert_eq!(decimal_digits(9), 1);
        assert_eq!(decimal_digits(10), 2);
        assert_eq!(decimal_digits(999), 3);
        assert_eq!(decimal_digits(1_000), 4);
        assert_eq!(decimal_digits(u64::MAX), 20);
    }
}
