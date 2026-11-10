//! The command line parser of cli.py as Python 3.12's argparse runs it (CPython 3.12.16 Lib/argparse.py, with
//! the help wrapping of Lib/textwrap.py): the same parsing steps, option abbreviations, '--' handling, help and
//! usage layout and error messages, so every command line is accepted, refused or answered exactly as 0.4.1 did.
//!
//! Only the features that parser uses are here: options without a value (store_true, help, version), options
//! with one value (str, int, Path), one positional argument and the subcommands. The code follows the Python
//! functions it ports (named in the comments) step by step; the exit codes are argparse's (0 after help or the
//! version, 2 for a usage error).

use std::io::Write;

use crate::pyfmt::{decimal_value, parse_int, repr_str, strip};

/// What an action does with its strings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// -h/--help: print the parser's help and exit 0.
    Help,
    /// --version: print the version text and exit 0.
    Version,
    /// action='store_true'.
    StoreTrue,
    /// One string, kept as it is.
    Text,
    /// One string through Python int().
    Int,
    /// One string through pathlib.Path(), kept as str(Path(text)).
    Path,
    /// The subcommands (add_subparsers, nargs=PARSER).
    Commands,
}

/// nargs: None (one string), 0, or PARSER.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Nargs {
    Zero,
    One,
    Parser,
}

/// One add_argument (or the add_subparsers action).
pub(super) struct Action {
    /// Empty for a positional.
    options: Vec<&'static str>,
    dest: &'static str,
    metavar: Option<&'static str>,
    help: Option<&'static str>,
    kind: Kind,
}

impl Action {
    pub(super) fn option(options: &[&'static str], dest: &'static str, kind: Kind, help: &'static str) -> Action {
        Action {
            options: options.to_vec(),
            dest,
            metavar: None,
            help: Some(help),
            kind,
        }
    }

    pub(super) fn with_metavar(mut self, metavar: &'static str) -> Action {
        self.metavar = Some(metavar);
        self
    }

    pub(super) fn positional(dest: &'static str, kind: Kind, help: Option<&'static str>) -> Action {
        Action {
            options: Vec::new(),
            dest,
            metavar: None,
            help,
            kind,
        }
    }

    fn nargs(&self) -> Nargs {
        match self.kind {
            Kind::Help | Kind::Version | Kind::StoreTrue => Nargs::Zero,
            Kind::Text | Kind::Int | Kind::Path => Nargs::One,
            Kind::Commands => Nargs::Parser,
        }
    }

    /// A positional with nargs None is required, and add_subparsers(required=True).
    fn required(&self) -> bool {
        self.options.is_empty()
    }
}

/// One ArgumentParser.
pub(super) struct Parser {
    prog: String,
    description: Option<&'static str>,
    actions: Vec<Action>,
    /// The subcommands of the Commands action: name, help and parser.
    commands: Vec<(&'static str, &'static str, Parser)>,
    /// The text of the Version action.
    version: Option<String>,
}

impl Parser {
    /// ArgumentParser(prog, description): with -h/--help as its first action.
    pub(super) fn new(prog: String, description: Option<&'static str>) -> Parser {
        Parser {
            prog,
            description,
            actions: vec![Action::option(
                &["-h", "--help"],
                "help",
                Kind::Help,
                "show this help message and exit",
            )],
            commands: Vec::new(),
            version: None,
        }
    }

    pub(super) fn add(&mut self, action: Action) {
        self.actions.push(action);
    }

    pub(super) fn add_version(&mut self, version: String) {
        self.version = Some(version);
        self.add(Action::option(
            &["--version"],
            "version",
            Kind::Version,
            "show program's version number and exit",
        ));
    }

    /// add_subparsers(dest) and then add_parser(name, help=...) for each command; the commands' prog is this
    /// parser's prog and the name (what argparse derives from a usage without positionals).
    pub(super) fn add_commands(&mut self, dest: &'static str, commands: Vec<(&'static str, &'static str, Parser)>) {
        self.add(Action::positional(dest, Kind::Commands, None));
        self.commands = commands;
    }

    /// _option_string_actions in insertion order: (option string, action index).
    fn option_strings(&self) -> impl Iterator<Item = (&'static str, usize)> + '_ {
        self.actions
            .iter()
            .enumerate()
            .flat_map(|(index, action)| action.options.iter().map(move |option| (*option, index)))
    }

    fn lookup(&self, option: &str) -> Option<usize> {
        self.option_strings()
            .find(|(candidate, _)| *candidate == option)
            .map(|(_, index)| index)
    }
}

/// A parsed value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Val {
    Bool(bool),
    Int(i128),
    Text(String),
}

/// The argparse Namespace: the values that were set, by dest.
#[derive(Default, Debug)]
pub(super) struct Namespace {
    values: Vec<(&'static str, Val)>,
    /// _UNRECOGNIZED_ARGS_ATTR: strings a subcommand did not recognize, for the top parser to report.
    unrecognized: Vec<String>,
}

impl Namespace {
    fn set(&mut self, dest: &'static str, value: Val) {
        match self.values.iter_mut().find(|(name, _)| *name == dest) {
            Some(entry) => entry.1 = value,
            None => self.values.push((dest, value)),
        }
    }

