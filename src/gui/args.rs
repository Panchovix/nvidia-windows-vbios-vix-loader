//! The command line of vbios-loader-gui as gui.py's argparse parser read and printed it (Python 3.12, which built
//! the 0.4.1 executables): `--version`, `--lang {en,es}`, `--theme {auto,light,dark}` and `-h`/`--help`, with
//! argparse's prefix abbreviations (`--la es`), `--opt=value`, `--`, its usage and help texts wrapped to the terminal
//! width, and its error messages.
//!
//! Only this parser's options are handled, but argparse's algorithm is followed step by step (option lookup,
//! explicit arguments, single-dash chains such as `-hx`, the order in which errors and `--help` are reached), so the
//! outcome and every printed byte are argparse's. The tests compare with output recorded from Python 3.12.

use crate::controller::{Language, Theme};

/// The first line of gui.py's docstring, the parser's description.
pub const DESCRIPTION: &str =
    "vbios-loader-gui: a window to pick a GPU and a VBIOS file, then load, remove or verify the override.";

/// The terminal width argparse falls back to when it cannot ask a terminal (shutil.get_terminal_size).
pub const DEFAULT_COLUMNS: i64 = 80;

const PROG: &str = super::PROG;
const LANG_CHOICES: &[&str] = &["en", "es"];
const THEME_CHOICES: &[&str] = &["auto", "light", "dark"];

/// What the arguments ask for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Parsed {
    /// Open the window.
    Run(Options),
    /// Print `help_text` to stdout, exit 0.
    Help,
    /// Print `version_text` to stdout, exit 0.
    Version,
    /// Print `error_text` to stderr, exit 2.
    Error(ArgError),
}

/// The window's options: `--lang` (None: the Windows display language) and `--theme` (default auto).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub lang: Option<Language>,
    pub theme: Theme,
}

/// An argparse error; `message` gives the text after "vbios-loader-gui: error: ".
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArgError {
    InvalidChoice {
        option: &'static str,
        value: String,
        choices: &'static [&'static str],
    },
    ExpectedOne {
        option: &'static str,
    },
    IgnoredExplicit {
        option: &'static str,
        value: String,
    },
    Ambiguous {
        arg: String,
        matches: Vec<&'static str>,
    },
    Unrecognized(Vec<String>),
}

