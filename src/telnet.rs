use crate::sys;
use crate::terminal::TerminalSize;
use std::io::{self, Write};
use std::time::{Duration, Instant};

const IAC: u8 = 255;
const SEND: u8 = 1;
const TELNET_OPTION_COUNT: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TelnetCommand {
    Se,
    Nop,
    Sb,
    Will,
    Wont,
    Do,
    Dont,
    Iac,
    Unknown(u8),
}

impl TelnetCommand {
    fn from_byte(byte: u8) -> Self {
        match byte {
            240 => Self::Se,
            241 => Self::Nop,
            250 => Self::Sb,
            251 => Self::Will,
            252 => Self::Wont,
            253 => Self::Do,
            254 => Self::Dont,
            255 => Self::Iac,
            byte => Self::Unknown(byte),
        }
    }

    const fn raw(self) -> u8 {
        match self {
            Self::Se => 240,
            Self::Nop => 241,
            Self::Sb => 250,
            Self::Will => 251,
            Self::Wont => 252,
            Self::Do => 253,
            Self::Dont => 254,
            Self::Iac => 255,
            Self::Unknown(byte) => byte,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TelnetOption(u8);

impl TelnetOption {
    const ECHO: Self = Self(1);
    const SGA: Self = Self(3);
    const TTYPE: Self = Self(24);
    const NAWS: Self = Self(31);
    const LINEMODE: Self = Self(34);
    const NEW_ENVIRON: Self = Self(39);

    const fn new(byte: u8) -> Self {
        Self(byte)
    }

    const fn raw(self) -> u8 {
        self.0
    }

    const fn index(self) -> usize {
        self.0 as usize
    }
}

trait ByteSource {
    fn read_byte(&mut self, deadline: Instant) -> io::Result<Option<u8>>;
}

struct TimeoutReader {
    buffer: [u8; 1024],
    head: usize,
    tail: usize,
}

impl TimeoutReader {
    fn new() -> Self {
        Self {
            buffer: [0; 1024],
            head: 0,
            tail: 0,
        }
    }
}

impl ByteSource for TimeoutReader {
    fn read_byte(&mut self, deadline: Instant) -> io::Result<Option<u8>> {
        if self.head < self.tail {
            let byte = self.buffer[self.head];
            self.head += 1;
            return Ok(Some(byte));
        }

        loop {
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }

            let timeout = sys::PollTimeout::from_duration(deadline.saturating_duration_since(now));

            match sys::stdin_readiness(timeout)? {
                sys::PollReadiness::Ready => match sys::read_stdin(&mut self.buffer)? {
                    sys::StdinRead::Bytes(bytes_read) => {
                        self.head = 1;
                        self.tail = bytes_read;
                        return Ok(Some(self.buffer[0]));
                    }
                    sys::StdinRead::Eof => return Ok(None),
                    sys::StdinRead::Interrupted => {}
                },
                sys::PollReadiness::Timeout => return Ok(None),
                sys::PollReadiness::Interrupted => {}
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionEvent {
    Resize(TerminalSize),
    Disconnected,
}

/// Drains client bytes between frames after negotiation has finished, so a
/// telnet client's mid-session window resizes (NAWS) reflow the animation and
/// a closed connection ends the session instead of piling up unread input.
/// The historical C implementation never reads the connection after
/// negotiation; this is a fork-specific improvement.
pub(crate) struct SessionInput {
    parser: TelnetParser,
}

/// Upper bound on stdin reads drained per frame, so a flooding client cannot
/// starve rendering; whatever remains waits in the kernel buffer.
const MAX_READS_PER_POLL: usize = 4;

impl SessionInput {
    pub(crate) fn new() -> Self {
        Self {
            parser: TelnetParser::new(),
        }
    }

    /// Parses a chunk of client bytes and returns the last valid window-size
    /// update it contains, if any. Other telnet traffic is consumed for
    /// framing but otherwise ignored.
    fn feed(&mut self, bytes: &[u8]) -> Option<TerminalSize> {
        let mut resized = None;
        for &byte in bytes {
            if let Some(TelnetEvent::Subnegotiation(payload)) = self.parser.push(byte) {
                if let Some(Subnegotiation::WindowSize(size)) = parse_subnegotiation(&payload) {
                    resized = Some(size);
                }
            }
        }
        resized
    }

    /// Non-blocking drain of pending client input. EOF, read failures, and
    /// poll failures on stdin all mean the client is gone, which ends the
    /// session cleanly rather than surfacing as a runtime error.
    pub(crate) fn poll(&mut self) -> Option<SessionEvent> {
        let mut buffer = [0u8; 1024];
        let mut resized = None;

        for _ in 0..MAX_READS_PER_POLL {
            match sys::stdin_readiness(sys::PollTimeout::from_duration(Duration::ZERO)) {
                Ok(sys::PollReadiness::Ready) => match sys::read_stdin(&mut buffer) {
                    Ok(sys::StdinRead::Bytes(count)) => {
                        if let Some(size) = self.feed(&buffer[..count]) {
                            resized = Some(size);
                        }
                    }
                    Ok(sys::StdinRead::Eof) | Err(_) => return Some(SessionEvent::Disconnected),
                    Ok(sys::StdinRead::Interrupted) => break,
                },
                Ok(sys::PollReadiness::Timeout) | Ok(sys::PollReadiness::Interrupted) => break,
                Err(_) => return Some(SessionEvent::Disconnected),
            }
        }

        resized.map(SessionEvent::Resize)
    }
}

struct TelnetState {
    options: OptionCommandTable,
    willack: OptionCommandTable,
    do_set: OptionCommandTable,
    will_set: OptionCommandTable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OptionCommandTable([Option<TelnetCommand>; TELNET_OPTION_COUNT]);

impl OptionCommandTable {
    const fn new() -> Self {
        Self([None; TELNET_OPTION_COUNT])
    }

    fn get(&self, option: TelnetOption) -> Option<TelnetCommand> {
        self.0[option.index()]
    }

    fn set(&mut self, option: TelnetOption, command: TelnetCommand) {
        self.0[option.index()] = Some(command);
    }

    fn get_or_set(&mut self, option: TelnetOption, command: TelnetCommand) -> TelnetCommand {
        if let Some(existing) = self.get(option) {
            existing
        } else {
            self.set(option, command);
            command
        }
    }
}

impl TelnetState {
    fn new() -> Self {
        let mut state = Self {
            options: OptionCommandTable::new(),
            willack: OptionCommandTable::new(),
            do_set: OptionCommandTable::new(),
            will_set: OptionCommandTable::new(),
        };

        state.options.set(TelnetOption::ECHO, TelnetCommand::Wont);
        state.options.set(TelnetOption::SGA, TelnetCommand::Will);
        state
            .options
            .set(TelnetOption::NEW_ENVIRON, TelnetCommand::Wont);
        state.willack.set(TelnetOption::ECHO, TelnetCommand::Do);
        state.willack.set(TelnetOption::SGA, TelnetCommand::Do);
        state.willack.set(TelnetOption::NAWS, TelnetCommand::Do);
        state.willack.set(TelnetOption::TTYPE, TelnetCommand::Do);
        state
            .willack
            .set(TelnetOption::LINEMODE, TelnetCommand::Dont);
        state
            .willack
            .set(TelnetOption::NEW_ENVIRON, TelnetCommand::Do);

        state
    }

    fn push_command(&mut self, out: &mut Vec<u8>, command: TelnetCommand) {
        out.extend_from_slice(&[IAC, command.raw()]);
    }

    fn push_option_command(
        &mut self,
        out: &mut Vec<u8>,
        command: TelnetCommand,
        option: TelnetOption,
    ) {
        match command {
            TelnetCommand::Do | TelnetCommand::Dont => {
                let current = self.do_set.get(option);
                if current != Some(command) {
                    self.do_set.set(option, command);
                    out.extend_from_slice(&[IAC, command.raw(), option.raw()]);
                }
            }
            TelnetCommand::Will | TelnetCommand::Wont => {
                let current = self.will_set.get(option);
                if current != Some(command) {
                    self.will_set.set(option, command);
                    out.extend_from_slice(&[IAC, command.raw(), option.raw()]);
                }
            }
            _ => self.push_command(out, command),
        }
    }
}

#[derive(Debug, Default, Eq, PartialEq)]
pub(crate) struct TelnetInfo {
    pub(crate) term: Option<String>,
    pub(crate) size: Option<TerminalSize>,
}

#[derive(Debug, Eq, PartialEq)]
enum Subnegotiation {
    TerminalType(String),
    WindowSize(TerminalSize),
}

fn parse_subnegotiation(bytes: &[u8]) -> Option<Subnegotiation> {
    match bytes.first().copied().map(TelnetOption::new) {
        Some(TelnetOption::TTYPE) if bytes.len() >= 2 => Some(Subnegotiation::TerminalType(
            String::from_utf8_lossy(&bytes[2..]).into_owned(),
        )),
        Some(TelnetOption::NAWS) if bytes.len() >= 5 => TerminalSize::try_new(
            u16::from_be_bytes([bytes[1], bytes[2]]) as i32,
            u16::from_be_bytes([bytes[3], bytes[4]]) as i32,
        )
        .map(Subnegotiation::WindowSize),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TelnetParserState {
    Data,
    DataIac,
    CommandOption {
        command: TelnetCommand,
        in_subnegotiation: bool,
    },
    Subnegotiation,
    SubnegotiationIac,
}

#[derive(Debug, Eq, PartialEq)]
enum TelnetEvent {
    Command(TelnetCommand),
    Negotiation {
        command: TelnetCommand,
        option: TelnetOption,
    },
    Subnegotiation(Vec<u8>),
}

struct TelnetParser {
    state: TelnetParserState,
    sb: Vec<u8>,
}

impl TelnetParser {
    fn new() -> Self {
        Self {
            state: TelnetParserState::Data,
            sb: Vec::with_capacity(1024),
        }
    }

    fn push(&mut self, byte: u8) -> Option<TelnetEvent> {
        match self.state {
            TelnetParserState::Data => {
                if byte == IAC {
                    self.state = TelnetParserState::DataIac;
                }
                None
            }
            TelnetParserState::DataIac => self.handle_iac(byte, false),
            TelnetParserState::CommandOption {
                command,
                in_subnegotiation,
            } => {
                self.state = if in_subnegotiation {
                    TelnetParserState::Subnegotiation
                } else {
                    TelnetParserState::Data
                };
                Some(TelnetEvent::Negotiation {
                    command,
                    option: TelnetOption::new(byte),
                })
            }
            TelnetParserState::Subnegotiation => {
                if byte == IAC {
                    self.state = TelnetParserState::SubnegotiationIac;
                } else if self.sb.len() < 1023 {
                    self.sb.push(byte);
                }
                None
            }
            TelnetParserState::SubnegotiationIac => self.handle_iac(byte, true),
        }
    }

    fn handle_iac(&mut self, byte: u8, in_subnegotiation: bool) -> Option<TelnetEvent> {
        let command = TelnetCommand::from_byte(byte);
        match command {
            TelnetCommand::Se if in_subnegotiation => {
                self.state = TelnetParserState::Data;
                let bytes = self.sb.clone();
                self.sb.clear();
                Some(TelnetEvent::Subnegotiation(bytes))
            }
            TelnetCommand::Se => {
                self.state = TelnetParserState::Data;
                None
            }
            TelnetCommand::Nop => {
                self.state = if in_subnegotiation {
                    TelnetParserState::Subnegotiation
                } else {
                    TelnetParserState::Data
                };
                Some(TelnetEvent::Command(TelnetCommand::Nop))
            }
            TelnetCommand::Will | TelnetCommand::Wont | TelnetCommand::Do | TelnetCommand::Dont => {
                self.state = TelnetParserState::CommandOption {
                    command,
                    in_subnegotiation,
                };
                None
            }
            TelnetCommand::Sb => {
                self.state = TelnetParserState::Subnegotiation;
                self.sb.clear();
                None
            }
            TelnetCommand::Iac => {
                if in_subnegotiation {
                    self.state = TelnetParserState::Subnegotiation;
                    if self.sb.len() < 1023 {
                        self.sb.push(IAC);
                    }
                } else {
                    self.state = TelnetParserState::Data;
                }
                None
            }
            _ => {
                self.state = if in_subnegotiation {
                    TelnetParserState::Subnegotiation
                } else {
                    TelnetParserState::Data
                };
                None
            }
        }
    }
}

#[derive(Debug, Default, Eq, PartialEq)]
struct NegotiationStep {
    output: Vec<u8>,
    extend_deadline: bool,
}

struct TelnetNegotiation {
    state: TelnetState,
    info: TelnetInfo,
    got_ttype: bool,
    got_naws: bool,
}

impl TelnetNegotiation {
    fn new() -> Self {
        Self {
            state: TelnetState::new(),
            info: TelnetInfo::default(),
            got_ttype: false,
            got_naws: false,
        }
    }

    fn initial_output(&mut self) -> Vec<u8> {
        let mut output = Vec::new();

        for option in 0..=255u8 {
            let option = TelnetOption::new(option);
            if let Some(command) = self.state.options.get(option) {
                self.state.push_option_command(&mut output, command, option);
            }
            if let Some(command) = self.state.willack.get(option) {
                self.state.push_option_command(&mut output, command, option);
            }
        }

        output
    }

    fn is_complete(&self) -> bool {
        self.got_ttype && self.got_naws
    }

    fn into_info(self) -> TelnetInfo {
        self.info
    }

    fn handle_event(&mut self, event: TelnetEvent) -> NegotiationStep {
        let mut step = NegotiationStep::default();

        match event {
            TelnetEvent::Command(TelnetCommand::Nop) => {
                self.state
                    .push_command(&mut step.output, TelnetCommand::Nop);
            }
            TelnetEvent::Command(_) => {}
            TelnetEvent::Negotiation { command, option } => match command {
                TelnetCommand::Will | TelnetCommand::Wont => {
                    self.handle_will_wont(command, option, &mut step.output)
                }
                TelnetCommand::Do | TelnetCommand::Dont => {
                    self.handle_do_dont(option, &mut step.output)
                }
                _ => {}
            },
            TelnetEvent::Subnegotiation(bytes) => {
                if self.handle_subnegotiation(&bytes) {
                    step.extend_deadline = true;
                }
            }
        }

        step
    }

    fn handle_will_wont(
        &mut self,
        command: TelnetCommand,
        option: TelnetOption,
        output: &mut Vec<u8>,
    ) {
        let response = self.state.willack.get_or_set(option, TelnetCommand::Wont);
        self.state.push_option_command(output, response, option);

        if command == TelnetCommand::Will && option == TelnetOption::TTYPE {
            output.extend_from_slice(&[
                IAC,
                TelnetCommand::Sb.raw(),
                TelnetOption::TTYPE.raw(),
                SEND,
                IAC,
                TelnetCommand::Se.raw(),
            ]);
        }
    }

    fn handle_do_dont(&mut self, option: TelnetOption, output: &mut Vec<u8>) {
        let response = self.state.options.get_or_set(option, TelnetCommand::Dont);
        self.state.push_option_command(output, response, option);
    }

    fn handle_subnegotiation(&mut self, bytes: &[u8]) -> bool {
        match parse_subnegotiation(bytes) {
            Some(Subnegotiation::TerminalType(term)) => {
                self.info.term = Some(term);
                self.got_ttype = true;
                true
            }
            Some(Subnegotiation::WindowSize(size)) => {
                self.info.size = Some(size);
                self.got_naws = true;
                true
            }
            None => false,
        }
    }
}

pub(crate) fn negotiate_telnet(out: &mut impl Write) -> io::Result<TelnetInfo> {
    let mut input = TimeoutReader::new();
    negotiate_telnet_with_source(out, &mut input)
}

fn negotiate_telnet_with_source(
    out: &mut impl Write,
    input: &mut impl ByteSource,
) -> io::Result<TelnetInfo> {
    let mut negotiation = TelnetNegotiation::new();
    out.write_all(&negotiation.initial_output())?;
    out.flush()?;

    let mut parser = TelnetParser::new();
    let mut deadline = Instant::now() + Duration::from_secs(1);

    while !negotiation.is_complete() {
        let Some(byte) = input.read_byte(deadline)? else {
            break;
        };

        let Some(event) = parser.push(byte) else {
            continue;
        };
        let step = negotiation.handle_event(event);

        if !step.output.is_empty() {
            out.write_all(&step.output)?;
            out.flush()?;
        }
        if step.extend_deadline {
            deadline = Instant::now() + Duration::from_secs(2);
        }
    }

    Ok(negotiation.into_info())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parser_events(bytes: &[u8]) -> Vec<TelnetEvent> {
        let mut parser = TelnetParser::new();
        bytes.iter().filter_map(|byte| parser.push(*byte)).collect()
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack
            .windows(needle.len())
            .any(|window| window == needle)
    }

    fn command(command: TelnetCommand) -> u8 {
        command.raw()
    }

    fn option(option: TelnetOption) -> u8 {
        option.raw()
    }

    fn option_command(command: TelnetCommand, option: TelnetOption) -> [u8; 3] {
        [IAC, command.raw(), option.raw()]
    }

    fn terminal_type_send() -> [u8; 6] {
        [
            IAC,
            command(TelnetCommand::Sb),
            option(TelnetOption::TTYPE),
            SEND,
            IAC,
            command(TelnetCommand::Se),
        ]
    }

    struct ScriptedByteSource {
        bytes: Vec<u8>,
        position: usize,
    }

    impl ScriptedByteSource {
        fn new(bytes: Vec<u8>) -> Self {
            Self { bytes, position: 0 }
        }
    }

    impl ByteSource for ScriptedByteSource {
        fn read_byte(&mut self, _deadline: Instant) -> io::Result<Option<u8>> {
            let Some(byte) = self.bytes.get(self.position).copied() else {
                return Ok(None);
            };
            self.position += 1;
            Ok(Some(byte))
        }
    }

    #[test]
    fn parses_terminal_type_subnegotiation() {
        assert_eq!(
            parse_subnegotiation(&[option(TelnetOption::TTYPE), 0, b'x', b't', b'e', b'r', b'm']),
            Some(Subnegotiation::TerminalType("xterm".to_string()))
        );
    }

    #[test]
    fn parses_window_size_subnegotiation() {
        assert_eq!(
            parse_subnegotiation(&[option(TelnetOption::NAWS), 0, 120, 0, 40]),
            Some(Subnegotiation::WindowSize(TerminalSize::new(120, 40)))
        );
    }

    #[test]
    fn ignores_zero_window_size_subnegotiation() {
        assert_eq!(
            parse_subnegotiation(&[option(TelnetOption::NAWS), 0, 0, 0, 40]),
            None
        );
        assert_eq!(
            parse_subnegotiation(&[option(TelnetOption::NAWS), 0, 120, 0, 0]),
            None
        );
    }

    #[test]
    fn ignores_incomplete_or_unknown_subnegotiation() {
        assert_eq!(parse_subnegotiation(&[]), None);
        assert_eq!(parse_subnegotiation(&[option(TelnetOption::TTYPE)]), None);
        assert_eq!(
            parse_subnegotiation(&[option(TelnetOption::NAWS), 0, 80, 0]),
            None
        );
        assert_eq!(
            parse_subnegotiation(&[option(TelnetOption::NEW_ENVIRON), 0]),
            None
        );
    }

    #[test]
    fn parser_emits_negotiation_commands() {
        assert_eq!(
            parser_events(&[
                IAC,
                command(TelnetCommand::Will),
                option(TelnetOption::TTYPE)
            ]),
            vec![TelnetEvent::Negotiation {
                command: TelnetCommand::Will,
                option: TelnetOption::TTYPE,
            }]
        );
    }

    #[test]
    fn parser_emits_subnegotiation_payloads() {
        assert_eq!(
            parser_events(&[
                IAC,
                command(TelnetCommand::Sb),
                option(TelnetOption::NAWS),
                0,
                80,
                0,
                24,
                IAC,
                command(TelnetCommand::Se)
            ]),
            vec![TelnetEvent::Subnegotiation(vec![
                option(TelnetOption::NAWS),
                0,
                80,
                0,
                24
            ])]
        );
    }

    #[test]
    fn parser_ignores_subnegotiation_end_outside_subnegotiation() {
        assert_eq!(
            parser_events(&[IAC, command(TelnetCommand::Se)]),
            Vec::<TelnetEvent>::new()
        );
        assert_eq!(
            parser_events(&[
                IAC,
                command(TelnetCommand::Sb),
                option(TelnetOption::TTYPE),
                0,
                b'x',
                IAC,
                command(TelnetCommand::Se),
                IAC,
                command(TelnetCommand::Se)
            ]),
            vec![TelnetEvent::Subnegotiation(vec![
                option(TelnetOption::TTYPE),
                0,
                b'x'
            ])]
        );
    }

    #[test]
    fn parser_keeps_subnegotiation_mode_after_embedded_commands() {
        assert_eq!(
            parser_events(&[
                IAC,
                command(TelnetCommand::Sb),
                b'a',
                IAC,
                command(TelnetCommand::Nop),
                b'b',
                IAC,
                command(TelnetCommand::Se)
            ]),
            vec![
                TelnetEvent::Command(TelnetCommand::Nop),
                TelnetEvent::Subnegotiation(vec![b'a', b'b']),
            ]
        );
    }

    #[test]
    fn parser_ignores_escaped_iac_data() {
        assert_eq!(parser_events(&[IAC, IAC]), Vec::<TelnetEvent>::new());
    }

    #[test]
    fn parser_keeps_escaped_iac_inside_subnegotiation() {
        assert_eq!(
            parser_events(&[
                IAC,
                command(TelnetCommand::Sb),
                option(TelnetOption::TTYPE),
                0,
                b'x',
                IAC,
                IAC,
                b'y',
                IAC,
                command(TelnetCommand::Se)
            ]),
            vec![TelnetEvent::Subnegotiation(vec![
                option(TelnetOption::TTYPE),
                0,
                b'x',
                IAC,
                b'y'
            ])]
        );
    }

    #[test]
    fn initial_output_advertises_supported_options() {
        let mut negotiation = TelnetNegotiation::new();
        let output = negotiation.initial_output();

        assert!(contains(
            &output,
            &option_command(TelnetCommand::Wont, TelnetOption::ECHO)
        ));
        assert!(contains(
            &output,
            &option_command(TelnetCommand::Will, TelnetOption::SGA)
        ));
        assert!(contains(
            &output,
            &option_command(TelnetCommand::Do, TelnetOption::TTYPE)
        ));
        assert!(contains(
            &output,
            &option_command(TelnetCommand::Do, TelnetOption::NAWS)
        ));
        assert!(contains(
            &output,
            &option_command(TelnetCommand::Dont, TelnetOption::LINEMODE)
        ));
        assert!(contains(
            &output,
            &option_command(TelnetCommand::Wont, TelnetOption::NEW_ENVIRON)
        ));
    }

    #[test]
    fn option_command_table_tracks_commands_by_telnet_option() {
        let mut table = OptionCommandTable::new();
        let unknown = TelnetOption::new(200);

        assert_eq!(table.get(unknown), None);
        table.set(unknown, TelnetCommand::Wont);
        assert_eq!(table.get(unknown), Some(TelnetCommand::Wont));
        assert_eq!(
            table.get_or_set(unknown, TelnetCommand::Will),
            TelnetCommand::Wont
        );

        let other = TelnetOption::new(201);
        assert_eq!(
            table.get_or_set(other, TelnetCommand::Will),
            TelnetCommand::Will
        );
        assert_eq!(table.get(other), Some(TelnetCommand::Will));
    }

    #[test]
    fn will_ttype_requests_terminal_type() {
        let mut negotiation = TelnetNegotiation::new();
        let _ = negotiation.initial_output();

        let step = negotiation.handle_event(TelnetEvent::Negotiation {
            command: TelnetCommand::Will,
            option: TelnetOption::TTYPE,
        });

        assert_eq!(step.output, terminal_type_send());
        assert!(!step.extend_deadline);
    }

    #[test]
    fn unknown_options_remain_pass_through_and_are_rejected() {
        let unknown = TelnetOption::new(200);
        let mut negotiation = TelnetNegotiation::new();
        let _ = negotiation.initial_output();

        let step = negotiation.handle_event(TelnetEvent::Negotiation {
            command: TelnetCommand::Will,
            option: unknown,
        });

        assert_eq!(step.output, option_command(TelnetCommand::Wont, unknown));

        let step = negotiation.handle_event(TelnetEvent::Negotiation {
            command: TelnetCommand::Do,
            option: unknown,
        });

        assert_eq!(step.output, option_command(TelnetCommand::Dont, unknown));
    }

    #[test]
    fn subnegotiation_updates_telnet_info() {
        let mut negotiation = TelnetNegotiation::new();

        let step = negotiation.handle_event(TelnetEvent::Subnegotiation(vec![
            option(TelnetOption::TTYPE),
            0,
            b'v',
            b't',
            b'1',
            b'0',
            b'0',
        ]));

        assert!(step.extend_deadline);
        assert_eq!(negotiation.info.term.as_deref(), Some("vt100"));
        assert!(!negotiation.is_complete());

        let step = negotiation.handle_event(TelnetEvent::Subnegotiation(vec![
            option(TelnetOption::NAWS),
            0,
            80,
            0,
            24,
        ]));

        assert!(step.extend_deadline);
        assert_eq!(negotiation.info.size, Some(TerminalSize::new(80, 24)));
        assert!(negotiation.is_complete());
    }

    #[test]
    fn negotiate_telnet_reads_scripted_terminal_info() {
        let mut input = ScriptedByteSource::new(vec![
            IAC,
            command(TelnetCommand::Will),
            option(TelnetOption::TTYPE),
            IAC,
            command(TelnetCommand::Sb),
            option(TelnetOption::TTYPE),
            0,
            b'x',
            b't',
            b'e',
            b'r',
            b'm',
            IAC,
            command(TelnetCommand::Se),
            IAC,
            command(TelnetCommand::Sb),
            option(TelnetOption::NAWS),
            0,
            100,
            0,
            40,
            IAC,
            command(TelnetCommand::Se),
        ]);
        let mut output = Vec::new();

        let info = negotiate_telnet_with_source(&mut output, &mut input).unwrap();

        assert_eq!(info.term.as_deref(), Some("xterm"));
        assert_eq!(info.size, Some(TerminalSize::new(100, 40)));
        assert!(contains(
            &output,
            &option_command(TelnetCommand::Do, TelnetOption::TTYPE)
        ));
        assert!(contains(
            &output,
            &option_command(TelnetCommand::Do, TelnetOption::NAWS)
        ));
        assert!(contains(&output, &terminal_type_send()));
    }

    #[test]
    fn negotiate_telnet_stops_when_scripted_input_ends() {
        let mut input = ScriptedByteSource::new(Vec::new());
        let mut output = Vec::new();

        let info = negotiate_telnet_with_source(&mut output, &mut input).unwrap();

        assert_eq!(info, TelnetInfo::default());
        assert!(contains(
            &output,
            &option_command(TelnetCommand::Do, TelnetOption::TTYPE)
        ));
        assert!(contains(
            &output,
            &option_command(TelnetCommand::Do, TelnetOption::NAWS)
        ));
    }

    fn naws(width_hi: u8, width_lo: u8, height_hi: u8, height_lo: u8) -> Vec<u8> {
        vec![
            IAC,
            command(TelnetCommand::Sb),
            option(TelnetOption::NAWS),
            width_hi,
            width_lo,
            height_hi,
            height_lo,
            IAC,
            command(TelnetCommand::Se),
        ]
    }

    #[test]
    fn session_input_extracts_window_size_updates() {
        let mut input = SessionInput::new();

        assert_eq!(input.feed(b"random keystrokes"), None);
        assert_eq!(
            input.feed(&naws(0, 80, 0, 24)),
            Some(TerminalSize::new(80, 24))
        );

        // The last update in a chunk wins.
        let mut chunk = naws(0, 100, 0, 40);
        chunk.extend(naws(0, 50, 0, 20));
        assert_eq!(input.feed(&chunk), Some(TerminalSize::new(50, 20)));
    }

    #[test]
    fn session_input_handles_split_and_invalid_updates() {
        let mut input = SessionInput::new();

        // A NAWS sequence split across reads parses once complete.
        let bytes = naws(0, 120, 0, 40);
        let (head, tail) = bytes.split_at(4);
        assert_eq!(input.feed(head), None);
        assert_eq!(input.feed(tail), Some(TerminalSize::new(120, 40)));

        // Zero and out-of-bounds dimensions are rejected at the boundary.
        assert_eq!(input.feed(&naws(0, 0, 0, 24)), None);
        assert_eq!(input.feed(&naws(0xff, 0xff, 0, 24)), None);

        // Non-NAWS subnegotiations are consumed but ignored.
        assert_eq!(
            input.feed(&[
                IAC,
                command(TelnetCommand::Sb),
                option(TelnetOption::TTYPE),
                0,
                b'v',
                b't',
                IAC,
                command(TelnetCommand::Se),
            ]),
            None
        );
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

        fn next_byte(&mut self) -> u8 {
            (self.next_u64() >> 24) as u8
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next_u64() % n as u64) as usize
        }
    }

    // Adversarial coverage for the hand-rolled telnet parser. Until a nightly
    // cargo-fuzz target exists, this in-tree generator runs inside the normal
    // test gate. The parser must never panic, must keep the subnegotiation
    // buffer bounded, must parse deterministically, and negotiation over finite
    // input must always terminate.
    #[test]
    fn parser_survives_adversarial_input() {
        let mut seeds: Vec<Vec<u8>> = Vec::new();

        for byte in 0..=255u8 {
            seeds.push(vec![byte]);
            seeds.push(vec![IAC, byte]);
            seeds.push(vec![IAC, command(TelnetCommand::Sb), byte]);
        }

        seeds.push(vec![IAC; 4096]);

        let mut oversized = vec![
            IAC,
            command(TelnetCommand::Sb),
            option(TelnetOption::TTYPE),
            0,
        ];
        oversized.resize(oversized.len() + 8192, b'A');
        seeds.push(oversized);

        seeds.push(vec![
            IAC,
            command(TelnetCommand::Sb),
            option(TelnetOption::NAWS),
            0,
            80,
            IAC,
            command(TelnetCommand::Se),
        ]);

        let mut rng = Rng::new(0x9E37_79B9_7F4A_7C15);
        for _ in 0..5_000 {
            let len = rng.below(64);
            let mut bytes = Vec::with_capacity(len);
            for _ in 0..len {
                // Bias toward IAC so command/subnegotiation paths are exercised often.
                bytes.push(if rng.below(3) == 0 {
                    IAC
                } else {
                    rng.next_byte()
                });
            }
            seeds.push(bytes);
        }

        for bytes in &seeds {
            let mut parser = TelnetParser::new();
            for &byte in bytes {
                let _ = parser.push(byte);
                assert!(parser.sb.len() <= 1023, "subnegotiation buffer overflowed");
            }

            let mut input = ScriptedByteSource::new(bytes.clone());
            let mut output = Vec::new();
            negotiate_telnet_with_source(&mut output, &mut input).unwrap();

            assert_eq!(parser_events(bytes), parser_events(bytes));
        }
    }
}