    pub(super) fn get(&self, dest: &str) -> Option<&Val> {
        self.values
            .iter()
            .find(|(name, _)| *name == dest)
            .map(|(_, value)| value)
    }
}

/// Where the parser writes, and the terminal width it formats for.
pub(super) struct Io<'a> {
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
    /// shutil.get_terminal_size().columns, read each time a formatter is made, as argparse does.
    pub columns: &'a dyn Fn() -> i128,
}

/// Why parsing stopped: an ArgumentError (its str()), or sys.exit after help, the version or an error.
enum Stop {
    Error(String),
    Exit(i32),
}

/// ArgumentError(action, message): "argument <name>: <message>".
fn argument_error(action: &Action, message: String) -> Stop {
    Stop::Error(format!("argument {}: {message}", action_name(action)))
}

/// _get_action_name: the option strings joined by '/', else the metavar, else the dest.
fn action_name(action: &Action) -> String {
    if !action.options.is_empty() {
        action.options.join("/")
    } else if let Some(metavar) = action.metavar {
        metavar.to_string()
    } else {
        action.dest.to_string()
    }
}

/// One interpretation of an option string: (action, option string, sep, explicit argument).
#[derive(Clone, Debug)]
struct OptionTuple {
    action: Option<usize>,
    option: String,
    /// None, Some("") (a short option's attached argument) or Some("=").
    sep: Option<&'static str>,
    explicit: Option<String>,
}

/// `^-\d+$|^-\d*\.\d+$` (_negative_number_matcher): Unicode digits, and `$` also matches before a final newline.
fn looks_negative(arg: &str) -> bool {
    let Some(rest) = arg.strip_prefix('-') else {
        return false;
    };
    let rest = rest.strip_suffix('\n').unwrap_or(rest);
    let digits = |text: &str| text.chars().all(|c| decimal_value(c).is_some());
    if !rest.is_empty() && digits(rest) {
        return true;
    }
    match rest.split_once('.') {
        Some((whole, fraction)) => digits(whole) && !fraction.is_empty() && digits(fraction),
        None => false,
    }
}

/// str.partition('='): (before, Some("="), after) or (text, None, None).
fn partition(text: &str) -> (&str, Option<&'static str>, Option<String>) {
    match text.split_once('=') {
        Some((before, after)) => (before, Some("="), Some(after.to_string())),
        None => (text, None, None),
    }
}

impl Parser {
    /// _parse_optional: None for a positional string, else the option tuples (one with no action for an unknown
    /// option).
    fn parse_optional(&self, arg: &str) -> Option<Vec<OptionTuple>> {
        if !arg.starts_with('-') {
            return None;
        }
        if let Some(index) = self.lookup(arg) {
            return Some(vec![OptionTuple {
                action: Some(index),
                option: arg.to_string(),
                sep: None,
                explicit: None,
            }]);
        }
        if arg.chars().count() == 1 {
            return None;
        }
        if let (option, Some(sep), explicit) = partition(arg) {
            if let Some(index) = self.lookup(option) {
                return Some(vec![OptionTuple {
                    action: Some(index),
                    option: option.to_string(),
                    sep: Some(sep),
                    explicit,
                }]);
            }
        }
        let tuples = self.option_tuples(arg);
        if !tuples.is_empty() {
            return Some(tuples);
        }
        // No option of these parsers looks like a negative number.
        if looks_negative(arg) || arg.contains(' ') {
            return None;
        }
        Some(vec![OptionTuple {
            action: None,
            option: arg.to_string(),
            sep: None,
            explicit: None,
        }])
    }

    /// _get_option_tuples (allow_abbrev=True): the options this string can be an abbreviation of.
    fn option_tuples(&self, arg: &str) -> Vec<OptionTuple> {
        let chars: Vec<char> = arg.chars().collect();
        let (prefix, sep, explicit) = partition(arg);
        let mut result = Vec::new();
        if chars[1] == '-' {
            for (option, index) in self.option_strings() {
                if option.starts_with(prefix) {
                    result.push(OptionTuple {
                        action: Some(index),
                        option: option.to_string(),
                        sep,
                        explicit: explicit.clone(),
                    });
                }
            }
        } else {
            // A single-dash option can carry its argument: -xVALUE.
            let short: String = chars[..2].iter().collect();
            let short_explicit: String = chars[2..].iter().collect();
            for (option, index) in self.option_strings() {
                if option == short {
                    result.push(OptionTuple {
                        action: Some(index),
                        option: option.to_string(),
                        sep: Some(""),
                        explicit: Some(short_explicit.clone()),
                    });
                } else if option.starts_with(prefix) {
                    result.push(OptionTuple {
                        action: Some(index),
                        option: option.to_string(),
                        sep,
                        explicit: explicit.clone(),
                    });
                }
            }
        }
        result
    }