impl ArgError {
    /// The message; `repr` is Python's repr() of a str (`pyfmt::repr_str`).
    pub fn message(&self, repr: &dyn Fn(&str) -> String) -> String {
        match self {
            ArgError::InvalidChoice { option, value, choices } => {
                format!(
                    "argument {option}: invalid choice: {} (choose from {})",
                    repr(value),
                    choices.join(", ")
                )
            }
            ArgError::ExpectedOne { option } => format!("argument {option}: expected one argument"),
            ArgError::IgnoredExplicit { option, value } => {
                format!("argument {option}: ignored explicit argument {}", repr(value))
            }
            ArgError::Ambiguous { arg, matches } => {
                format!("ambiguous option: {arg} could match {}", matches.join(", "))
            }
            ArgError::Unrecognized(args) => format!("unrecognized arguments: {}", args.join(" ")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Help,
    Version,
    Lang,
    Theme,
}

impl Action {
    /// argparse's _get_action_name: the option strings joined by '/'.
    fn name(self) -> &'static str {
        match self {
            Action::Help => "-h/--help",
            Action::Version => "--version",
            Action::Lang => "--lang",
            Action::Theme => "--theme",
        }
    }

    /// nargs=None (one argument) for --lang and --theme, nargs=0 for --help and --version.
    fn takes_value(self) -> bool {
        matches!(self, Action::Lang | Action::Theme)
    }
}

/// The parser's option strings in the order argparse registered them (its _option_string_actions dict).
const OPTION_STRINGS: [(&str, Action); 5] = [
    ("-h", Action::Help),
    ("--help", Action::Help),
    ("--version", Action::Version),
    ("--lang", Action::Lang),
    ("--theme", Action::Theme),
];

fn lookup(option: &str) -> Option<Action> {
    OPTION_STRINGS
        .iter()
        .find(|(name, _)| *name == option)
        .map(|(_, action)| *action)
}

/// One interpretation of an option-like argument: (action, option string, separator, explicit argument).
#[derive(Clone, Debug)]
struct OptionTuple {
    action: Option<Action>,
    option: String,
    sep: Option<&'static str>,
    explicit: Option<String>,
}

/// One argument as argparse classified it: 'A', '-' (the "--" that ends options) or 'O'.
enum Kind {
    Arg,
    DoubleDash,
    Option(Vec<OptionTuple>),
}

fn chars_len(text: &str) -> usize {
    text.chars().count()
}

/// The first `n` characters (code points), like `text[:n]`.
fn head(text: &str, n: usize) -> &str {
    match text.char_indices().nth(n) {
        Some((index, _)) => &text[..index],
        None => text,
    }
}

/// Everything after the first `n` characters, like `text[n:]`.
fn tail(text: &str, n: usize) -> &str {
    &text[head(text, n).len()..]
}

/// argparse's _negative_number_matcher, `^-\d+$|^-\d*\.\d+$` ($ also matches before a final newline).
fn looks_negative(arg: &str) -> bool {
    let arg = arg.strip_suffix('\n').unwrap_or(arg);
    let Some(rest) = arg.strip_prefix('-') else {
        return false;
    };
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    if digits(rest) {
        return true;
    }
    match rest.split_once('.') {
        Some((whole, fraction)) => (whole.is_empty() || digits(whole)) && digits(fraction),
        None => false,
    }
}

/// argparse's `str.partition('=')`: (before, Some("="), after), or (all, None, None) without '='.
fn partition(arg: &str) -> (&str, Option<&'static str>, Option<String>) {
    match arg.split_once('=') {
        Some((before, after)) => (before, Some("="), Some(after.to_string())),
        None => (arg, None, None),
    }
}

/// argparse's _get_option_tuples (allow_abbrev=True).
fn option_tuples(arg: &str) -> Vec<OptionTuple> {
    let mut result = Vec::new();
    let second = arg.chars().nth(1);
    if second == Some('-') {
        let (prefix, sep, explicit) = partition(arg);
        for (name, action) in OPTION_STRINGS {
            if name.starts_with(prefix) {
                result.push(OptionTuple {
                    action: Some(action),
                    option: name.to_string(),
                    sep,
                    explicit: explicit.clone(),
                });
            }
        }
    } else {
        let (prefix, sep, explicit) = partition(arg);
        let short = head(arg, 2);
        for (name, action) in OPTION_STRINGS {
            if name == short {
                let rest = tail(arg, 2).to_string();
                result.push(OptionTuple {
                    action: Some(action),
                    option: name.to_string(),
                    sep: Some(""),
                    explicit: Some(rest),
                });
            } else if name.starts_with(prefix) {
                result.push(OptionTuple {
                    action: Some(action),
                    option: name.to_string(),
                    sep,
                    explicit: explicit.clone(),
                });
            }
        }
    }
    result
}

/// argparse's _parse_optional: None for a positional ('A'), else the possible interpretations ('O').
fn parse_optional(arg: &str) -> Option<Vec<OptionTuple>> {
    if !arg.starts_with('-') {
        return None;
    }
    if let Some(action) = lookup(arg) {
        return Some(vec![OptionTuple {
            action: Some(action),
            option: arg.to_string(),
            sep: None,
            explicit: None,
        }]);
    }
    if chars_len(arg) == 1 {
        return None;
    }
    if let (prefix, Some(sep), explicit) = partition(arg) {
        if let Some(action) = lookup(prefix) {
            return Some(vec![OptionTuple {
                action: Some(action),
                option: prefix.to_string(),
                sep: Some(sep),
                explicit,
            }]);
        }
    }
    let tuples = option_tuples(arg);
    if !tuples.is_empty() {
        return Some(tuples);
    }
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

struct State {
    lang: Option<Language>,
    theme: Theme,
    extras: Vec<String>,
}

/// What a step of the parse leads to: go on at this index, or stop with this outcome.
enum Step {
    Next(usize),
    Done(Parsed),
}

/// argparse's take_action: --help and --version end the parse; --lang and --theme check their choice.
fn take_action(action: Action, value: Option<String>, state: &mut State) -> Option<Parsed> {
    match action {
        Action::Help => Some(Parsed::Help),
        Action::Version => Some(Parsed::Version),
        Action::Lang | Action::Theme => {
            let value = value.unwrap_or_default();
            let choices = if action == Action::Lang {
                LANG_CHOICES
            } else {
                THEME_CHOICES
            };
            if action == Action::Lang {
                match Language::from_code(&value) {
                    Some(language) => state.lang = Some(language),
                    None => return Some(invalid_choice(action, value, choices)),
                }
            } else {
                match Theme::from_code(&value) {
                    Some(theme) => state.theme = theme,
                    None => return Some(invalid_choice(action, value, choices)),
                }
            }
            None
        }
    }
}

fn invalid_choice(action: Action, value: String, choices: &'static [&'static str]) -> Parsed {
    Parsed::Error(ArgError::InvalidChoice {
        option: action.name(),
        value,
        choices,
    })
}

/// argparse's consume_optional for the option at `index`.
fn consume_optional(args: &[String], kinds: &[Kind], index: usize, tuples: &[OptionTuple], state: &mut State) -> Step {
    if tuples.len() > 1 {
        let matches = tuples
            .iter()
            .filter_map(|t| {
                OPTION_STRINGS
                    .iter()
                    .find(|(name, _)| *name == t.option)
                    .map(|(name, _)| *name)
            })
            .collect();
        return Step::Done(Parsed::Error(ArgError::Ambiguous {
            arg: args[index].clone(),
            matches,
        }));
    }
    let Some(first) = tuples.first() else {
        state.extras.push(args[index].clone());
        return Step::Next(index + 1);
    };
    let (mut action, mut option, mut sep, mut explicit) =
        (first.action, first.option.clone(), first.sep, first.explicit.clone());
    let mut actions: Vec<(Action, Option<String>)> = Vec::new();
    let stop;
    loop {
        let Some(current) = action else {
            state.extras.push(args[index].clone());
            return Step::Next(index + 1);
        };
        match explicit.take() {
            Some(value) => {
                let second_is_dash = option.chars().nth(1) == Some('-');
                if !current.takes_value() && !second_is_dash && !value.is_empty() {
                    // A single-dash flag with more characters after it: -hx is -h followed by -x.
                    if sep.is_some_and(|s| !s.is_empty()) || value.starts_with('-') {
                        let error = ArgError::IgnoredExplicit {
                            option: current.name(),
                            value,
                        };
                        return Step::Done(Parsed::Error(error));
                    }
                    actions.push((current, None));
                    let next = format!("-{}", head(&value, 1));
                    match lookup(&next) {
                        Some(found) => {
                            action = Some(found);
                            option = next;
                            let rest = tail(&value, 1);
                            if rest.is_empty() {
                                sep = None;
                                explicit = None;
                            } else if let Some(after) = rest.strip_prefix('=') {
                                sep = Some("=");
                                explicit = Some(after.to_string());
                            } else {
                                sep = Some("");
                                explicit = Some(rest.to_string());
                            }
                        }
                        None => {
                            state.extras.push(format!("-{value}"));
                            stop = index + 1;
                            break;
                        }
                    }
                } else if current.takes_value() {
                    actions.push((current, Some(value)));
                    stop = index + 1;
                    break;
                } else {
                    return Step::Done(Parsed::Error(ArgError::IgnoredExplicit {
                        option: current.name(),
                        value,
                    }));
                }
            }
            None => {
                let start = index + 1;
                if current.takes_value() {
                    if !matches!(kinds.get(start), Some(Kind::Arg)) {
                        return Step::Done(Parsed::Error(ArgError::ExpectedOne { option: current.name() }));
                    }
                    actions.push((current, Some(args[start].clone())));
                    stop = start + 1;
                } else {
                    actions.push((current, None));
                    stop = start;
                }
                break;
            }
        }
    }
    for (current, value) in actions {
        if let Some(outcome) = take_action(current, value, state) {
            return Step::Done(outcome);
        }
    }
    Step::Next(stop)
}

/// Parse the arguments (without the program name) like `parser.parse_args(argv)` in gui.py.
pub fn parse(args: &[String]) -> Parsed {
    let mut kinds = Vec::with_capacity(args.len());
    let mut after_double_dash = false;
    for arg in args {
        if after_double_dash {
            kinds.push(Kind::Arg);
        } else if arg == "--" {
            kinds.push(Kind::DoubleDash);
            after_double_dash = true;
        } else {
            kinds.push(match parse_optional(arg) {
                Some(tuples) => Kind::Option(tuples),
                None => Kind::Arg,
            });
        }
    }
    let mut state = State {
        lang: None,
        theme: Theme::Auto,
        extras: Vec::new(),
    };
    let mut index = 0;
    while index < args.len() {
        match &kinds[index] {
            Kind::Option(tuples) => match consume_optional(args, &kinds, index, tuples, &mut state) {
                Step::Next(next) => index = next,
                Step::Done(outcome) => return outcome,
            },
            Kind::Arg | Kind::DoubleDash => {
                // This parser has no positionals: anything not taken by an option is left over.
                state.extras.push(args[index].clone());
                index += 1;
            }
        }
    }
    if !state.extras.is_empty() {
        return Parsed::Error(ArgError::Unrecognized(state.extras));
    }
    Parsed::Run(Options {
        lang: state.lang,
        theme: state.theme,
    })
}

// ---- argparse's HelpFormatter and textwrap ------------------------------------------------------------------

/// The usage parts of the options (argparse's _format_actions_usage, split by its part regexp).
const USAGE_PARTS: [&str; 4] = ["[-h]", "[--version]", "[--lang {en,es}]", "[--theme {auto,light,dark}]"];
/// (invocation, help) of each option, as the "options:" section lists them.
const OPTIONS_HELP: [(&str, &str); 4] = [
    ("-h, --help", "show this help message and exit"),
    ("--version", "show program's version number and exit"),
    ("--lang {en,es}", "language (default: the Windows display language)"),
    (
        "--theme {auto,light,dark}",
        "light or dark (default auto: the Windows app mode)",
    ),
];

fn len(text: &str) -> i64 {
    text.chars().count() as i64
}

fn spaces(n: i64) -> String {
    " ".repeat(n.max(0) as usize)
}

/// Python's `\w` for one character (close enough outside ASCII; every text wrapped here is ASCII).
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// textwrap's `letter`, `[^\d\W]`: a word character that is not a digit.
fn is_letter(c: char) -> bool {
    is_word(c) && !c.is_ascii_digit()
}

/// textwrap's `word_punct`, `[\w!"'&.,?]`.
fn is_word_punct(c: char) -> bool {
    is_word(c) || "!\"'&.,?".contains(c)
}

/// The chunks textwrap's wordsep_re splits one word (no whitespace) into: after a hyphen between letters
/// ("vbios-", "loader-", "gui:"), and around em-dashes ("--" between words).
fn split_word(word: &[char]) -> Vec<String> {
    let n = word.len();
    let letter = |i: usize| i < n && is_letter(word[i]);
    let dashes = |from: usize| word[from.min(n)..].iter().take_while(|c| **c == '-').count();
    let em_dash_at = |i: usize| {
        let run = dashes(i);
        run >= 2 && i + run < n && is_word(word[i + run])
    };
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < n {
        if word[start] == '-' && start > 0 && is_word_punct(word[start - 1]) && em_dash_at(start) {
            let end = start + dashes(start);
            chunks.push(word[start..end].iter().collect());
            start = end;
            continue;
        }
        let mut end = start + 1;
        loop {
            let hyphen = end < n
                && word[end] == '-'
                && ((end >= 2 && letter(end - 2) && letter(end - 1))
                    || (end >= 3 && letter(end - 3) && word[end - 2] == '-' && letter(end - 1)))
                && letter(end + 1)
                && (letter(end + 2) || (end + 2 < n && word[end + 2] == '-' && letter(end + 3)));
            if hyphen {
                chunks.push(word[start..=end].iter().collect());
                start = end + 1;
                break;
            }
            if end == n || (is_word_punct(word[end - 1]) && em_dash_at(end)) {
                chunks.push(word[start..end].iter().collect());
                start = end;
                break;
            }
            end += 1;
        }
    }
    chunks
}

/// argparse's whitespace collapse (`\s+` to one space, then strip) followed by textwrap's chunk split.
fn chunks(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    for (i, word) in text.split_ascii_whitespace().enumerate() {
        if i > 0 {
            result.push(" ".to_string());
        }
        let chars: Vec<char> = word.chars().collect();
        result.extend(split_word(&chars));
    }
    result
}

/// textwrap.TextWrapper._wrap_chunks with break_long_words, break_on_hyphens and drop_whitespace.
fn wrap_chunks(mut chunks: Vec<String>, width: i64, initial: &str, subsequent: &str) -> Vec<String> {
    let blank = |chunk: &String| chunk.chars().all(char::is_whitespace);
    let mut lines: Vec<String> = Vec::new();
    chunks.reverse();
    while !chunks.is_empty() {
        let mut line: Vec<String> = Vec::new();
        let mut line_len = 0;
        let indent = if lines.is_empty() { initial } else { subsequent };
        let room = width - len(indent);
        if !lines.is_empty() && chunks.last().is_some_and(blank) {
            chunks.pop();
        }
        while let Some(chunk) = chunks.last() {
            if line_len + len(chunk) > room {
                break;
            }
            line_len += len(chunk);
            line.extend(chunks.pop());
        }
        if chunks.last().is_some_and(|chunk| len(chunk) > room) {
            // _handle_long_word: put as much of the chunk as fits, after its last hyphen if it has one.
            let space_left = if room < 1 { 1 } else { room - line_len };
            if let Some(chunk) = chunks.pop() {
                let chars: Vec<char> = chunk.chars().collect();
                let mut end = space_left.max(0) as usize;
                if chars.len() as i64 > space_left {
                    let limit = end.min(chars.len());
                    if let Some(hyphen) = chars[..limit].iter().rposition(|c| *c == '-') {
                        if hyphen > 0 && chars[..hyphen].iter().any(|c| *c != '-') {
                            end = hyphen + 1;
                        }
                    }
                }
                let end = end.min(chars.len());
                line.push(chars[..end].iter().collect());
                chunks.push(chars[end..].iter().collect());
            }
        }
        if line.last().is_some_and(blank) {
            line.pop();
        }
        if !line.is_empty() {
            lines.push(format!("{indent}{}", line.concat()));
        }
    }
    lines
}

/// textwrap.wrap(text, width) after argparse's whitespace collapse (HelpFormatter._split_lines).
fn wrap(text: &str, width: i64) -> Vec<String> {
    wrap_chunks(chunks(text), width, "", "")
}

/// HelpFormatter._fill_text: textwrap.fill with the same indent on every line.
fn fill(text: &str, width: i64, indent: &str) -> String {
    wrap_chunks(chunks(text), width, indent, indent).join("\n")
}

/// HelpFormatter._format_usage, "usage: ...\n\n", for this parser (no positionals).
fn format_usage(width: i64) -> String {
    let prefix = "usage: ";
    let mut usage = format!("{PROG} {}", USAGE_PARTS.join(" "));
    let text_width = width;
    if len(prefix) + len(&usage) > text_width {
        let get_lines = |parts: &[&str], indent: &str, prefix: Option<&str>| {
            let mut lines: Vec<String> = Vec::new();
            let mut line: Vec<&str> = Vec::new();
            let indent_length = len(indent);
            let mut line_len = prefix.map_or(indent_length - 1, |p| len(p) - 1);
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
                    *first = first[indent.len()..].to_string();
                }
            }
            lines
        };
        // `len(prefix) + len(prog) <= 0.75 * text_width`, in integers.
        let lines = if 4 * (len(prefix) + len(PROG)) <= 3 * text_width {
            let indent = spaces(len(prefix) + len(PROG) + 1);
            let mut parts = vec![PROG];
            parts.extend(USAGE_PARTS);
            get_lines(&parts, &indent, Some(prefix))
        } else {
            let indent = spaces(len(prefix));
            let mut lines = vec![PROG.to_string()];
            lines.extend(get_lines(&USAGE_PARTS, &indent, None));
            lines
        };
        usage = lines.join("\n");
    }
    format!("{prefix}{usage}\n\n")
}

