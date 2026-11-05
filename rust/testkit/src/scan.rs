//! Reading Rust source as text, for the guard tests (IP-03 decision D03-11, IP-04 decision D04-11).
//!
//! A guard that looks for a name must not be fooled by a comment that mentions it, and sometimes must not be fooled by a string either. [`code_of`] blanks the
//! comments (and, if asked, the contents of strings and character literals) and keeps every line break, so that a line number of the result is a line number of
//! the file. It is a scanner and not a parser: it needs no dependency, it knows line and nested block comments, strings, raw strings (`r#"…"#`), character
//! literals and lifetimes, and that is all the guards ask of it.

/// The number of `#` of a raw string that starts at `c[i]` (`r"` is 0, `r#"` is 1), or `None` if `c[i]` does not start one.
fn raw_string_hashes(c: &[char], i: usize) -> Option<usize> {
    let hashes = c[i + 1..].iter().take_while(|&&ch| ch == '#').count();
    (c.get(i + 1 + hashes) == Some(&'"')).then_some(hashes)
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The text of a Rust source with every comment blanked out. With `keep_strings` the contents of strings, raw strings and character literals are kept (a guard
/// that looks for a name in a string needs them); without it they are blanked too, and only their quotes remain.
pub fn code_of(text: &str, keep_strings: bool) -> String {
    let c: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    // a character of a string or a literal: kept, or blanked (a line break is always kept)
    let literal = |ch: char| {
        if keep_strings || ch == '\n' {
            ch
        } else {
            ' '
        }
    };
    while i < c.len() {
        let next = c.get(i + 1).copied();
        if c[i] == '/' && next == Some('/') {
            while i < c.len() && c[i] != '\n' {
                out.push(' ');
                i += 1;
            }
        } else if c[i] == '/' && next == Some('*') {
            let mut depth = 0;
            while i < c.len() {
                if c[i] == '/' && c.get(i + 1) == Some(&'*') {
                    depth += 1;
                    out.push_str("  ");
                    i += 2;
                } else if c[i] == '*' && c.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    out.push_str("  ");
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    out.push(if c[i] == '\n' { '\n' } else { ' ' });
                    i += 1;
                }
            }
        } else if c[i] == 'r'
            && (i == 0 || !is_ident(c[i - 1]))
            && raw_string_hashes(&c, i).is_some()
        {
            let hashes = raw_string_hashes(&c, i).unwrap_or(0);
            let start = i + 1 + hashes + 1; // after `r`, the hashes and the opening quote
            out.extend(&c[i..start]);
            i = start;
            while i < c.len() {
                let closes = c[i] == '"' && (0..hashes).all(|k| c.get(i + 1 + k) == Some(&'#'));
                if closes {
                    out.push('"');
                    out.extend(std::iter::repeat_n('#', hashes));
                    i += 1 + hashes;
                    break;
                }
                out.push(literal(c[i]));
                i += 1;
            }
        } else if c[i] == '"' {
            out.push('"');
            i += 1;
            while i < c.len() {
                if c[i] == '\\' && i + 1 < c.len() {
                    out.push(literal(c[i]));
                    out.push(literal(c[i + 1]));
                    i += 2;
                    continue;
                }
                if c[i] == '"' {
                    out.push('"');
                    i += 1;
                    break;
                }
                out.push(literal(c[i]));
                i += 1;
            }
        } else if c[i] == '\'' {
            // a character literal ('x' or '\n'), or a lifetime ('a), which has no closing quote
            let len = if next == Some('\\') {
                c[i + 1..].iter().position(|&ch| ch == '\'').map(|p| p + 2)
            } else if c.get(i + 2) == Some(&'\'') {
                Some(3)
            } else {
                None
            };
            match len {
                Some(len) if i + len <= c.len() => {
                    out.push('\'');
                    out.extend(c[i + 1..i + len - 1].iter().map(|&ch| literal(ch)));
                    out.push('\'');
                    i += len;
                }
                _ => {
                    out.push('\'');
                    i += 1;
                }
            }
        } else {
            out.push(c[i]);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_are_blanked_and_line_numbers_are_kept() {
        let source = "let a = 1; // Layer_1\n/* Layer_2\n   /* nested Layer_3 */ still a comment */\nlet b = 2;\n";
        let code = code_of(source, true);
        assert!(!code.contains("Layer_"), "{code}");
        assert_eq!(code.lines().count(), source.lines().count());
        assert_eq!(code.lines().nth(3), Some("let b = 2;"));
    }

    #[test]
    fn strings_are_kept_or_blanked_as_asked() {
        let source = "let a = \"Monitoring_Data\"; let b = r#\"Calibration_Data\"#; let c = '\\n'; let d = 'x';";
        let kept = code_of(source, true);
        assert!(
            kept.contains("Monitoring_Data") && kept.contains("Calibration_Data"),
            "{kept}"
        );
        let blanked = code_of(source, false);
        assert!(
            !blanked.contains("Monitoring_Data") && !blanked.contains("Calibration_Data"),
            "{blanked}"
        );
        // the quotes stay, so that the shape of the code does
        assert!(
            blanked.contains("\"                \"") && blanked.contains("let d = ' ';"),
            "{blanked}"
        );
    }

    #[test]
    fn a_quote_in_a_character_literal_and_a_lifetime_do_not_confuse_it() {
        let source = "fn f<'a>(x: &'a str) -> char { let q = '\"'; let s = \"after\"; q }";
        let kept = code_of(source, true);
        assert!(
            kept.contains("\"after\"") && kept.contains("&'a str"),
            "{kept}"
        );
        let blanked = code_of(source, false);
        assert!(
            !blanked.contains("after") && blanked.contains("&'a str"),
            "{blanked}"
        );
    }
}