    /// _match_argument for an option: how many of the following strings ('A' in the pattern) it takes.
    fn match_option(&self, action: &Action, pattern: &[u8]) -> Result<usize, Stop> {
        match action.nargs() {
            Nargs::Zero => Ok(0),
            _ if pattern.first() == Some(&b'A') => Ok(1),
            _ => Err(argument_error(action, "expected one argument".to_string())),
        }
    }

    /// _match_arguments_partial for the one positional left (if any): its string count, or None (no match).
    fn match_positional(action: &Action, pattern: &[u8]) -> Option<usize> {
        // '(-*A-*)' for nargs None, '(-*A[-AO]*)' for PARSER.
        let leading = pattern.iter().take_while(|&&c| c == b'-').count();
        if pattern.get(leading) != Some(&b'A') {
            return None;
        }
        Some(match action.nargs() {
            Nargs::Parser => pattern.len(),
            _ => leading + 1 + pattern[leading + 1..].iter().take_while(|&&c| c == b'-').count(),
        })
    }

    /// parse_args: parse_known_args, then "unrecognized arguments" for anything left.
    pub(super) fn parse_args(&self, args: &[String], io: &mut Io) -> Result<Namespace, i32> {
        let (namespace, extras) = self.parse_known_args(args, io)?;
        if !extras.is_empty() {
            return Err(self.error(&format!("unrecognized arguments: {}", extras.join(" ")), io));
        }
        Ok(namespace)
    }

    /// parse_known_args (_parse_known_args2): an ArgumentError becomes this parser's usage error.
    fn parse_known_args(&self, args: &[String], io: &mut Io) -> Result<(Namespace, Vec<String>), i32> {
        let mut namespace = Namespace::default();
        match self.parse_known(args, &mut namespace, io) {
            Ok(mut extras) => {
                extras.append(&mut namespace.unrecognized);
                Ok((namespace, extras))
            }
            Err(Stop::Error(message)) => Err(self.error(&message, io)),
            Err(Stop::Exit(code)) => Err(code),
        }
    }

    /// _parse_known_args: options and positionals in turn, until the last option string.
    fn parse_known(&self, args: &[String], namespace: &mut Namespace, io: &mut Io) -> Result<Vec<String>, Stop> {
        // 'O' for an option string, 'A' for an argument, '-' for the first '--' (all after it are arguments).
        let mut option_indices: Vec<(usize, Vec<OptionTuple>)> = Vec::new();
        let mut pattern = Vec::with_capacity(args.len());
        for (index, arg) in args.iter().enumerate() {
            if arg == "--" {
                pattern.push(b'-');
                pattern.extend(std::iter::repeat_n(b'A', args.len() - index - 1));
                break;
            }
            match self.parse_optional(arg) {
                None => pattern.push(b'A'),
                Some(tuples) => {
                    option_indices.push((index, tuples));
                    pattern.push(b'O');
                }
            }
        }
        let mut state = State {
            parser: self,
            args,
            pattern: &pattern,
            option_indices: &option_indices,
            seen: vec![false; self.actions.len()],
            extras: Vec::new(),
            positionals: self
                .actions
                .iter()
                .enumerate()
                .filter(|(_, action)| action.options.is_empty())
                .map(|(index, _)| index)
                .collect(),
            namespace,
        };
        let mut start = 0;
        let max_option = option_indices.last().map(|(index, _)| *index);
        while max_option.is_some_and(|max| start <= max) {
            let next = option_indices
                .iter()
                .map(|(index, _)| *index)
                .find(|index| *index >= start)
                .unwrap_or(start);
            if start != next {
                let end = state.consume_positionals(start, io)?;
                if end > start {
                    start = end;
                    continue;
                }
                start = end;
            }
            if !option_indices.iter().any(|(index, _)| *index == start) {
                state.extras.extend_from_slice(&args[start..next]);
                start = next;
            }
            start = state.consume_optional(start, io)?;
        }
        let stop = state.consume_positionals(start, io)?;
        state.extras.extend_from_slice(&args[stop..]);
        let required: Vec<String> = self
            .actions
            .iter()
            .zip(&state.seen)
            .filter(|(action, seen)| action.required() && !**seen)
            .map(|(action, _)| action_name(action))
            .collect();
        if !required.is_empty() {
            return Err(Stop::Error(format!(
                "the following arguments are required: {}",
                required.join(", ")
            )));
        }
        Ok(state.extras)
    }

