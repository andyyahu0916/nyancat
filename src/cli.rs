use crate::animation::{FRAME_HEIGHT, FRAME_WIDTH};
use std::fmt;
use std::num::NonZeroU32;
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct CropBounds {
    pub(crate) rows: AxisCrop,
    pub(crate) cols: AxisCrop,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AxisRange {
    pub(crate) min: i32,
    pub(crate) max: i32,
}

impl AxisRange {
    pub(crate) const fn new(min: i32, max: i32) -> Self {
        Self { min, max }
    }

    const fn with_min(self, min: i32) -> Self {
        Self { min, ..self }
    }

    const fn with_max(self, max: i32) -> Self {
        Self { max, ..self }
    }

    #[cfg(test)]
    const fn as_pair(self) -> (i32, i32) {
        (self.min, self.max)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum AxisCrop {
    #[default]
    Auto,
    AutoBounded {
        min: Option<i32>,
        max: Option<i32>,
    },
    Fixed(AxisRange),
}

impl AxisCrop {
    fn centered(frame_size: i32, size: i32) -> Self {
        // Compute in i64 so large --width/--height values cannot overflow the
        // i32 sum/difference. Release builds have no overflow checks and would
        // otherwise wrap silently into garbage crop bounds.
        let frame_size = i64::from(frame_size);
        let size = i64::from(size);
        Self::Fixed(AxisRange::new(
            ((frame_size - size) / 2) as i32,
            ((frame_size + size) / 2) as i32,
        ))
    }

    fn set_min(&mut self, min: i32) {
        *self = match *self {
            Self::Auto => Self::AutoBounded {
                min: Some(min),
                max: None,
            },
            Self::AutoBounded { max, .. } => Self::AutoBounded {
                min: Some(min),
                max,
            },
            Self::Fixed(range) => Self::Fixed(range.with_min(min)),
        };
    }

    fn set_max(&mut self, max: i32) {
        *self = match *self {
            Self::Auto => Self::AutoBounded {
                min: None,
                max: Some(max),
            },
            Self::AutoBounded { min, .. } => Self::AutoBounded {
                min,
                max: Some(max),
            },
            Self::Fixed(range) => Self::Fixed(range.with_max(max)),
        };
    }

    pub(crate) fn resolve(self, automatic: AxisRange) -> AxisRange {
        match self {
            Self::Auto => automatic,
            Self::AutoBounded { min, max } => {
                AxisRange::new(min.unwrap_or(automatic.min), max.unwrap_or(automatic.max))
            }
            Self::Fixed(range) => range,
        }
    }

    pub(crate) fn is_terminal_dependent(self) -> bool {
        matches!(self, Self::Auto | Self::AutoBounded { .. })
    }

    fn known_range(self) -> Option<AxisRange> {
        match self {
            Self::AutoBounded {
                min: Some(min),
                max: Some(max),
            } => Some(AxisRange::new(min, max)),
            Self::Fixed(range) => Some(range),
            Self::Auto | Self::AutoBounded { .. } => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FrameLimit(NonZeroU32);

impl FrameLimit {
    pub(crate) fn new(frames: u32) -> Option<Self> {
        NonZeroU32::new(frames).map(Self)
    }

    pub(crate) const fn get(self) -> u32 {
        self.0.get()
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct Config {
    pub(crate) telnet: bool,
    pub(crate) show_counter: bool,
    pub(crate) frame_limit: Option<FrameLimit>,
    pub(crate) clear_screen: bool,
    pub(crate) set_title: bool,
    pub(crate) show_intro: bool,
    pub(crate) skip_intro: bool,
    pub(crate) delay: Duration,
    pub(crate) benchmark: bool,
    pub(crate) truecolor: bool,
    pub(crate) crop: CropBounds,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum CliAction {
    Run(Config),
    Help { program: String },
    Version,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum CliError {
    UnexpectedArgument {
        argument: String,
    },
    MissingValue {
        option: String,
    },
    UnexpectedValue {
        option: String,
        value: String,
    },
    InvalidValue {
        option: String,
        value: String,
    },
    NonPositiveValue {
        option: String,
        value: i32,
    },
    ValueOutOfRange {
        option: String,
        value: i32,
        min: i32,
        max: i32,
    },
    UnknownOption {
        option: String,
    },
    InvalidCropRange {
        axis: &'static str,
        min: i32,
        max: i32,
    },
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedArgument { argument } => {
                write!(f, "unexpected argument: {argument}")
            }
            Self::MissingValue { option } => write!(f, "missing value for {option}"),
            Self::UnexpectedValue { option, value } => {
                write!(f, "unexpected value for {option}: {value}")
            }
            Self::InvalidValue { option, value } => {
                write!(f, "invalid value for {option}: {value}")
            }
            Self::NonPositiveValue { option, value } => {
                write!(f, "value for {option} must be positive: {value}")
            }
            Self::ValueOutOfRange {
                option,
                value,
                min,
                max,
            } => write!(
                f,
                "value for {option} out of range: {value} (expected {min}-{max})"
            ),
            Self::UnknownOption { option } => write!(f, "unknown option: {option}"),
            Self::InvalidCropRange { axis, min, max } => write!(
                f,
                "invalid {axis} crop range: minimum {min} must be less than maximum {max}"
            ),
        }
    }
}

impl std::error::Error for CliError {}

impl Default for Config {
    fn default() -> Self {
        Self {
            telnet: false,
            show_counter: true,
            frame_limit: None,
            clear_screen: true,
            set_title: true,
            show_intro: false,
            skip_intro: false,
            delay: Duration::from_millis(90),
            benchmark: false,
            truecolor: false,
            crop: CropBounds::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OptionId {
    NoClear,
    NoTitle,
    Intro,
    SkipIntro,
    Telnet,
    Benchmark,
    TrueColor,
    Help,
    Version,
    NoCounter,
    Delay,
    Frames,
    MinRows,
    MaxRows,
    MinCols,
    MaxCols,
    Width,
    Height,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OptionArity {
    Flag,
    Value,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OptionSpec {
    id: OptionId,
    short: char,
    long: &'static str,
    arity: OptionArity,
    value_name: Option<&'static str>,
    description: &'static str,
}

impl OptionSpec {
    const fn flag(
        id: OptionId,
        short: char,
        long: &'static str,
        description: &'static str,
    ) -> Self {
        Self {
            id,
            short,
            long,
            arity: OptionArity::Flag,
            value_name: None,
            description,
        }
    }

    const fn value(
        id: OptionId,
        short: char,
        long: &'static str,
        value_name: &'static str,
        description: &'static str,
    ) -> Self {
        Self {
            id,
            short,
            long,
            arity: OptionArity::Value,
            value_name: Some(value_name),
            description,
        }
    }

    const fn takes_value(self) -> bool {
        matches!(self.arity, OptionArity::Value)
    }
}

const OPTION_SPECS: &[OptionSpec] = &[
    OptionSpec::flag(
        OptionId::Intro,
        'i',
        "intro",
        "Show the introduction / about information at startup.",
    ),
    OptionSpec::flag(
        OptionId::SkipIntro,
        'I',
        "skip-intro",
        "Skip the introduction in telnet mode.",
    ),
    OptionSpec::flag(
        OptionId::Telnet,
        't',
        "telnet",
        "Speak telnet over stdin/stdout; does not open a port.",
    ),
    OptionSpec::flag(
        OptionId::TrueColor,
        'T',
        "truecolor",
        "Enable 24-bit TrueColor mode (high-definition rendering).",
    ),
    OptionSpec::flag(
        OptionId::NoCounter,
        'n',
        "no-counter",
        "Do not display the timer.",
    ),
    OptionSpec::flag(
        OptionId::NoTitle,
        's',
        "no-title",
        "Do not set the titlebar text.",
    ),
    OptionSpec::flag(
        OptionId::NoClear,
        'e',
        "no-clear",
        "Do not clear the display between frames.",
    ),
    OptionSpec::flag(
        OptionId::Benchmark,
        'b',
        "benchmark",
        "Run in benchmark mode (0ms delay). Warning: high CPU usage.",
    ),
    OptionSpec::flag(OptionId::Help, 'h', "help", "Show this help message."),
    OptionSpec::flag(
        OptionId::Version,
        'V',
        "version",
        "Show version information and exit.",
    ),
    OptionSpec::value(
        OptionId::Delay,
        'd',
        "delay",
        "ms",
        "Delay image rendering by anywhere between 10ms and 1000ms.",
    ),
    OptionSpec::value(
        OptionId::Frames,
        'f',
        "frames",
        "frames",
        "Display the requested positive number of frames, then quit.",
    ),
    OptionSpec::value(
        OptionId::MinRows,
        'r',
        "min-rows",
        "row",
        "Crop the animation from the top.",
    ),
    OptionSpec::value(
        OptionId::MaxRows,
        'R',
        "max-rows",
        "row",
        "Crop the animation from the bottom.",
    ),
    OptionSpec::value(
        OptionId::MinCols,
        'c',
        "min-cols",
        "col",
        "Crop the animation from the left.",
    ),
    OptionSpec::value(
        OptionId::MaxCols,
        'C',
        "max-cols",
        "col",
        "Crop the animation from the right.",
    ),
    OptionSpec::value(
        OptionId::Width,
        'W',
        "width",
        "width",
        "Crop the animation to the given width.",
    ),
    OptionSpec::value(
        OptionId::Height,
        'H',
        "height",
        "height",
        "Crop the animation to the given height.",
    ),
];

fn option_by_long(long: &str) -> Option<OptionSpec> {
    OPTION_SPECS.iter().copied().find(|spec| spec.long == long)
}

fn option_by_short(short: char) -> Option<OptionSpec> {
    OPTION_SPECS
        .iter()
        .copied()
        .find(|spec| spec.short == short)
}

pub(crate) fn parse_args(args: &[String]) -> Result<CliAction, CliError> {
    let program = args
        .first()
        .cloned()
        .unwrap_or_else(|| "nyancat".to_string());
    let mut config = Config::default();
    let mut i = 1usize;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            if let Some(argument) = args.get(i + 1) {
                return Err(CliError::UnexpectedArgument {
                    argument: argument.clone(),
                });
            }
            break;
        }

        if let Some(long) = arg.strip_prefix("--") {
            let (name, value) = long.split_once('=').map_or((long, None), |(name, value)| {
                (name, Some(value.to_string()))
            });
            let Some(spec) = option_by_long(name) else {
                return Err(CliError::UnknownOption {
                    option: format!("--{name}"),
                });
            };

            let option = format!("--{name}");
            if spec.takes_value() {
                let value = match value {
                    Some(value) => value,
                    None => {
                        i += 1;
                        args.get(i).cloned().ok_or_else(|| CliError::MissingValue {
                            option: option.clone(),
                        })?
                    }
                };
                apply_value_option(&mut config, spec, &option, &value)?;
            } else {
                if let Some(value) = value {
                    return Err(CliError::UnexpectedValue { option, value });
                }
                if let Some(action) = apply_flag(&mut config, spec, &option, &program)? {
                    return Ok(action);
                }
            }
        } else if arg.starts_with('-') && arg.len() > 1 {
            let bytes = arg.as_bytes();
            let mut pos = 1usize;
            while pos < bytes.len() {
                let opt = bytes[pos] as char;
                pos += 1;

                let Some(spec) = option_by_short(opt) else {
                    return Err(CliError::UnknownOption {
                        option: format!("-{opt}"),
                    });
                };

                if spec.takes_value() {
                    let option = format!("-{opt}");
                    let value = if pos < bytes.len() {
                        String::from_utf8_lossy(&bytes[pos..]).into_owned()
                    } else {
                        i += 1;
                        args.get(i).cloned().ok_or_else(|| CliError::MissingValue {
                            option: option.clone(),
                        })?
                    };
                    apply_value_option(&mut config, spec, &option, &value)?;
                    break;
                }
                if let Some(action) = apply_flag(&mut config, spec, &format!("-{opt}"), &program)? {
                    return Ok(action);
                }
            }
        } else {
            return Err(CliError::UnexpectedArgument {
                argument: arg.clone(),
            });
        }

        i += 1;
    }

    validate_crop_bounds(config.crop)?;
    Ok(CliAction::Run(config))
}

fn validate_crop_bounds(crop: CropBounds) -> Result<(), CliError> {
    validate_axis_crop("row", crop.rows)?;
    validate_axis_crop("column", crop.cols)
}

fn validate_axis_crop(axis: &'static str, crop: AxisCrop) -> Result<(), CliError> {
    if let Some(range) = crop.known_range() {
        if range.min >= range.max {
            return Err(CliError::InvalidCropRange {
                axis,
                min: range.min,
                max: range.max,
            });
        }
    }

    Ok(())
}

fn apply_flag(
    config: &mut Config,
    spec: OptionSpec,
    option: &str,
    program: &str,
) -> Result<Option<CliAction>, CliError> {
    match spec.id {
        OptionId::NoClear => config.clear_screen = false,
        OptionId::NoTitle => config.set_title = false,
        OptionId::Intro => config.show_intro = true,
        OptionId::SkipIntro => config.skip_intro = true,
        OptionId::Telnet => config.telnet = true,
        OptionId::Benchmark => config.benchmark = true,
        OptionId::TrueColor => config.truecolor = true,
        OptionId::Help => {
            return Ok(Some(CliAction::Help {
                program: program.to_string(),
            }));
        }
        OptionId::Version => return Ok(Some(CliAction::Version)),
        OptionId::NoCounter => config.show_counter = false,
        _ => {
            return Err(CliError::UnknownOption {
                option: option.to_string(),
            });
        }
    }

    Ok(None)
}

/// Fork-specific safety bounds on the geometric crop options. The limits are
/// far larger than any real terminal, so they do not affect normal use, but
/// they stop extreme values from overflowing crop arithmetic or forcing
/// multi-billion-iteration render loops.
const CROP_OFFSET_LIMIT: i32 = 10_000;
const CROP_SIZE_LIMIT: i32 = 10_000;

fn bounded(option: &str, value: i32, min: i32, max: i32) -> Result<i32, CliError> {
    if (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(CliError::ValueOutOfRange {
            option: option.to_string(),
            value,
            min,
            max,
        })
    }
}

fn apply_value_option(
    config: &mut Config,
    spec: OptionSpec,
    option: &str,
    value: &str,
) -> Result<(), CliError> {
    let parsed = value.parse::<i32>().map_err(|_| CliError::InvalidValue {
        option: option.to_string(),
        value: value.to_string(),
    })?;

    match spec.id {
        OptionId::Delay => {
            config.delay = Duration::from_millis(bounded(option, parsed, 10, 1000)? as u64);
        }
        OptionId::Frames => {
            if parsed <= 0 {
                return Err(CliError::NonPositiveValue {
                    option: option.to_string(),
                    value: parsed,
                });
            }
            config.frame_limit = FrameLimit::new(parsed as u32);
        }
        OptionId::MinRows => {
            let offset = bounded(option, parsed, -CROP_OFFSET_LIMIT, CROP_OFFSET_LIMIT)?;
            config.crop.rows.set_min(offset);
        }
        OptionId::MaxRows => {
            let offset = bounded(option, parsed, -CROP_OFFSET_LIMIT, CROP_OFFSET_LIMIT)?;
            config.crop.rows.set_max(offset);
        }
        OptionId::MinCols => {
            let offset = bounded(option, parsed, -CROP_OFFSET_LIMIT, CROP_OFFSET_LIMIT)?;
            config.crop.cols.set_min(offset);
        }
        OptionId::MaxCols => {
            let offset = bounded(option, parsed, -CROP_OFFSET_LIMIT, CROP_OFFSET_LIMIT)?;
            config.crop.cols.set_max(offset);
        }
        OptionId::Width => {
            let size = bounded(option, parsed, 1, CROP_SIZE_LIMIT)?;
            config.crop.cols = AxisCrop::centered(FRAME_WIDTH as i32, size);
        }
        OptionId::Height => {
            let size = bounded(option, parsed, 1, CROP_SIZE_LIMIT)?;
            config.crop.rows = AxisCrop::centered(FRAME_HEIGHT as i32, size);
        }
        _ => {
            return Err(CliError::UnknownOption {
                option: option.to_string(),
            });
        }
    }

    Ok(())
}

#[cfg(test)]
pub(crate) fn usage_text(program: &str) -> String {
    usage_text_with_style(program, true)
}

fn usage_text_with_style(program: &str, styled: bool) -> String {
    let mut usage = format!("Terminal Nyancat\n\nusage: {program} [options]\n\n");

    for spec in OPTION_SPECS {
        let option = match spec.value_name {
            Some(value_name) => format!("-{}, --{} <{}>", spec.short, spec.long, value_name),
            None => format!("-{}, --{}", spec.short, spec.long),
        };
        if styled {
            usage.push_str(&format!(
                "  {option:<25}\x1b[3m{}\x1b[0m\n",
                spec.description
            ));
        } else {
            usage.push_str(&format!("  {option:<25}{}\n", spec.description));
        }
    }

    usage
}

pub(crate) fn print_usage(program: &str, styled: bool) {
    print!("{}", usage_text_with_style(program, styled));
}

pub(crate) fn print_version() {
    println!("nyancat {}", env!("CARGO_PKG_VERSION"));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value_spec(short: char) -> OptionSpec {
        option_by_short(short).unwrap()
    }

    #[test]
    fn width_and_height_options_center_crop() {
        let mut config = Config::default();
        apply_value_option(&mut config, value_spec('W'), "-W", "40").unwrap();
        apply_value_option(&mut config, value_spec('H'), "-H", "24").unwrap();
        assert_eq!(
            config.crop.cols.resolve(AxisRange::new(0, 0)).as_pair(),
            (12, 52)
        );
        assert_eq!(
            config.crop.rows.resolve(AxisRange::new(0, 0)).as_pair(),
            (20, 44)
        );
    }

    #[test]
    fn width_height_and_crop_offsets_are_bounded() {
        let mut config = Config::default();

        // Sizes must be positive and within the fork-specific safety cap.
        assert_eq!(
            apply_value_option(&mut config, value_spec('W'), "-W", "0"),
            Err(CliError::ValueOutOfRange {
                option: "-W".to_string(),
                value: 0,
                min: 1,
                max: 10_000,
            })
        );
        assert_eq!(
            apply_value_option(&mut config, value_spec('W'), "-W", "10001"),
            Err(CliError::ValueOutOfRange {
                option: "-W".to_string(),
                value: 10_001,
                min: 1,
                max: 10_000,
            })
        );
        // i32::MIN previously overflowed AxisCrop::centered.
        assert_eq!(
            apply_value_option(&mut config, value_spec('H'), "-H", "-2147483648"),
            Err(CliError::ValueOutOfRange {
                option: "-H".to_string(),
                value: -2147483648,
                min: 1,
                max: 10_000,
            })
        );
        // Offsets may be negative but are still bounded; i32::MIN previously
        // overflowed the rainbow-tail negation in the render path.
        assert_eq!(
            apply_value_option(&mut config, value_spec('c'), "-c", "-2147483648"),
            Err(CliError::ValueOutOfRange {
                option: "-c".to_string(),
                value: -2147483648,
                min: -10_000,
                max: 10_000,
            })
        );

        // Representative in-range values are accepted.
        assert!(apply_value_option(&mut config, value_spec('W'), "-W", "10000").is_ok());
        assert!(apply_value_option(&mut config, value_spec('c'), "-c", "-10000").is_ok());
    }

    #[test]
    fn option_specs_resolve_long_and_short_forms() {
        let frames = option_by_long("frames").unwrap();

        assert_eq!(frames.short, 'f');
        assert_eq!(option_by_short('f'), Some(frames));
        assert!(frames.takes_value());
        assert!(!option_by_long("telnet").unwrap().takes_value());
        assert_eq!(option_by_long("wat"), None);
        assert_eq!(option_by_short('?'), None);
    }

    #[test]
    fn option_specs_are_unique() {
        for (index, spec) in OPTION_SPECS.iter().enumerate() {
            for other in &OPTION_SPECS[index + 1..] {
                assert_ne!(spec.short, other.short);
                assert_ne!(spec.long, other.long);
            }
        }
    }

    #[test]
    fn usage_text_lists_every_option_spec() {
        let usage = usage_text("nyancat");

        for spec in OPTION_SPECS {
            assert!(usage.contains(&format!("-{}, --{}", spec.short, spec.long)));
            assert!(usage.contains(spec.description));
            if let Some(value_name) = spec.value_name {
                assert!(usage.contains(&format!("<{value_name}>")));
            }
        }
    }

    #[test]
    fn usage_text_resets_italic_descriptions() {
        let usage = usage_text("nyancat");

        for line in usage.lines().filter(|line| line.contains("\x1b[3m")) {
            assert!(line.ends_with("\x1b[0m"));
        }
    }

    #[test]
    fn plain_usage_text_omits_terminal_styling() {
        let usage = usage_text_with_style("nyancat", false);

        assert!(!usage.contains('\x1b'));
        assert!(usage.contains("-t, --telnet"));
    }

    #[test]
    fn public_docs_list_every_option_spec() {
        let readme = include_str!("../README.md");
        let manpage = include_str!("../nyancat.1");

        for spec in OPTION_SPECS {
            let short = format!("-{}", spec.short);
            let long = format!("--{}", spec.long);

            assert!(
                readme.contains(&format!("`{short}`")),
                "README missing {short}"
            );
            assert!(
                readme.contains(&format!("`{long}`")),
                "README missing {long}"
            );
            assert!(
                manpage.contains(&roff_escape_option(&short)),
                "manpage missing {short}"
            );
            assert!(
                manpage.contains(&roff_escape_option(&long)),
                "manpage missing {long}"
            );
        }
    }

    fn roff_escape_option(option: &str) -> String {
        option.replace('-', "\\-")
    }

    #[test]
    fn default_crop_is_automatic() {
        let config = Config::default();

        assert_eq!(config.crop.cols, AxisCrop::Auto);
        assert_eq!(config.crop.rows, AxisCrop::Auto);
        assert_eq!(
            config.crop.cols.resolve(AxisRange::new(12, 52)).as_pair(),
            (12, 52)
        );
        assert!(config.crop.cols.is_terminal_dependent());
        assert!(config.crop.rows.is_terminal_dependent());
    }

    #[test]
    fn min_and_max_options_bound_automatic_crop() {
        let mut config = Config::default();
        apply_value_option(&mut config, value_spec('r'), "-r", "20").unwrap();
        apply_value_option(&mut config, value_spec('C'), "-C", "60").unwrap();

        assert_eq!(
            config.crop.rows.resolve(AxisRange::new(10, 40)).as_pair(),
            (20, 40)
        );
        assert_eq!(
            config.crop.cols.resolve(AxisRange::new(12, 52)).as_pair(),
            (12, 60)
        );
        assert!(config.crop.rows.is_terminal_dependent());
        assert!(config.crop.cols.is_terminal_dependent());

        apply_value_option(&mut config, value_spec('R'), "-R", "30").unwrap();

        assert_eq!(
            config.crop.rows.resolve(AxisRange::new(10, 40)).as_pair(),
            (20, 30)
        );
    }

    #[test]
    fn parses_short_flags_and_options() {
        let args = vec![
            "nyancat".to_string(),
            "-Tnse".to_string(),
            "-d".to_string(),
            "120".to_string(),
            "-f3".to_string(),
            "--skip-intro".to_string(),
            "--width=40".to_string(),
            "--height".to_string(),
            "24".to_string(),
        ];

        let CliAction::Run(config) = parse_args(&args).unwrap() else {
            panic!("expected run action");
        };

        assert!(config.truecolor);
        assert!(!config.show_counter);
        assert!(!config.set_title);
        assert!(!config.clear_screen);
        assert_eq!(config.delay, Duration::from_millis(120));
        assert_eq!(config.frame_limit.map(FrameLimit::get), Some(3));
        assert!(config.skip_intro);
        assert_eq!(
            config.crop.cols.resolve(AxisRange::new(0, 0)).as_pair(),
            (12, 52)
        );
        assert_eq!(
            config.crop.rows.resolve(AxisRange::new(0, 0)).as_pair(),
            (20, 44)
        );
    }

    #[test]
    fn help_is_returned_without_exiting() {
        let args = vec!["nyancat".to_string(), "--help".to_string()];

        assert_eq!(
            parse_args(&args),
            Ok(CliAction::Help {
                program: "nyancat".to_string()
            })
        );
    }

    #[test]
    fn version_is_returned_without_exiting() {
        assert_eq!(
            parse_args(&["nyancat".to_string(), "--version".to_string()]),
            Ok(CliAction::Version)
        );
        assert_eq!(
            parse_args(&["nyancat".to_string(), "-V".to_string()]),
            Ok(CliAction::Version)
        );
    }

    #[test]
    fn invalid_delay_is_reported() {
        let mut config = Config::default();
        let delay = value_spec('d');

        assert_eq!(
            apply_value_option(&mut config, delay, "-d", "9"),
            Err(CliError::ValueOutOfRange {
                option: "-d".to_string(),
                value: 9,
                min: 10,
                max: 1000,
            })
        );
        assert_eq!(
            apply_value_option(&mut config, delay, "-d", "not-a-number"),
            Err(CliError::InvalidValue {
                option: "-d".to_string(),
                value: "not-a-number".to_string(),
            })
        );
    }

    #[test]
    fn missing_option_value_is_reported() {
        let args = vec!["nyancat".to_string(), "-d".to_string()];

        assert_eq!(
            parse_args(&args),
            Err(CliError::MissingValue {
                option: "-d".to_string()
            })
        );
    }

    #[test]
    fn positional_arguments_are_rejected() {
        for argument in ["scene.txt", "-", ""] {
            let args = vec!["nyancat".to_string(), argument.to_string()];

            assert_eq!(
                parse_args(&args),
                Err(CliError::UnexpectedArgument {
                    argument: argument.to_string()
                })
            );
        }

        assert_eq!(
            CliError::UnexpectedArgument {
                argument: "scene.txt".to_string()
            }
            .to_string(),
            "unexpected argument: scene.txt"
        );
    }

    #[test]
    fn arguments_after_double_dash_are_rejected() {
        for argument in ["scene.txt", "--help", ""] {
            let args = vec![
                "nyancat".to_string(),
                "--".to_string(),
                argument.to_string(),
            ];

            assert_eq!(
                parse_args(&args),
                Err(CliError::UnexpectedArgument {
                    argument: argument.to_string()
                })
            );
        }
    }

    #[test]
    fn trailing_double_dash_is_accepted() {
        let args = vec!["nyancat".to_string(), "--".to_string()];

        assert_eq!(parse_args(&args), Ok(CliAction::Run(Config::default())));
    }

    #[test]
    fn flag_options_reject_inline_values() {
        let args = vec!["nyancat".to_string(), "--no-counter=false".to_string()];

        assert_eq!(
            parse_args(&args),
            Err(CliError::UnexpectedValue {
                option: "--no-counter".to_string(),
                value: "false".to_string()
            })
        );
    }

    #[test]
    fn frame_count_must_be_positive_when_provided() {
        let args = vec!["nyancat".to_string(), "--frames=-1".to_string()];

        assert_eq!(
            parse_args(&args),
            Err(CliError::NonPositiveValue {
                option: "--frames".to_string(),
                value: -1
            })
        );

        let args = vec!["nyancat".to_string(), "-f0".to_string()];

        assert_eq!(
            parse_args(&args),
            Err(CliError::NonPositiveValue {
                option: "-f".to_string(),
                value: 0
            })
        );
    }

    #[test]
    fn unknown_option_is_reported() {
        let args = vec!["nyancat".to_string(), "--wat".to_string()];

        assert_eq!(
            parse_args(&args),
            Err(CliError::UnknownOption {
                option: "--wat".to_string()
            })
        );
    }

    #[test]
    fn invalid_final_crop_ranges_are_reported() {
        for (axis, min_option, max_option, min, max) in [
            ("row", "--min-rows", "--max-rows", 20, 20),
            ("row", "--min-rows", "--max-rows", 21, 20),
            ("column", "--min-cols", "--max-cols", 30, 30),
            ("column", "--min-cols", "--max-cols", 31, 30),
        ] {
            let args = vec![
                "nyancat".to_string(),
                format!("{min_option}={min}"),
                format!("{max_option}={max}"),
            ];

            assert_eq!(
                parse_args(&args),
                Err(CliError::InvalidCropRange { axis, min, max })
            );
        }

        assert_eq!(
            CliError::InvalidCropRange {
                axis: "row",
                min: 20,
                max: 20,
            }
            .to_string(),
            "invalid row crop range: minimum 20 must be less than maximum 20"
        );
    }

    #[test]
    fn crop_validation_uses_the_final_combined_range() {
        let corrected_bound = vec![
            "nyancat".to_string(),
            "--min-rows=30".to_string(),
            "--max-rows=20".to_string(),
            "--max-rows=40".to_string(),
        ];
        let overwritten_bounds = vec![
            "nyancat".to_string(),
            "--min-cols=50".to_string(),
            "--max-cols=40".to_string(),
            "--width=20".to_string(),
        ];
        let mixed_fixed_and_bounds = vec![
            "nyancat".to_string(),
            "--width=40".to_string(),
            "--min-cols=20".to_string(),
            "--max-cols=30".to_string(),
        ];
        let terminal_dependent_bound = vec!["nyancat".to_string(), "--min-rows=100".to_string()];

        for args in [
            corrected_bound,
            overwritten_bounds,
            mixed_fixed_and_bounds,
            terminal_dependent_bound,
        ] {
            assert!(matches!(parse_args(&args), Ok(CliAction::Run(_))));
        }
    }

    struct Rng(u64);

    impl Rng {
        fn new(seed: u64) -> Self {
            Self(seed | 1)
        }

        fn next_u64(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next_u64() % n as u64) as usize
        }
    }

    // Adversarial coverage for the hand-rolled argument parser. Random argv,
    // including malformed numbers, embedded '=', combined short flags, and
    // arbitrary unicode, must never panic and must parse deterministically.
    #[test]
    fn parse_args_survives_adversarial_argv() {
        let tokens: &[&str] = &[
            "-i",
            "-t",
            "-T",
            "-n",
            "-s",
            "-e",
            "-b",
            "-h",
            "-V",
            "--telnet",
            "--truecolor",
            "--help",
            "--version",
            "--no-counter",
            "-d",
            "--delay",
            "-f",
            "--frames",
            "-r",
            "-R",
            "-c",
            "-C",
            "-W",
            "-H",
            "120",
            "-1",
            "0",
            "=",
            "==",
            "--",
            "-",
            "---",
            "",
            "--delay=120",
            "--frames=-1",
            "--no-counter=false",
            "-Tnse",
            "-d120",
            "-f3",
            "--width=",
            "-x",
            "--unknown",
            "💥",
            "\u{0}",
            "-é",
            "--délai",
            " ",
            "\t",
            "-2147483648",
            "2147483648",
            "--frames=999999999999999999999",
        ];

        let mut rng = Rng::new(0xDEAD_BEEF_1234_5678);
        for _ in 0..10_000 {
            let mut args = vec!["nyancat".to_string()];
            let count = rng.below(6);
            for _ in 0..count {
                if rng.below(4) == 0 {
                    let len = rng.below(6);
                    let token: String = (0..len)
                        .map(|_| char::from_u32(rng.below(0x0011_0000) as u32).unwrap_or('?'))
                        .collect();
                    args.push(token);
                } else {
                    args.push(tokens[rng.below(tokens.len())].to_string());
                }
            }

            // Must not panic, and must be deterministic for identical input.
            assert_eq!(parse_args(&args), parse_args(&args));
        }
    }
}
