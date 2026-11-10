//! The few Python behaviours the reports depend on: repr() of a str and of a list of str (with Python's
//! str.isprintable rules) and int(text, base), plus the str predicates devices.py relies on (isspace, strip,
//! isdecimal, isdigit). The Unicode tables come from CPython 3.12, the Python of loader 0.4.1 (`pyfmt_table.rs`).

use std::fmt::Write as _;

#[path = "pyfmt_table.rs"]
mod table;

/// Python repr(s): quotes, escapes and the printable test exactly as CPython does.
pub fn repr_str(s: &str) -> String {
    // Single quotes unless the text has a single quote and no double quote.
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for c in s.chars() {
        let cp = c as u32;
        if c == quote || c == '\\' {
            out.push('\\');
            out.push(c);
        } else if c == '\t' {
            out.push_str("\\t");
        } else if c == '\n' {
            out.push_str("\\n");
        } else if c == '\r' {
            out.push_str("\\r");
        } else if cp < 0x20 || cp == 0x7F {
            let _ = write!(out, "\\x{cp:02x}");
        } else if cp < 0x7F || is_printable(c) {
            out.push(c);
        } else if cp <= 0xFF {
            let _ = write!(out, "\\x{cp:02x}");
        } else if cp <= 0xFFFF {
            let _ = write!(out, "\\u{cp:04x}");
        } else {
            let _ = write!(out, "\\U{cp:08x}");
        }
    }
    out.push(quote);
    out
}

/// Python repr(list_of_str): "['a', 'b']", "[]".
pub fn repr_list(items: &[String]) -> String {
    let parts: Vec<String> = items.iter().map(|item| repr_str(item)).collect();
    format!("[{}]", parts.join(", "))
}