    /// _get_values and the action call.
    fn take_action(
        &self,
        index: usize,
        strings: Vec<String>,
        namespace: &mut Namespace,
        io: &mut Io,
    ) -> Result<(), Stop> {
        let action = &self.actions[index];
        match action.kind {
            Kind::Help => {
                let help = self.format_help((io.columns)());
                let _ = io.out.write_all(help.as_bytes());
                Err(Stop::Exit(0))
            }
            Kind::Version => {
                // formatter.add_text(version): the text is wrapped like a description.
                let formatter = Formatter::new(&self.prog, (io.columns)());
                let text = formatter.finish(&[formatter.text(self.version.as_deref().unwrap_or(""))]);
                let _ = io.out.write_all(text.as_bytes());
                Err(Stop::Exit(0))
            }
            Kind::StoreTrue => {
                namespace.set(action.dest, Val::Bool(true));
                Ok(())
            }
            Kind::Text => {
                namespace.set(action.dest, Val::Text(strings[0].clone()));
                Ok(())
            }
            Kind::Path => {
                namespace.set(action.dest, Val::Text(super::path_text(&strings[0], cfg!(windows))));
                Ok(())
            }
            Kind::Int => match parse_int(&strings[0], 10) {
                Some(value) => {
                    namespace.set(action.dest, Val::Int(value));
                    Ok(())
                }
                None => Err(argument_error(
                    action,
                    format!("invalid int value: {}", repr_str(&strings[0])),
                )),
            },
            Kind::Commands => {
                let name = &strings[0];
                let Some((_, _, parser)) = self.commands.iter().find(|(command, _, _)| command == name) else {
                    let choices: Vec<&str> = self.commands.iter().map(|(command, _, _)| *command).collect();
                    return Err(argument_error(
                        action,
                        format!(
                            "invalid choice: {} (choose from {})",
                            repr_str(name),
                            choices.join(", ")
                        ),
                    ));
                };
                namespace.set(action.dest, Val::Text(name.clone()));
                let (sub, extras) = parser.parse_known_args(&strings[1..], io).map_err(Stop::Exit)?;
                for (dest, value) in sub.values {
                    namespace.set(dest, value);
                }
                namespace.unrecognized.extend(extras);
                Ok(())
            }
        }
    }

    /// error(): the usage and "<prog>: error: <message>" on stderr, exit status 2.
    fn error(&self, message: &str, io: &mut Io) -> i32 {
        let usage = self.format_usage((io.columns)());
        let _ = io.err.write_all(usage.as_bytes());
        let _ = writeln!(io.err, "{}: error: {message}", self.prog);
        2
    }
}

/// The loop state of _parse_known_args.
struct State<'p, 'n> {
    parser: &'p Parser,
    args: &'p [String],
    pattern: &'p [u8],
    option_indices: &'p [(usize, Vec<OptionTuple>)],
    seen: Vec<bool>,
    extras: Vec<String>,
    /// The positional actions not consumed yet.
    positionals: Vec<usize>,
    namespace: &'n mut Namespace,
}

impl State<'_, '_> {
    fn take(&mut self, index: usize, strings: Vec<String>, io: &mut Io) -> Result<(), Stop> {
        self.seen[index] = true;
        self.parser.take_action(index, strings, self.namespace, io)
    }