/// HelpFormatter.format_help's ending: runs of three or more newlines become two, then strip and end with one.
fn finish(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut newlines = 0;
    for c in text.chars() {
        if c == '\n' {
            newlines += 1;
            if newlines <= 2 {
                out.push(c);
            }
        } else {
            newlines = 0;
            out.push(c);
        }
    }
    format!("{}\n", out.trim_matches('\n'))
}

/// `parser.format_usage()`: what argparse prints before an error.
pub fn usage_text(columns: i64) -> String {
    finish(&format_usage(columns - 2))
}

/// `parser.format_help()` for a terminal `columns` wide.
pub fn help_text(columns: i64) -> String {
    let width = columns - 2;
    let max_help_position = 24.min((width - 20).max(4));
    let mut text = format_usage(width);
    text.push_str(&fill(DESCRIPTION, width.max(11), ""));
    text.push_str("\n\n\noptions:\n");
    let indent = 2;
    let action_max_length = OPTIONS_HELP
        .iter()
        .map(|(invocation, _)| len(invocation) + indent)
        .max()
        .unwrap_or(0);
    let help_position = (action_max_length + 2).min(max_help_position);
    let help_width = (width - help_position).max(11);
    let action_width = help_position - indent - 2;
    for (invocation, help) in OPTIONS_HELP {
        let indent_first = if len(invocation) <= action_width {
            let pad = spaces(action_width - len(invocation));
            text.push_str(&format!("{}{invocation}{pad}  ", spaces(indent)));
            0
        } else {
            text.push_str(&format!("{}{invocation}\n", spaces(indent)));
            help_position
        };
        let lines = wrap(help, help_width);
        for (i, line) in lines.iter().enumerate() {
            let lead = if i == 0 { indent_first } else { help_position };
            text.push_str(&format!("{}{line}\n", spaces(lead)));
        }
    }
    text.push('\n');
    finish(&text)
}

