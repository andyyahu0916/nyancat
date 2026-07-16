use crate::sys;
use std::num::NonZeroU16;

const MAX_TERMINAL_DIMENSION: u16 = 10_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalType {
    Xterm256,
    Ansi16,
    Linux,
    Fallback,
    Vtnt,
    Vt220,
    Vt100Ascii,
    TrueColor,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TerminalSize {
    width: NonZeroU16,
    height: NonZeroU16,
}

impl TerminalSize {
    pub(crate) const fn new(width: u16, height: u16) -> Self {
        if width == 0
            || height == 0
            || width > MAX_TERMINAL_DIMENSION
            || height > MAX_TERMINAL_DIMENSION
        {
            panic!("terminal dimensions must be between 1 and 10000");
        }

        let (Some(width), Some(height)) = (NonZeroU16::new(width), NonZeroU16::new(height)) else {
            panic!("terminal dimensions must be non-zero");
        };

        Self { width, height }
    }

    pub(crate) fn try_new(width: i32, height: i32) -> Option<Self> {
        let width = u16::try_from(width).ok()?;
        let height = u16::try_from(height).ok()?;
        if width == 0
            || height == 0
            || width > MAX_TERMINAL_DIMENSION
            || height > MAX_TERMINAL_DIMENSION
        {
            return None;
        }
        let width = NonZeroU16::new(width)?;
        let height = NonZeroU16::new(height)?;

        Some(Self { width, height })
    }

    pub(crate) fn with_width(self, width: u16) -> Self {
        Self::new(width, self.height.get())
    }

    pub(crate) const fn width(self) -> i32 {
        self.width.get() as i32
    }

    pub(crate) const fn height(self) -> i32 {
        self.height.get() as i32
    }
}

impl Default for TerminalSize {
    fn default() -> Self {
        Self::new(80, 24)
    }
}

pub(crate) fn terminal_size() -> TerminalSize {
    // Rendering goes to stdout, so query that terminal even when stdin is
    // redirected from a file or pipe.
    sys::stdout_terminal_size()
        .and_then(|(width, height)| TerminalSize::try_new(width, height))
        .unwrap_or_default()
}

pub(crate) fn detect_terminal_type(term: Option<&str>, size: TerminalSize) -> TerminalType {
    let Some(term) = term else {
        return TerminalType::Ansi16;
    };
    let term = term.to_ascii_lowercase();

    if term.contains("xterm") || term.contains("toaru") {
        TerminalType::Xterm256
    } else if term.contains("linux") {
        TerminalType::Linux
    } else if term.contains("vtnt") || term.contains("cygwin") {
        TerminalType::Vtnt
    } else if term.contains("vt220") {
        TerminalType::Vt220
    } else if term.contains("fallback") {
        TerminalType::Fallback
    } else if term.contains("rxvt-256color") {
        TerminalType::Xterm256
    } else if term.contains("rxvt") {
        TerminalType::Linux
    } else if term.contains("vt100") && size.width() == 40 {
        TerminalType::Vt100Ascii
    } else if term.starts_with("st") {
        TerminalType::Xterm256
    } else if term.starts_with("truecolor") {
        TerminalType::TrueColor
    } else if term.contains("256color") {
        // Fork-specific divergence from the historical C detection chain: any
        // terminal advertising 256-color support (e.g. screen-256color,
        // tmux-256color) uses the 256-color palette instead of the upstream
        // 16-color fallback. Placed last so it only upgrades terminals the
        // historical chain leaves unclassified; rxvt-256color and friends are
        // still matched by their explicit branches above.
        TerminalType::Xterm256
    } else {
        TerminalType::Ansi16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_expected_terminal_types() {
        assert_eq!(
            detect_terminal_type(Some("xterm-256color"), TerminalSize::new(80, 24)),
            TerminalType::Xterm256
        );
        assert_eq!(
            detect_terminal_type(Some("linux"), TerminalSize::new(80, 24)),
            TerminalType::Linux
        );
        assert_eq!(
            detect_terminal_type(Some("vt220"), TerminalSize::new(80, 24)),
            TerminalType::Vt220
        );
        assert_eq!(
            detect_terminal_type(Some("vt100"), TerminalSize::new(40, 24)),
            TerminalType::Vt100Ascii
        );
        assert_eq!(
            detect_terminal_type(None, TerminalSize::new(80, 24)),
            TerminalType::Ansi16
        );
    }

    #[test]
    fn terminals_advertising_256color_use_the_256_color_palette() {
        // Fork-specific upgrade: screen/tmux and other *-256color terminals.
        assert_eq!(
            detect_terminal_type(Some("screen-256color"), TerminalSize::new(80, 24)),
            TerminalType::Xterm256
        );
        assert_eq!(
            detect_terminal_type(Some("tmux-256color"), TerminalSize::new(80, 24)),
            TerminalType::Xterm256
        );
        // The explicit rxvt-256color branch must still win over the rxvt branch.
        assert_eq!(
            detect_terminal_type(Some("rxvt-256color"), TerminalSize::new(80, 24)),
            TerminalType::Xterm256
        );
        // Plain screen advertises only 8/16 colors, so it stays the 16-color fallback.
        assert_eq!(
            detect_terminal_type(Some("screen"), TerminalSize::new(80, 24)),
            TerminalType::Ansi16
        );
    }

    #[test]
    fn terminal_size_defaults_to_standard_dimensions() {
        assert_eq!(TerminalSize::default(), TerminalSize::new(80, 24));
        assert_eq!(
            TerminalSize::new(80, 24).with_width(40),
            TerminalSize::new(40, 24)
        );
        assert_eq!(TerminalSize::new(80, 24).width(), 80);
        assert_eq!(TerminalSize::new(80, 24).height(), 24);
        assert_eq!(
            TerminalSize::try_new(80, 24),
            Some(TerminalSize::new(80, 24))
        );
        assert_eq!(TerminalSize::try_new(0, 24), None);
        assert_eq!(TerminalSize::try_new(80, -1), None);
        assert_eq!(
            TerminalSize::try_new(MAX_TERMINAL_DIMENSION as i32, MAX_TERMINAL_DIMENSION as i32),
            Some(TerminalSize::new(
                MAX_TERMINAL_DIMENSION,
                MAX_TERMINAL_DIMENSION
            ))
        );
        assert_eq!(
            TerminalSize::try_new(MAX_TERMINAL_DIMENSION as i32 + 1, 24),
            None
        );
        assert_eq!(TerminalSize::try_new(80, i32::MAX), None);
    }
}