    /// consume_optional: the option at `start` and its argument; returns where the next string starts.
    fn consume_optional(&mut self, start: usize, io: &mut Io) -> Result<usize, Stop> {
        let tuples = &self
            .option_indices
            .iter()
            .find(|(index, _)| *index == start)
            .expect("consume_optional is only called at an option index")
            .1;
        if tuples.len() > 1 {
            let options: Vec<&str> = tuples.iter().map(|tuple| tuple.option.as_str()).collect();
            return Err(Stop::Error(format!(
                "ambiguous option: {} could match {}",
                self.args[start],
                options.join(", ")
            )));
        }
        let OptionTuple {
            mut action,
            mut option,
            mut sep,
            mut explicit,
        } = tuples[0].clone();
        let parser = self.parser;
        let mut taken: Vec<(usize, Vec<String>)> = Vec::new();
        let stop;
        loop {
            let Some(index) = action else {
                self.extras.push(self.args[start].clone());
                return Ok(start + 1);
            };
            let current = &parser.actions[index];
            if let Some(argument) = explicit.clone() {
                let count = parser.match_option(current, b"A")?;
                let second = option.chars().nth(1);
                if count == 0 && second != Some('-') && !argument.is_empty() {
                    // -xyz: a single-dash option without a value followed by more single-dash options.
                    let first = argument.chars().next().unwrap_or('-');
                    if sep.is_some_and(|sep| !sep.is_empty()) || first == '-' {
                        return Err(argument_error(
                            current,
                            format!("ignored explicit argument {}", repr_str(&argument)),
                        ));
                    }
                    taken.push((index, Vec::new()));
                    let dash = option.chars().next().unwrap_or('-');
                    option = format!("{dash}{first}");
                    if let Some(next) = parser.lookup(&option) {
                        action = Some(next);
                        let rest: String = argument.chars().skip(1).collect();
                        if rest.is_empty() {
                            sep = None;
                            explicit = None;
                        } else if let Some(value) = rest.strip_prefix('=') {
                            sep = Some("=");
                            explicit = Some(value.to_string());
                        } else {
                            sep = Some("");
                            explicit = Some(rest);
                        }
                    } else {
                        self.extras.push(format!("{dash}{argument}"));
                        stop = start + 1;
                        break;
                    }
                } else if count == 1 {
                    stop = start + 1;
                    taken.push((index, vec![argument]));
                    break;
                } else {
                    return Err(argument_error(
                        current,
                        format!("ignored explicit argument {}", repr_str(&argument)),
                    ));
                }
            } else {
                let first = start + 1;
                let count = parser.match_option(current, &self.pattern[first..])?;
                stop = first + count;
                taken.push((index, self.args[first..stop].to_vec()));
                break;
            }
        }
        for (index, strings) in taken {
            self.take(index, strings, io)?;
        }
        Ok(stop)
    }

    /// consume_positionals: as many positionals as match from `start`; returns where they stopped.
    fn consume_positionals(&mut self, start: usize, io: &mut Io) -> Result<usize, Stop> {
        let Some(&index) = self.positionals.first() else {
            return Ok(start);
        };
        let action = &self.parser.actions[index];
        let Some(count) = Parser::match_positional(action, &self.pattern[start..]) else {
            return Ok(start);
        };
        let mut strings = self.args[start..start + count].to_vec();
        // The first '--' is dropped (for PARSER only when it comes first).
        let marker = self.pattern[start..start + count].iter().position(|&c| c == b'-');
        let drop = match action.nargs() {
            Nargs::Parser => marker == Some(0),
            _ => marker.is_some(),
        };
        if drop {
            if let Some(at) = strings.iter().position(|s| s == "--") {
                strings.remove(at);
            }
        }
        self.positionals.remove(0);
        self.take(index, strings, io)?;
        Ok(start + count)
    }
}

// Help formatting (HelpFormatter) ------------------------------------------------------------------------------

/// HelpFormatter(prog) with its width: the terminal columns minus 2.
struct Formatter<'a> {
    prog: &'a str,
    width: i128,
    max_help_position: i128,
}

/// The length of a text in characters (Python len()).
fn len(text: &str) -> i128 {
    text.chars().count() as i128
}

fn spaces(count: i128) -> String {
    " ".repeat(usize::try_from(count).unwrap_or(0))
}

