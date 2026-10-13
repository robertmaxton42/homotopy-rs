//! Labels with inline LaTeX, such as the names of generators: text with formulas between dollar
//! signs, like `f $\otimes$ g`.
//!
//! Each label is parsed on its own, so that what one label contains can never affect the text
//! around it. The rules:
//!
//! - A formula is the text between two `$`s. It cannot be empty, and a `\` in it escapes the
//!   next character, so `$\$$` is a formula that is a dollar sign.
//! - A `$` that does not start a formula (it has no `$` to end it, or the two are adjacent) is
//!   an ordinary dollar sign, and so is `\$` outside formulas. So `costs $5` and `a $$ b` are
//!   plain text, and a label that forgets its last `$` shows a dollar sign instead of turning
//!   the rest of the label, or whatever is put after it, into a formula.
//!
//! To put labels together into one, [`seal`] each of them first.

/// A piece of a label.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Piece<'a> {
    /// Text, as written (with its escapes, but without the dollar signs, which are [`Dollar`]s).
    ///
    /// [`Dollar`]: Piece::Dollar
    Text(&'a str),
    /// A dollar sign that does not delimit a formula.
    Dollar,
    /// The source of a formula, without its dollar signs.
    Math(&'a str),
}

/// The pieces that make up a label.
#[must_use]
pub fn pieces(label: &str) -> Vec<Piece<'_>> {
    let bytes = label.as_bytes();
    let mut pieces = vec![];
    let mut text_start = 0;
    let mut i = 0;

    while i < bytes.len() {
        let skip = match bytes[i] {
            b'\\' if bytes.get(i + 1) == Some(&b'$') => {
                if text_start < i {
                    pieces.push(Piece::Text(&label[text_start..i]));
                }
                pieces.push(Piece::Dollar);
                text_start = i + 2;
                2
            }
            // An escaped backslash, so that it cannot escape a dollar sign.
            b'\\' if bytes.get(i + 1) == Some(&b'\\') => 2,
            b'$' => {
                if text_start < i {
                    pieces.push(Piece::Text(&label[text_start..i]));
                }
                match closing_dollar(bytes, i + 1) {
                    Some(end) if end > i + 1 => {
                        pieces.push(Piece::Math(&label[i + 1..end]));
                        text_start = end + 1;
                        i = end;
                    }
                    // An empty formula is two dollar signs.
                    Some(end) => {
                        pieces.push(Piece::Dollar);
                        pieces.push(Piece::Dollar);
                        text_start = end + 1;
                        i = end;
                    }
                    None => {
                        pieces.push(Piece::Dollar);
                        text_start = i + 1;
                    }
                }
                1
            }
            _ => 1,
        };
        i += skip;
    }
    if text_start < bytes.len() {
        pieces.push(Piece::Text(&label[text_start..]));
    }
    pieces
}

/// The index of the `$` that ends a formula, given where its source starts.
const fn closing_dollar(bytes: &[u8], start: usize) -> Option<usize> {
    let mut j = start;
    while j < bytes.len() {
        match bytes[j] {
            b'\\' => j += 2,
            b'$' => return Some(j),
            _ => j += 1,
        }
    }
    None
}

/// A label that is shown the same as `label`, but is safe to put in a longer label.
///
/// Its dollar signs that are not part of a formula are escaped, so it cannot interact with the
/// text around it. Sealing a label that was sealed already changes nothing.
///
/// A label that ends in a backslash would escape a dollar sign put after it, so it is followed by
/// a zero-width space.
#[must_use]
pub fn seal(label: &str) -> String {
    let mut sealed = String::with_capacity(label.len());
    for piece in pieces(label) {
        match piece {
            Piece::Text(text) => {
                sealed.push_str(text);
                let backslashes = text.bytes().rev().take_while(|&b| b == b'\\').count();
                if backslashes % 2 == 1 {
                    sealed.push('\u{200b}');
                }
            }
            Piece::Dollar => sealed.push_str("\\$"),
            Piece::Math(math) => {
                sealed.push('$');
                sealed.push_str(math);
                sealed.push('$');
            }
        }
    }
    sealed
}

/// The formula that shows `label` inside another formula, with its text in `\text{...}` and its
/// formulas as they are: `f $\alpha$` becomes `\text{f }{\alpha}`.
#[must_use]
pub fn to_math(label: &str) -> String {
    let mut math = String::with_capacity(label.len());
    for piece in pieces(label) {
        match piece {
            Piece::Text(text) => {
                math.push_str("\\text{");
                for c in text.chars() {
                    match c {
                        '\\' => math.push_str("\\textbackslash{}"),
                        '{' | '}' | '$' | '%' | '#' | '&' | '_' => {
                            math.push('\\');
                            math.push(c);
                        }
                        '^' => math.push_str("\\textasciicircum{}"),
                        '~' => math.push_str("\\textasciitilde{}"),
                        c => math.push(c),
                    }
                }
                math.push('}');
            }
            Piece::Dollar => math.push_str("\\text{\\$}"),
            Piece::Math(formula) => {
                math.push('{');
                math.push_str(formula);
                math.push('}');
            }
        }
    }
    math
}