/// Python int(text, base) for base 10 and 16: surrounding whitespace, a sign, a 0x prefix (base 16), single
/// underscores between digits. None when Python raises ValueError. Values beyond i128 saturate.
///
/// Like CPython, non-ASCII whitespace counts as a space and non-ASCII decimal digits (U+0660 and so on) as
/// their ASCII digit, also in the prefix; a base-10 text of more than 4300 digits is refused (Python's
/// int_max_str_digits). Other bases from 2 to 36 follow the same rules (0o/0b prefixes); any other base gives None.
pub fn parse_int(text: &str, base: u32) -> Option<i128> {
    if !(2..=36).contains(&base) {
        return None;
    }
    // _PyUnicode_TransformDecimalAndSpaceToASCII: ASCII stays (NUL and DEL included); any other character
    // that is neither whitespace nor a decimal digit makes the text invalid.
    let mut ascii = Vec::with_capacity(text.len());
    for c in text.chars() {
        let cp = c as u32;
        if cp < 127 {
            ascii.push(cp as u8);
        } else if is_space(c) {
            ascii.push(b' ');
        } else {
            ascii.push(b'0' + decimal_value(c)? as u8);
        }
    }
    // PyLong_FromString with the C locale's isspace (no \x1c-\x1f, which stay as they are above).
    let space = |b: u8| matches!(b, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r');
    let mut i = 0;
    while i < ascii.len() && space(ascii[i]) {
        i += 1;
    }
    let mut negative = false;
    if i < ascii.len() && (ascii[i] == b'+' || ascii[i] == b'-') {
        negative = ascii[i] == b'-';
        i += 1;
    }
    if i + 1 < ascii.len() && ascii[i] == b'0' {
        let marker = ascii[i + 1].to_ascii_lowercase();
        if (base == 16 && marker == b'x') || (base == 8 && marker == b'o') || (base == 2 && marker == b'b') {
            i += 2;
            // One underscore is allowed right after the prefix.
            if i < ascii.len() && ascii[i] == b'_' {
                i += 1;
            }
        }
    }
    if i < ascii.len() && ascii[i] == b'_' {
        return None;
    }
    let start = i;
    let mut previous_underscore = false;
    let mut digits = 0usize;
    let mut value: i128 = 0;
    let mut saturated = false;
    while i < ascii.len() {
        let byte = ascii[i];
        if byte == b'_' {
            if previous_underscore {
                return None;
            }
            previous_underscore = true;
        } else {
            let digit = match byte {
                b'0'..=b'9' => u32::from(byte - b'0'),
                b'a'..=b'z' => u32::from(byte - b'a') + 10,
                b'A'..=b'Z' => u32::from(byte - b'A') + 10,
                _ => break,
            };
            if digit >= base {
                break;
            }
            previous_underscore = false;
            digits += 1;
            match value
                .checked_mul(i128::from(base))
                .and_then(|v| v.checked_add(i128::from(digit)))
            {
                Some(next) => value = next,
                None => saturated = true,
            }
        }
        i += 1;
    }
    if previous_underscore || i == start {
        return None;
    }
    while i < ascii.len() && space(ascii[i]) {
        i += 1;
    }
    if i != ascii.len() {
        return None;
    }
    if !base.is_power_of_two() && digits > 4300 {
        return None;
    }
    Some(match (saturated, negative) {
        (true, false) => i128::MAX,
        (true, true) => i128::MIN,
        (false, false) => value,
        (false, true) => -value,
    })
}

/// Python str.isprintable() of one character: false for the categories C* and Z* except the ASCII space.
pub fn is_printable(c: char) -> bool {
    let cp = c as u32;
    if cp < 0x80 {
        return (0x20..0x7F).contains(&cp);
    }
    !in_ranges(&table::NONPRINTABLE, cp)
}

/// Python str.isspace() of one character (what str.strip() and str.split() remove), including \x1c-\x1f.
pub fn is_space(c: char) -> bool {
    table::SPACES.binary_search(&(c as u32)).is_ok()
}

/// Python str.strip() without arguments.
pub fn strip(s: &str) -> &str {
    s.trim_matches(is_space)
}

/// The value of a Unicode decimal digit (str.isdecimal(): what int() and the `\d` of a str regex accept), else None.
pub fn decimal_value(c: char) -> Option<u32> {
    let cp = c as u32;
    let index = match table::DECIMAL_ZEROS.binary_search(&cp) {
        Ok(index) => index,
        Err(0) => return None,
        Err(index) => index - 1,
    };
    let offset = cp - table::DECIMAL_ZEROS[index];
    (offset < 10).then_some(offset)
}

/// Python str.isdigit() of one character: the decimal digits plus superscripts, circled digits and the like
/// (which int() refuses).
pub fn is_digit(c: char) -> bool {
    decimal_value(c).is_some() || in_ranges(&table::DIGITS_NOT_DECIMAL, c as u32)
}

/// True when `cp` is in one of the sorted, inclusive ranges.
fn in_ranges(ranges: &[(u32, u32)], cp: u32) -> bool {
    let index = ranges.partition_point(|&(first, _)| first <= cp);
    index > 0 && cp <= ranges[index - 1].1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repr_quotes_and_escapes() {
        // Expected values from CPython 3.12's repr().
        assert_eq!(repr_str(""), "''");
        assert_eq!(repr_str("abc"), "'abc'");
        assert_eq!(repr_str("a'b"), "\"a'b\"");
        assert_eq!(repr_str("a\"b"), "'a\"b'");
        assert_eq!(repr_str("a'\"b"), "'a\\'\"b'");
        assert_eq!(repr_str("back\\slash"), "'back\\\\slash'");
        assert_eq!(repr_str("\t\n\r\x00\x1b\x7f"), "'\\t\\n\\r\\x00\\x1b\\x7f'");
        assert_eq!(repr_str("1\x007"), "'1\\x007'");
        assert_eq!(
            repr_str("\u{7f}\u{80}\u{a0}\u{ad}\u{200b}\u{1f600}\u{e0001}\u{d7ff}é€"),
            "'\\x7f\\x80\\xa0\\xad\\u200b😀\\U000e0001\\ud7ffé€'"
        );
        assert_eq!(
            repr_str("\u{fffd}\u{feff}\u{3000}\u{2028}"),
            "'\u{fffd}\\ufeff\\u3000\\u2028'"
        );
        assert_eq!(repr_list(&[]), "[]");
        assert_eq!(
            repr_list(&["a".to_string(), "".to_string(), "it's".to_string()]),
            "['a', '', \"it's\"]"
        );
    }

    #[test]
    fn printable_table() {
        assert!(is_printable(' ') && is_printable('~') && is_printable('é') && is_printable('\u{fffd}'));
        assert!(!is_printable('\u{7f}') && !is_printable('\u{85}') && !is_printable('\u{a0}'));
        assert!(!is_printable('\u{10ffff}') && !is_printable('\u{e0000}') && is_printable('\u{1f600}'));
    }

    #[test]
    fn int_like_python() {
        // (text, int(text, 10), int(text, 16)) as CPython 3.12 gives them; None is ValueError.
        let cases: &[(&str, Option<i128>, Option<i128>)] = &[
            ("0", Some(0), Some(0)),
            ("00", Some(0), Some(0)),
            ("0_0", Some(0), Some(0)),
            ("-0", Some(0), Some(0)),
            ("1_2", Some(12), Some(0x12)),
            ("-1", Some(-1), Some(-1)),
            (" 0x12 ", None, Some(0x12)),
            ("+0x12", None, Some(0x12)),
            ("-0x12", None, Some(-0x12)),
            ("0X12", None, Some(0x12)),
            ("0x_1", None, Some(1)),
            ("0x_ff_ff", None, Some(0xffff)),
            ("0x__1", None, None),
            ("0x_", None, None),
            ("0x", None, None),
            ("_1", None, None),
            ("1_", None, None),
            ("1__2", None, None),
            ("+_1", None, None),
            ("-_1", None, None),
            ("+ 12", None, None),
            ("0_x1", None, None),
            ("0b1", None, Some(0xb1)),
            ("ff", None, Some(255)),
            ("fF", None, Some(255)),
            ("0x100000000", None, Some(0x1_0000_0000)),
            ("12 34", None, None),
            ("", None, None),
            ("   ", None, None),
            ("\x0b12\x0c", Some(12), Some(0x12)),
            ("\u{a0}12\u{3000}", Some(12), Some(0x12)),
            ("\u{85}12", Some(12), Some(0x12)),
            (" \x1c12", None, None),
            ("\x1c12", None, None),
            ("12\x1c", None, None),
            ("\x0012", None, None),
            ("12\x00", None, None),
            ("1\x7f", None, None),
            ("١٢", Some(12), Some(0x12)),
            ("０x12", None, Some(0x12)),
            ("0x１２", None, Some(0x12)),
            ("0x٣", None, Some(3)),
            ("٠x12", None, Some(0x12)),
            ("²", None, None),
            ("①", None, None),
            ("é", None, None),
        ];
        for &(text, ten, sixteen) in cases {
            assert_eq!(parse_int(text, 10), ten, "int({text:?})");
            assert_eq!(parse_int(text, 16), sixteen, "int({text:?}, 16)");
        }
    }

    #[test]
    fn int_limits() {
        // CPython refuses more than 4300 decimal digits (leading zeros and all), not hex digits.
        assert_eq!(parse_int(&"0".repeat(4300), 10), Some(0));
        assert_eq!(parse_int(&"0".repeat(4301), 10), None);
        assert_eq!(parse_int(&format!("{}1", "1_".repeat(4300)), 10), None);
        assert_eq!(parse_int(&"1".repeat(4300), 10), Some(i128::MAX));
        assert_eq!(parse_int(&format!("-{}", "1".repeat(50)), 10), Some(i128::MIN));
        assert_eq!(parse_int(&"f".repeat(5000), 16), Some(i128::MAX));
        assert_eq!(parse_int(&"f".repeat(5000), 15), None);
        assert_eq!(
            parse_int("170141183460469231731687303715884105727", 10),
            Some(i128::MAX)
        );
        assert_eq!(
            parse_int("-170141183460469231731687303715884105727", 10),
            Some(-i128::MAX)
        );
        assert_eq!(parse_int("12", 1), None);
        assert_eq!(parse_int("12", 37), None);
        assert_eq!(parse_int("0o17", 8), Some(15));
        assert_eq!(parse_int("0b_101", 2), Some(5));
    }

    #[test]
    fn never_panics_on_random_text() {
        use crate::testing::Rng;
        let mut rng = Rng::new(0xF00D);
        for _ in 0..20000 {
            let size = rng.below(30);
            let text = rng.text(size);
            let _ = repr_str(&text);
            let _ = repr_list(&[text.clone(), rng.text(3)]);
            let _ = strip(&text);
            for base in [0, 1, 2, 8, 10, 16, 36, 37] {
                let _ = parse_int(&text, base);
            }
            let digits: String = (0..rng.below(60)).map(|_| *rng.pick(&['9', 'f', '_', '0'])).collect();
            let _ = parse_int(&format!("-0x{digits}"), 16);
            let _ = parse_int(&digits, 10);
        }
    }

    #[test]
    fn str_predicates() {
        assert_eq!(strip("\x1c\u{3000} 01:00.0\t\u{85}"), "01:00.0");
        assert_eq!(strip("\u{200b}x"), "\u{200b}x");
        assert!(is_space('\x1f') && is_space('\u{2029}') && !is_space('\u{180e}') && !is_space('\u{200b}'));
        assert_eq!(decimal_value('7'), Some(7));
        assert_eq!(decimal_value('\u{0669}'), Some(9));
        assert_eq!(decimal_value('\u{1d7ce}'), Some(0));
        assert_eq!(decimal_value('\u{1d7ff}'), Some(9));
        assert_eq!(decimal_value('/'), None);
        assert_eq!(decimal_value(':'), None);
        assert_eq!(decimal_value('\u{b2}'), None);
        assert!(is_digit('\u{b2}') && is_digit('\u{2460}') && is_digit('5') && !is_digit('a') && !is_digit('½'));
    }
}