impl<'a> Formatter<'a> {
    fn new(prog: &'a str, columns: i128) -> Formatter<'a> {
        let width = columns.saturating_sub(2);
        Formatter {
            prog,
            width,
            max_help_position: 24.min(width.saturating_sub(20).max(4)),
        }
    }

    /// format_help(): the items joined, runs of 3+ newlines made 2, outer newlines stripped, one newline added.
    fn finish(&self, items: &[String]) -> String {
        let joined = items.concat();
        if joined.is_empty() {
            return joined;
        }
        let mut collapsed = String::with_capacity(joined.len());
        let mut newlines = 0;
        for c in joined.chars() {
            if c == '\n' {
                newlines += 1;
                continue;
            }
            collapsed.push_str(if newlines >= 3 { "\n\n" } else { &"\n\n"[..newlines] });
            newlines = 0;
            collapsed.push(c);
        }
        collapsed.push_str(if newlines >= 3 { "\n\n" } else { &"\n\n"[..newlines] });
        format!("{}\n", collapsed.trim_matches('\n'))
    }

    /// _format_text at indent 0: the text filled to the width (at least 11), then a blank line.
    fn text(&self, text: &str) -> String {
        format!("{}\n\n", wrap(text, self.width.max(11)).join("\n"))
    }

    /// The usage parts of the options (first) and of the positionals: "[-h]", "[--gpu INDEX]", "payload",
    /// "{status,...}" and "...".
    fn usage_parts(parser: &Parser) -> (Vec<String>, Vec<String>) {
        let mut optionals = Vec::new();
        let mut positionals = Vec::new();
        for action in &parser.actions {
            if action.options.is_empty() {
                positionals.push(invocation(parser, action));
                if action.nargs() == Nargs::Parser {
                    positionals.push("...".to_string());
                }
            } else if action.nargs() == Nargs::Zero {
                optionals.push(format!("[{}]", action.options[0]));
            } else {
                optionals.push(format!("[{} {}]", action.options[0], metavar(action)));
            }
        }
        (optionals, positionals)
    }

    /// _format_usage with the 'usage: ' prefix.
    fn usage(&self, parser: &Parser) -> String {
        let prefix = "usage: ";
        let prog = self.prog.to_string();
        let (optionals, positionals) = Formatter::usage_parts(parser);
        let all: Vec<String> = optionals.iter().chain(&positionals).cloned().collect();
        let mut usage = if all.is_empty() {
            prog.clone()
        } else {
            format!("{prog} {}", all.join(" "))
        };
        let text_width = self.width;
        if len(prefix) + len(&usage) > text_width {
            let get_lines = |parts: &[String], indent: &str, prefix: Option<&str>| -> Vec<String> {
                let mut lines = Vec::new();
                let mut line: Vec<&str> = Vec::new();
                let indent_length = len(indent);
                let mut line_len = prefix.map_or(indent_length, len) - 1;
                for part in parts {
                    if line_len + 1 + len(part) > text_width && !line.is_empty() {
                        lines.push(format!("{indent}{}", line.join(" ")));
                        line.clear();
                        line_len = indent_length - 1;
                    }
                    line.push(part);
                    line_len += len(part) + 1;
                }
                if !line.is_empty() {
                    lines.push(format!("{indent}{}", line.join(" ")));
                }
                if prefix.is_some() {
                    if let Some(first) = lines.first_mut() {
                        *first = first.chars().skip(indent.chars().count()).collect();
                    }
                }
                lines
            };
            // A short prog is followed by the parts; a long one gets a line of its own (0.75 * text_width).
            let lines = if 4 * (len(prefix) + len(&prog)) <= 3 * text_width {
                let indent = spaces(len(prefix) + len(&prog) + 1);
                if !optionals.is_empty() {
                    let mut first = vec![prog.clone()];
                    first.extend(optionals.iter().cloned());
                    let mut lines = get_lines(&first, &indent, Some(prefix));
                    lines.extend(get_lines(&positionals, &indent, None));
                    lines
                } else if !positionals.is_empty() {
                    let mut first = vec![prog.clone()];
                    first.extend(positionals.iter().cloned());
                    get_lines(&first, &indent, Some(prefix))
                } else {
                    vec![prog.clone()]
                }
            } else {
                let indent = spaces(len(prefix));
                let mut lines = get_lines(&all, &indent, None);
                if lines.len() > 1 {
                    lines = get_lines(&optionals, &indent, None);
                    lines.extend(get_lines(&positionals, &indent, None));
                }
                let mut with_prog = vec![prog.clone()];
                with_prog.extend(lines);
                with_prog
            };
            usage = lines.join("\n");
        }
        format!("{prefix}{usage}\n\n")
    }

    /// _format_action: the invocation, padded or on its own line, then the help wrapped at help_position.
    fn action(&self, entry: &Entry, indent: i128, max_length: i128) -> String {
        let help_position = (max_length + 2).min(self.max_help_position);
        let help_width = (self.width - help_position).max(11);
        let action_width = help_position - indent - 2;
        let header = &entry.invocation;
        let help = entry.help.filter(|help| !help.is_empty());
        let mut out = String::new();
        let mut indent_first = 0;
        match help {
            None => out.push_str(&format!("{}{header}\n", spaces(indent))),
            Some(_) if len(header) <= action_width => {
                let padding = spaces(action_width - len(header));
                out.push_str(&format!("{}{header}{padding}  ", spaces(indent)));
            }
            Some(_) => {
                out.push_str(&format!("{}{header}\n", spaces(indent)));
                indent_first = help_position;
            }
        }
        match help {
            Some(help) if !strip(help).is_empty() => {
                let lines = wrap(help, help_width);
                for (number, line) in lines.iter().enumerate() {
                    let at = if number == 0 { indent_first } else { help_position };
                    out.push_str(&format!("{}{line}\n", spaces(at)));
                }
            }
            _ if !out.ends_with('\n') => out.push('\n'),
            _ => {}
        }
        for sub in &entry.subactions {
            out.push_str(&self.action(sub, indent + 2, max_length));
        }
        out
    }
}

/// What _format_action shows of an action: its invocation, help and the subcommands under it.
struct Entry {
    invocation: String,
    help: Option<&'static str>,
    subactions: Vec<Entry>,
}

/// The metavar of an option with a value: its metavar, else the dest in upper case.
fn metavar(action: &Action) -> String {
    action
        .metavar
        .map_or_else(|| action.dest.to_uppercase(), str::to_string)
}

/// _format_action_invocation.
fn invocation(parser: &Parser, action: &Action) -> String {
    if action.options.is_empty() {
        if let Some(metavar) = action.metavar {
            return metavar.to_string();
        }
        if action.kind == Kind::Commands {
            let names: Vec<&str> = parser.commands.iter().map(|(name, _, _)| *name).collect();
            return format!("{{{}}}", names.join(","));
        }
        return action.dest.to_string();
    }
    if action.nargs() == Nargs::Zero {
        return action.options.join(", ");
    }
    let value = metavar(action);
    let parts: Vec<String> = action
        .options
        .iter()
        .map(|option| format!("{option} {value}"))
        .collect();
    parts.join(", ")
}

impl Parser {
    fn entry(&self, action: &Action) -> Entry {
        let subactions = if action.kind == Kind::Commands {
            self.commands
                .iter()
                .map(|(name, help, _)| Entry {
                    invocation: name.to_string(),
                    help: Some(help),
                    subactions: Vec::new(),
                })
                .collect()
        } else {
            Vec::new()
        };
        Entry {
            invocation: invocation(self, action),
            help: action.help,
            subactions,
        }
    }