#[cfg(test)]
mod tests {
    use super::{
        pieces, seal, to_math,
        Piece::{Dollar, Math, Text},
    };

    #[test]
    fn formulas_are_delimited_by_pairs_of_dollars() {
        assert_eq!(pieces(""), vec![]);
        assert_eq!(pieces("f"), vec![Text("f")]);
        assert_eq!(pieces("$x$"), vec![Math("x")]);
        assert_eq!(
            pieces("a $x$ b $\\alpha_1$"),
            vec![Text("a "), Math("x"), Text(" b "), Math("\\alpha_1")]
        );
        assert_eq!(
            pieces("ünï $ç$ → $∂$"),
            vec![Text("ünï "), Math("ç"), Text(" → "), Math("∂")]
        );
    }

    #[test]
    fn an_unmatched_dollar_is_a_dollar_sign() {
        assert_eq!(pieces("$"), vec![Dollar]);
        assert_eq!(pieces("a $x"), vec![Text("a "), Dollar, Text("x")]);
        assert_eq!(pieces("$x"), vec![Dollar, Text("x")]);
        // The first pair is a formula, and the last dollar is on its own.
        assert_eq!(
            pieces("$x$ then $y"),
            vec![Math("x"), Text(" then "), Dollar, Text("y")]
        );
        assert_eq!(
            pieces("a$b$c$d"),
            vec![Text("a"), Math("b"), Text("c"), Dollar, Text("d")]
        );
    }

    #[test]
    fn empty_formulas_are_dollar_signs() {
        assert_eq!(pieces("$$"), vec![Dollar, Dollar]);
        assert_eq!(
            pieces("a $$ b"),
            vec![Text("a "), Dollar, Dollar, Text(" b")]
        );
        // The pair is not reused as the start of a formula.
        assert_eq!(pieces("$$x$"), vec![Dollar, Dollar, Text("x"), Dollar]);
    }

    #[test]
    fn dollars_can_be_escaped() {
        assert_eq!(pieces("\\$5"), vec![Dollar, Text("5")]);
        assert_eq!(
            pieces("\\$5 and $x$"),
            vec![Dollar, Text("5 and "), Math("x")]
        );
        // In a formula, the escape belongs to the formula, which goes on.
        assert_eq!(pieces("$a\\$b$"), vec![Math("a\\$b")]);
        assert_eq!(pieces("$\\$$"), vec![Math("\\$")]);
        // An escaped backslash does not escape what follows it.
        assert_eq!(pieces("$a\\\\$ b"), vec![Math("a\\\\"), Text(" b")]);
        assert_eq!(pieces("a\\\\$x$"), vec![Text("a\\\\"), Math("x")]);
        // A lone backslash at the end is just a backslash.
        assert_eq!(pieces("a\\"), vec![Text("a\\")]);
        assert_eq!(pieces("$a\\"), vec![Dollar, Text("a\\")]);
    }

    #[test]
    fn sealed_labels_cannot_affect_what_follows() {
        for label in ["$", "a $x", "$x$ then $y", "a$b$c$d", "$$", "\\$5", "$a\\"] {
            let sealed = seal(label);
            // Whatever follows, the sealed label is parsed as it was alone.
            let alone = pieces(&sealed);
            let followed = format!("{sealed}$z$ and $w");
            let mut expected = alone.clone();
            expected.extend([Math("z"), Text(" and "), Dollar, Text("w")]);
            assert_eq!(pieces(&followed), expected, "{label}");
        }
    }

    #[test]
    fn sealing_keeps_what_is_shown_and_is_idempotent() {
        for label in ["f", "$x$", "a $x", "a$b$c$d", "$$", "\\$5 $x$", "$a\\$b$"] {
            let sealed = seal(label);
            assert_eq!(seal(&sealed), sealed, "{label}");
            let shown = |label: &str| {
                pieces(label)
                    .into_iter()
                    .map(|piece| match piece {
                        Text(text) => text.to_owned(),
                        Dollar => "$".to_owned(),
                        Math(math) => format!("${math}$"),
                    })
                    .collect::<String>()
            };
            assert_eq!(shown(&sealed), shown(label), "{label}");
        }
        assert_eq!(seal("a $x"), "a \\$x");
        assert_eq!(seal("$x$"), "$x$");
    }

    #[test]
    fn labels_can_be_put_in_formulas() {
        assert_eq!(to_math(""), "");
        assert_eq!(to_math("f"), "\\text{f}");
        assert_eq!(to_math("$\\alpha$ 1"), "{\\alpha}\\text{ 1}");
        assert_eq!(to_math("a $x"), "\\text{a }\\text{\\$}\\text{x}");
        assert_eq!(to_math("100% {a_b}"), "\\text{100\\% \\{a\\_b\\}}");
        // Whatever the label, the braces balance.
        let math = to_math("}$x$\\{");
        assert_eq!(math, "\\text{\\}}{x}\\text{\\textbackslash{}\\{}");
    }
}