/// What `--version` prints ("vbios-loader-gui 0.5.0"), through argparse's formatter.
pub fn version_text(columns: i64) -> String {
    let version = format!("{PROG} {}", crate::VERSION);
    finish(&format!("{}\n\n", fill(&version, (columns - 2).max(11), "")))
}

/// What argparse prints for an error: the usage, then "vbios-loader-gui: error: <message>".
pub fn error_text(columns: i64, message: &str) -> String {
    format!("{}{PROG}: error: {message}\n", usage_text(columns))
}

/// The terminal width from COLUMNS as shutil.get_terminal_size reads it: Python's int(), used when positive. A
/// width beyond i64 is i64::MAX: every text fits on one line either way.
pub fn columns_from_env(value: Option<&str>) -> Option<i64> {
    let number = crate::pyfmt::parse_int(value?, 10)?;
    (number > 0).then(|| i64::try_from(number).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    /// Python's repr() of a str, as `gui::main` passes it.
    use crate::pyfmt::repr_str as repr;

    /// (argv, outcome) recorded from gui.py's parser on Python 3.12: "run <lang|-> <theme>", "help", "version" or
    /// "error <message>".
    const CASES: &[(&[&str], &str)] = &[
        (&["--help"], "help"),
        (&["-h"], "help"),
        (&["--version"], "version"),
        (
            &["--lang", "fr"],
            "error argument --lang: invalid choice: 'fr' (choose from en, es)",
        ),
        (&["--lang"], "error argument --lang: expected one argument"),
        (
            &["--theme", "blue"],
            "error argument --theme: invalid choice: 'blue' (choose from auto, light, dark)",
        ),
        (&["--theme=dark", "x"], "error unrecognized arguments: x"),
        (&["foo"], "error unrecognized arguments: foo"),
        (&["--foo"], "error unrecognized arguments: --foo"),
        (&["--lang=es", "--theme", "dark"], "run es dark"),
        (&["--la", "es"], "run es auto"),
        (
            &["--l=fr"],
            "error argument --lang: invalid choice: 'fr' (choose from en, es)",
        ),
        (&["-x"], "error unrecognized arguments: -x"),
        (&["-"], "error unrecognized arguments: -"),
        (&["--"], "error unrecognized arguments: --"),
        (&["--", "x"], "error unrecognized arguments: -- x"),
        (&["-hx"], "help"),
        (&["-h=x"], "error argument -h/--help: ignored explicit argument 'x'"),
        (&["--help=x"], "error argument -h/--help: ignored explicit argument 'x'"),
        (&["--version", "--lang", "fr"], "version"),
        (
            &["--lang", "fr", "--version"],
            "error argument --lang: invalid choice: 'fr' (choose from en, es)",
        ),
        (&["x", "--help"], "help"),
        (&["--lang", "-h"], "error argument --lang: expected one argument"),
        (
            &["--lang", "-1"],
            "error argument --lang: invalid choice: '-1' (choose from en, es)",
        ),
        (&["--lang", "--theme"], "error argument --lang: expected one argument"),
        (
            &["--theme", ""],
            "error argument --theme: invalid choice: '' (choose from auto, light, dark)",
        ),
        (
            &["--lang="],
            "error argument --lang: invalid choice: '' (choose from en, es)",
        ),
        (&["--lang", "en", "--lang", "es"], "run es auto"),
        (&["-hh"], "help"),
        (&["--v"], "version"),
        (&["--ver"], "version"),
        (&["--th", "light"], "run - light"),
        (&["--t=light"], "run - light"),
        (&["a", "b"], "error unrecognized arguments: a b"),
        (&["--x=1"], "error unrecognized arguments: --x=1"),
        (&["--lang", "es", "extra"], "error unrecognized arguments: extra"),
        (&["-l"], "error unrecognized arguments: -l"),
        (
            &["--lang", "español"],
            "error argument --lang: invalid choice: 'español' (choose from en, es)",
        ),
        (&["--LANG", "es"], "error unrecognized arguments: --LANG es"),
        (&["---lang"], "error unrecognized arguments: ---lang"),
        (
            &["--lang", " es"],
            "error argument --lang: invalid choice: ' es' (choose from en, es)",
        ),
        (&["--lang es"], "error unrecognized arguments: --lang es"),
        (
            &["--version="],
            "error argument --version: ignored explicit argument ''",
        ),
        (&["-h-"], "error argument -h/--help: ignored explicit argument '-'"),
        (&["--lang", "--"], "error argument --lang: expected one argument"),
        (
            &["--lang", "es", "--", "--theme", "dark"],
            "error unrecognized arguments: -- --theme dark",
        ),
        (&["-1"], "error unrecognized arguments: -1"),
        (&["-a b"], "error unrecognized arguments: -a b"),
        (
            &["--lang", "it's"],
            "error argument --lang: invalid choice: \"it's\" (choose from en, es)",
        ),
        (
            &["--lang", "a\"b'c"],
            "error argument --lang: invalid choice: 'a\"b\\'c' (choose from en, es)",
        ),
        (
            &["--lang", "tab\there"],
            "error argument --lang: invalid choice: 'tab\\there' (choose from en, es)",
        ),
        (
            &["--theme", "DARK"],
            "error argument --theme: invalid choice: 'DARK' (choose from auto, light, dark)",
        ),
        (&[], "run - auto"),
        (&["--theme", "auto"], "run - auto"),
        (&["--theme", "light", "--lang", "en"], "run en light"),
        (&["-v"], "error unrecognized arguments: -v"),
        (&["--he"], "help"),
        (&["--h=1"], "error argument -h/--help: ignored explicit argument '1'"),
        (&["--hel", "x"], "help"),
        (
            &["x", "--lang", "zz"],
            "error argument --lang: invalid choice: 'zz' (choose from en, es)",
        ),
        (
            &["--lang", "zz", "x"],
            "error argument --lang: invalid choice: 'zz' (choose from en, es)",
        ),
        (
            &["--=x"],
            "error ambiguous option: --=x could match --help, --version, --lang, --theme",
        ),
        (
            &["-=x"],
            "error ambiguous option: -=x could match -h, --help, --version, --lang, --theme",
        ),
        (
            &["--lang=a=b"],
            "error argument --lang: invalid choice: 'a=b' (choose from en, es)",
        ),
        (
            &["--theme", "dark", "--theme"],
            "error argument --theme: expected one argument",
        ),
        (
            &["--lang", "-"],
            "error argument --lang: invalid choice: '-' (choose from en, es)",
        ),
        (&["-h", "--lang", "fr"], "help"),
        (&["--version", "--help"], "version"),
        (&["--help", "--version"], "help"),
        (
            &["--lang", "-1.5"],
            "error argument --lang: invalid choice: '-1.5' (choose from en, es)",
        ),
        (
            &["--lang", "-.5"],
            "error argument --lang: invalid choice: '-.5' (choose from en, es)",
        ),
        (&["-1.5"], "error unrecognized arguments: -1.5"),
        (&["--lang", "es", "-x", "y"], "error unrecognized arguments: -x y"),
        (&["--Lang"], "error unrecognized arguments: --Lang"),
        (&["-H"], "error unrecognized arguments: -H"),
        (&[""], "error unrecognized arguments: "),
        (&["-hx=y"], "help"),
    ];

    fn outcome(parsed: &Parsed) -> String {
        match parsed {
            Parsed::Run(options) => format!(
                "run {} {}",
                options.lang.map_or("-", |language| language.code()),
                options.theme.code()
            ),
            Parsed::Help => "help".to_string(),
            Parsed::Version => "version".to_string(),
            Parsed::Error(error) => format!("error {}", error.message(&repr)),
        }
    }

    #[test]
    fn arguments_are_read_like_argparse() {
        for (args, expected) in CASES {
            assert_eq!(outcome(&parse(&argv(args))), *expected, "{args:?}");
        }
    }

    #[test]
    fn options_map_to_the_controller_types() {
        let parsed = parse(&argv(&["--lang", "es", "--theme", "dark"]));
        assert_eq!(
            parsed,
            Parsed::Run(Options {
                lang: Some(Language::Es),
                theme: Theme::Dark
            })
        );
    }

    const HELP_80: &str = "\
usage: vbios-loader-gui [-h] [--version] [--lang {en,es}]
                        [--theme {auto,light,dark}]

vbios-loader-gui: a window to pick a GPU and a VBIOS file, then load, remove
or verify the override.

options:
  -h, --help            show this help message and exit
  --version             show program's version number and exit
  --lang {en,es}        language (default: the Windows display language)
  --theme {auto,light,dark}
                        light or dark (default auto: the Windows app mode)
";

    const HELP_120: &str = "\
usage: vbios-loader-gui [-h] [--version] [--lang {en,es}] [--theme {auto,light,dark}]

vbios-loader-gui: a window to pick a GPU and a VBIOS file, then load, remove or verify the override.

options:
  -h, --help            show this help message and exit
  --version             show program's version number and exit
  --lang {en,es}        language (default: the Windows display language)
  --theme {auto,light,dark}
                        light or dark (default auto: the Windows app mode)
";

    const HELP_60: &str = "\
usage: vbios-loader-gui [-h] [--version] [--lang {en,es}]
                        [--theme {auto,light,dark}]

vbios-loader-gui: a window to pick a GPU and a VBIOS file,
then load, remove or verify the override.

options:
  -h, --help            show this help message and exit
  --version             show program's version number and
                        exit
  --lang {en,es}        language (default: the Windows
                        display language)
  --theme {auto,light,dark}
                        light or dark (default auto: the
                        Windows app mode)
";

    const HELP_40: &str = "\
usage: vbios-loader-gui [-h]
                        [--version]
                        [--lang {en,es}]
                        [--theme {auto,light,dark}]

vbios-loader-gui: a window to pick a
GPU and a VBIOS file, then load,
remove or verify the override.

options:
  -h, --help      show this help
                  message and exit
  --version       show program's
                  version number and
                  exit
  --lang {en,es}  language (default:
                  the Windows display
                  language)
  --theme {auto,light,dark}
                  light or dark
                  (default auto: the
                  Windows app mode)
";

    const HELP_30: &str = "\
usage: vbios-loader-gui
       [-h] [--version]
       [--lang {en,es}]
       [--theme {auto,light,dark}]

vbios-loader-gui: a window
to pick a GPU and a VBIOS
file, then load, remove or
verify the override.

options:
  -h, --help
        show this help
        message and exit
  --version
        show program's
        version number and
        exit
  --lang {en,es}
        language (default:
        the Windows display
        language)
  --theme {auto,light,dark}
        light or dark
        (default auto: the
        Windows app mode)
";

    const HELP_10: &str = "\
usage: vbios-loader-gui
       [-h]
       [--version]
       [--lang {en,es}]
       [--theme {auto,light,dark}]

vbios-
loader-gui:
a window to
pick a GPU
and a VBIOS
file, then
load,
remove or
verify the
override.

options:
  -h, --help
    show this
    help
    message and
    exit
  --version
    show
    program's
    version
    number and
    exit
  --lang {en,es}
    language
    (default:
    the Windows
    display
    language)
  --theme {auto,light,dark}
    light or
    dark
    (default
    auto: the
    Windows app
    mode)
";

    #[test]
    fn help_is_argparse_help_at_each_width() {
        for (columns, expected) in [
            (80, HELP_80),
            (120, HELP_120),
            (200, HELP_120),
            (60, HELP_60),
            (40, HELP_40),
            (30, HELP_30),
            (10, HELP_10),
        ] {
            assert_eq!(help_text(columns), expected, "COLUMNS={columns}");
        }
    }

    #[test]
    fn version_and_errors_print_like_argparse() {
        assert_eq!(version_text(80), format!("vbios-loader-gui {}\n", crate::VERSION));
        assert_eq!(
            error_text(80, "argument --lang: expected one argument"),
            "usage: vbios-loader-gui [-h] [--version] [--lang {en,es}]\n                        \
             [--theme {auto,light,dark}]\nvbios-loader-gui: error: argument --lang: expected one argument\n"
        );
        assert_eq!(
            usage_text(120),
            "usage: vbios-loader-gui [-h] [--version] [--lang {en,es}] [--theme {auto,light,dark}]\n"
        );
    }

    #[test]
    fn textwrap_breaks_after_hyphens_between_letters() {
        let chars: Vec<char> = "vbios-loader-gui:".chars().collect();
        assert_eq!(split_word(&chars), ["vbios-", "loader-", "gui:"]);
        let chars: Vec<char> = "a--b".chars().collect();
        assert_eq!(split_word(&chars), ["a", "--", "b"]);
        let chars: Vec<char> = "0.5.0".chars().collect();
        assert_eq!(split_word(&chars), ["0.5.0"]);
        let chars: Vec<char> = "x-y-z".chars().collect();
        assert_eq!(split_word(&chars), ["x-y-z"]);
        assert_eq!(wrap("aaaaaaaaaaaa-bbbb cc", 11), ["aaaaaaaaaaa", "a-bbbb cc"]);
    }

    #[test]
    fn columns_follow_python_int_and_positive_values() {
        assert_eq!(columns_from_env(Some("120")), Some(120));
        assert_eq!(columns_from_env(Some(" +1_00 ")), Some(100));
        assert_eq!(columns_from_env(Some("0")), None);
        assert_eq!(columns_from_env(Some("-5")), None);
        assert_eq!(columns_from_env(Some("1__0")), None);
        assert_eq!(columns_from_env(Some("wide")), None);
        assert_eq!(columns_from_env(None), None);
        // Python's int() also reads other decimal digits and spaces, refuses \x1c and more than 4300 digits.
        assert_eq!(columns_from_env(Some("\u{661}\u{662}\u{660}")), Some(120));
        assert_eq!(columns_from_env(Some("\u{3000}80")), Some(80));
        assert_eq!(columns_from_env(Some("80\x1c")), None);
        assert_eq!(columns_from_env(Some(&"9".repeat(4301))), None);
        let huge = format!("1{}", "0".repeat(30));
        assert_eq!(columns_from_env(Some(&huge)), Some(i64::MAX));
        assert_eq!(help_text(i64::MAX), help_text(1000));
        assert_eq!(error_text(i64::MAX, "x"), error_text(1000, "x"));
    }
}