    /// format_usage(): "usage: ..." and a newline.
    pub(super) fn format_usage(&self, columns: i128) -> String {
        let formatter = Formatter::new(&self.prog, columns);
        formatter.finish(&[formatter.usage(self)])
    }

    /// format_help(): usage, description, then the positional arguments and options sections.
    pub(super) fn format_help(&self, columns: i128) -> String {
        let formatter = Formatter::new(&self.prog, columns);
        let mut items = vec![formatter.usage(self)];
        if let Some(description) = self.description {
            items.push(formatter.text(description));
        }
        let groups: [(&str, Vec<Entry>); 2] = [
            (
                "positional arguments",
                self.actions
                    .iter()
                    .filter(|action| action.options.is_empty())
                    .map(|action| self.entry(action))
                    .collect(),
            ),
            (
                "options",
                self.actions
                    .iter()
                    .filter(|action| !action.options.is_empty())
                    .map(|action| self.entry(action))
                    .collect(),
            ),
        ];
        // add_argument: the longest invocation (with its indentation) over all sections.
        let mut max_length = 0;
        for (_, entries) in &groups {
            for entry in entries {
                max_length = max_length.max(len(&entry.invocation) + 2);
                for sub in &entry.subactions {
                    max_length = max_length.max(len(&sub.invocation) + 4);
                }
            }
        }
        for (title, entries) in &groups {
            let body: String = entries
                .iter()
                .map(|entry| formatter.action(entry, 2, max_length))
                .collect();
            if !body.is_empty() {
                items.push(format!("\n{title}:\n{body}\n"));
            }
        }
        formatter.finish(&items)
    }
}

// textwrap ------------------------------------------------------------------------------------------------------

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `[^\d\W]`: a word character that is not a digit.
fn is_letter(c: char) -> bool {
    is_word(c) && decimal_value(c).is_none()
}

/// `[\w!"'&.,?]`.
fn is_word_punct(c: char) -> bool {
    is_word(c) || "!\"'&.,?".contains(c)
}

/// textwrap's whitespace: '\t\n\x0b\x0c\r '.
fn is_wrap_space(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\x0b' | '\x0c' | '\r' | ' ')
}

/// TextWrapper.wordsep_re.split (break_on_hyphens): whitespace runs, words, and words cut after the hyphen of a
/// compound word ("machine-" "readable").
fn chunks(text: &[char]) -> Vec<Vec<char>> {
    let n = text.len();
    let at = |i: usize| text.get(i).copied();
    let letter = |i: usize| at(i).is_some_and(is_letter);
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        if is_wrap_space(text[i]) {
            let end = i + text[i..].iter().take_while(|&&c| is_wrap_space(c)).count();
            out.push(text[i..end].to_vec());
            i = end;
            continue;
        }
        // An em-dash between words: (?<=wp) -{2,} (?=\w).
        if text[i] == '-' && i > 0 && is_word_punct(text[i - 1]) {
            let end = i + text[i..].iter().take_while(|&&c| c == '-').count();
            if end - i >= 2 && at(end).is_some_and(is_word) {
                out.push(text[i..end].to_vec());
                i = end;
                continue;
            }
        }
        // The shortest run of non-whitespace followed by a compound-word hyphen, the end of the word or an em-dash.
        let mut j = i + 1;
        loop {
            if at(j) == Some('-') {
                let behind = (j >= 2 && letter(j - 2) && letter(j - 1))
                    || (j >= 3 && letter(j - 3) && at(j - 2) == Some('-') && letter(j - 1));
                let ahead = letter(j + 1) && (letter(j + 2) || (at(j + 2) == Some('-') && letter(j + 3)));
                if behind && ahead {
                    out.push(text[i..=j].to_vec());
                    i = j + 1;
                    break;
                }
            }
            if j == n || is_wrap_space(text[j]) {
                out.push(text[i..j].to_vec());
                i = j;
                break;
            }
            if is_word_punct(text[j - 1]) && text[j] == '-' && at(j + 1) == Some('-') {
                let end = j + text[j..].iter().take_while(|&&c| c == '-').count();
                if at(end).is_some_and(is_word) {
                    out.push(text[i..j].to_vec());
                    i = j;
                    break;
                }
            }
            j += 1;
        }
    }
    out
}

/// HelpFormatter._split_lines: ASCII whitespace runs made one space, stripped, then textwrap.wrap(text, width)
/// with its defaults (width > 0 here: argparse passes at least 11).
fn wrap(text: &str, width: i128) -> Vec<String> {
    let mut collapsed = String::with_capacity(text.len());
    let mut in_space = false;
    for c in text.chars() {
        if c.is_ascii_whitespace() || c == '\x0b' {
            if !in_space {
                collapsed.push(' ');
            }
            in_space = true;
        } else {
            collapsed.push(c);
            in_space = false;
        }
    }
    let text: Vec<char> = strip(&collapsed).chars().collect();
    let is_blank = |chunk: &[char]| chunk.iter().all(|&c| crate::pyfmt::is_space(c));
    let size = |chunk: &[char]| chunk.len() as i128;
    // _wrap_chunks with the chunks as a stack (last = next).
    let mut stack: Vec<Vec<char>> = chunks(&text);
    stack.reverse();
    let mut lines: Vec<String> = Vec::new();
    while !stack.is_empty() {
        let mut line: Vec<Vec<char>> = Vec::new();
        let mut used: i128 = 0;
        if !lines.is_empty() && stack.last().is_some_and(|chunk| is_blank(chunk)) {
            stack.pop();
        }
        while let Some(chunk) = stack.last() {
            if used + size(chunk) <= width {
                used += size(chunk);
                line.extend(stack.pop());
            } else {
                break;
            }
        }
        if stack.last().is_some_and(|chunk| size(chunk) > width) {
            // _handle_long_word: break it, after its last hyphen when one fits.
            let space_left = if width < 1 { 1 } else { width - used };
            if let Some(chunk) = stack.last_mut() {
                let mut end = usize::try_from(space_left).unwrap_or(0).min(chunk.len());
                if size(chunk) > space_left {
                    if let Some(hyphen) = chunk[..end].iter().rposition(|&c| c == '-') {
                        if hyphen > 0 && chunk[..hyphen].iter().any(|&c| c != '-') {
                            end = hyphen + 1;
                        }
                    }
                }
                let rest = chunk.split_off(end);
                line.push(std::mem::replace(chunk, rest));
            }
        }
        if line.last().is_some_and(|chunk| is_blank(chunk)) {
            line.pop();
        }
        if !line.is_empty() {
            lines.push(line.concat().into_iter().collect());
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        chunks(&chars)
            .into_iter()
            .map(|chunk| chunk.into_iter().collect())
            .collect()
    }

    #[test]
    fn chunks_split_compound_words_as_textwrap_does() {
        assert_eq!(
            words("Look, goof-ball -- use the -b option!"),
            ["Look,", " ", "goof-", "ball", " ", "--", " ", "use", " ", "the", " ", "-b", " ", "option!"]
        );
        assert_eq!(words("vbios-loader: x"), ["vbios-", "loader:", " ", "x"]);
        assert_eq!(words("machine-readable"), ["machine-", "readable"]);
        assert_eq!(words("a-b x--y 0.5.0"), ["a-b", " ", "x", "--", "y", " ", "0.5.0"]);
    }

    #[test]
    fn wrap_breaks_long_words_after_a_hyphen() {
        assert_eq!(wrap("vbios-loader 0.5.0", 11), ["vbios-", "loader", "0.5.0"]);
        assert_eq!(wrap("  a\t\nb  ", 70), ["a b"]);
        assert_eq!(wrap("compatibility: x", 11), ["compatibili", "ty: x"]);
    }

    #[test]
    fn negative_numbers_follow_the_regex() {
        for text in ["-1", "-12", "-1.5", "-.5", "-\u{661}", "-5\n"] {
            assert!(looks_negative(text), "{text:?}");
        }
        for text in ["-", "-x", "-1.", "-1..2", "-5\n\n", "--1", "-1e3"] {
            assert!(!looks_negative(text), "{text:?}");
        }
    }
}
