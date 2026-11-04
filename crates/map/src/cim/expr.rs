//! ArcGIS Pro label expressions as RustGIS label templates (see `symbology::template_parts`).
//!
//! One tokenizer reads all four of Pro's expression engines (Arcade, VBScript, Python, JScript):
//! field references (`$feature.X`, `$feature["X"]`, `$feature['X']`, `[X]`, `!X!`), string
//! literals, concatenation (`+`, `&`) and the engines' newlines. An expression made only of those
//! is carried over exactly; any other (a function call, arithmetic, a comparison) is drawn with
//! the first field it refers to.
//!
//! The input is hostile: the expression is walked once, front to back, with no recursion, and
//! each name or literal is scanned only as far as its end.

use crate::symbology::MAX_TEMPLATE;

/// A label expression as a label class's `field`.
pub(crate) struct Translated {
    /// A plain field name, or a template of `{FIELD}` placeholders (`{{` and `}}` for braces).
    pub template: String,
    /// Whether `template` draws what the expression does; else it is the first field referred to.
    pub exact: bool,
    /// The fields `template` refers to, in the order they first appear, each once.
    pub fields: Vec<String>,
    /// Whether Pro text formatting tags (`<BOL>`, `<CLR …>`…) were dropped from the text.
    pub tags_dropped: bool,
    /// Each field of a join, as written (`T.Name`) and as read (`Name`), each once.
    pub joins: Vec<(String, String)>,
}

/// `expression` in `engine` ("Arcade", "VBScript", "Python", "JScript") as a label template.
/// Exact when the whole expression is field references, string literals, concatenation (`+`,
/// `&`) and newlines; else the first field referenced (`exact: false`), or `None` when none.
pub(crate) fn template(expression: &str, engine: &str) -> Option<Translated> {
    let e = body(expression);
    // A bare name (a join's `T.Name` too) when the whole expression is one.
    if name_in(e, true).is_some_and(|(_, rest)| rest.is_empty()) && newline(e).is_none() {
        return Translated::field(e, true);
    }
    let vbscript = engine.trim().eq_ignore_ascii_case("VBScript");
    let Some((pieces, tags_dropped)) = scan(e, vbscript) else {
        return Translated::field(first_reference(e)?, false);
    };
    let first = pieces.iter().find_map(|p| match p {
        Piece::Field(written) => Some(*written),
        Piece::Text(_) => None,
    })?;
    // One field alone is a plain field name.
    if let [Piece::Field(written)] = pieces.as_slice() {
        return Translated::field(written, true);
    }
    // A placeholder only carries a plain name (a quoted one can hold anything).
    if pieces.iter().any(|p| matches!(p, Piece::Field(written) if !written.chars().all(|c| is_name_char(c) || c == '.'))) {
        return Translated::field(first, false);
    }
    let (mut fields, mut joins, mut text) = (Vec::<String>::new(), Vec::<(String, String)>::new(), String::new());
    // The template's length so far, not counting `\r` (a `\r\n` or a lone `\r` ends up one
    // character), so never more than its final length: building stops once it is too long.
    let mut length = 0;
    for piece in &pieces {
        if length > MAX_TEMPLATE {
            return Translated::field(first, false);
        }
        match piece {
            Piece::Text(t) => {
                for c in t.chars() {
                    match c {
                        '{' => text.push_str("{{"),
                        '}' => text.push_str("}}"),
                        c => text.push(c),
                    }
                    length += match c {
                        '{' | '}' => 2,
                        '\r' => 0,
                        _ => 1,
                    };
                }
            }
            Piece::Field(written) => {
                let name = read_as(written);
                length += name.chars().count() + 2;
                text.push('{');
                text.push_str(name);
                text.push('}');
                if !fields.iter().any(|f| f == name) {
                    fields.push(name.to_string());
                }
                if name != *written && !joins.iter().any(|(w, _)| w == written) {
                    joins.push((written.to_string(), name.to_string()));
                }
            }
        }
    }
    let template = text.replace("\r\n", "\n").replace('\r', "\n");
    if template.chars().count() > MAX_TEMPLATE {
        return Translated::field(first, false);
    }
    Some(Translated { template, exact: true, fields, tags_dropped, joins })
}

/// The field an expression stands for when it is only one field reference.
pub(crate) fn field(expression: &str) -> Option<String> {
    template(expression, "Arcade").filter(|t| t.exact && t.fields.first() == Some(&t.template)).map(|t| t.template)
}

impl Translated {
    /// The field `written` (read by the part after its last dot) on its own; `None` when that is
    /// longer than a template can be, as no field's name is.
    fn field(written: &str, exact: bool) -> Option<Translated> {
        let name = read_as(written);
        if name.chars().count() > MAX_TEMPLATE {
            return None;
        }
        let joins = if written == name { Vec::new() } else { vec![(written.to_string(), name.to_string())] };
        Some(Translated { template: name.to_string(), exact, fields: vec![name.to_string()], tags_dropped: false, joins })
    }
}

/// A piece of an expression: text (newlines as `\n`, `\r` or both), or a field as written.
enum Piece<'a> {
    Text(String),
    Field(&'a str),
}

/// `expression` without the whitespace around it, a leading `return` and a trailing `;`.
fn body(expression: &str) -> &str {
    let e = expression.trim();
    let e = match (e.get(..6), e.get(6..)) {
        (Some(word), Some(rest)) if word.eq_ignore_ascii_case("return") && !rest.starts_with(is_name_char) => rest.trim_start(),
        _ => e,
    };
    e.strip_suffix(';').unwrap_or(e).trim_end()
}

/// `e` as its pieces, and whether formatting tags were dropped from its text; `None` when it is
/// anything but operands joined by `+` or `&`.
fn scan(e: &str, vbscript: bool) -> Option<(Vec<Piece<'_>>, bool)> {
    let (mut pieces, mut tags_dropped, mut rest, mut want_operand) = (Vec::new(), false, e, true);
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        if want_operand {
            let (piece, after) = operand(rest, vbscript, &mut tags_dropped)?;
            pieces.push(piece);
            rest = after;
        } else {
            rest = rest.strip_prefix(['+', '&'])?;
        }
        want_operand = !want_operand;
    }
    // (Empty, or ending in an operator.)
    if want_operand {
        return None;
    }
    Some((pieces, tags_dropped))
}

/// The operand at the start of `s` and what follows it.
fn operand<'a>(s: &'a str, vbscript: bool, tags_dropped: &mut bool) -> Option<(Piece<'a>, &'a str)> {
    if let Some(after) = s.strip_prefix("$feature") {
        let (name, rest) = feature_field(after)?;
        return Some((Piece::Field(name), rest));
    }
    let mut chars = s.chars();
    let first = chars.next()?;
    let after = chars.as_str();
    match first {
        '[' | '!' => {
            let (name, rest) = name_in(after, true)?;
            Some((Piece::Field(name), rest.strip_prefix(if first == '[' { ']' } else { '!' })?))
        }
        '"' | '\'' => {
            let (text, rest) = literal(after, first, vbscript)?;
            Some((Piece::Text(markup(&text, tags_dropped)), rest))
        }
        _ => {
            let end = s.find(|c: char| !is_name_char(c) && c != '.').unwrap_or(s.len());
            let (word, rest) = (s.get(..end)?, s.get(end..)?);
            if let Some(text) = newline(word) {
                Some((Piece::Text(text.to_string()), rest))
            } else if word.eq_ignore_ascii_case("chr") {
                let (code, rest) = rest.strip_prefix('(')?.split_at_checked(2)?;
                let rest = rest.strip_prefix(')')?;
                let text = match code {
                    "10" => "\n",
                    "13" => "\r",
                    _ => return None,
                };
                Some((Piece::Text(text.to_string()), rest))
            } else {
                None
            }
        }
    }
}

/// The text of the newline `word` (`TextFormatting.NewLine`, `vbNewLine`, `vbCrLf`, `vbLf`).
fn newline(word: &str) -> Option<&'static str> {
    if ["TextFormatting.NewLine", "vbNewLine", "vbCrLf"].iter().any(|w| word.eq_ignore_ascii_case(w)) {
        Some("\r\n")
    } else if word.eq_ignore_ascii_case("vbLf") {
        Some("\n")
    } else {
        None
    }
}

/// The field of `$feature` followed by `after` (`.X`, `["X"]` or `['X']`), and what follows it.
/// A quoted name is anything up to its closing quote.
fn feature_field(after: &str) -> Option<(&str, &str)> {
    if let Some(r) = after.strip_prefix('.') {
        return name_in(r, true);
    }
    let r = after.strip_prefix('[')?;
    let mut chars = r.chars();
    let quote = chars.next().filter(|q| *q == '"' || *q == '\'')?;
    let (name, rest) = chars.as_str().split_once(quote)?;
    Some((name, rest.strip_prefix(']')?)).filter(|(name, _)| !name.is_empty())
}

/// The field name at the start of `s` (with dots when `dots`, not ending in one), and what
/// follows it.
fn name_in(s: &str, dots: bool) -> Option<(&str, &str)> {
    let end = s.find(|c: char| !(is_name_char(c) || (dots && c == '.'))).unwrap_or(s.len());
    let (name, rest) = (s.get(..end)?, s.get(end..)?);
    (!name.is_empty() && !name.ends_with('.')).then_some((name, rest))
}

/// The string literal after its opening `quote` (its text, and what follows its closing quote);
/// `None` when it isn't closed. VBScript doubles a quote; the other engines escape with `\`.
fn literal(s: &str, quote: char, vbscript: bool) -> Option<(String, &str)> {
    let mut text = String::new();
    let mut chars = s.char_indices();
    while let Some((i, c)) = chars.next() {
        if c == quote {
            let rest = s.get(i + c.len_utf8()..)?;
            if vbscript && rest.starts_with(quote) {
                text.push(quote);
                chars.next();
                continue;
            }
            return Some((text, rest));
        }
        if c == '\\' && !vbscript {
            match chars.next()?.1 {
                'n' => text.push('\n'),
                'r' => text.push('\r'),
                e @ ('"' | '\'' | '\\') => text.push(e),
                other => {
                    text.push('\\');
                    text.push(other);
                }
            }
        } else {
            text.push(c);
        }
    }
    None
}

/// The Pro text formatting tags that are dropped from a label's text.
const FORMAT_TAGS: [&str; 5] = ["BOL", "ITA", "UND", "FNT", "CLR"];

/// A literal's text with `<br>` as a newline and Pro's formatting tags dropped (`tags_dropped`
/// set when one is). Any other `<` is text.
fn markup(text: &str, tags_dropped: &mut bool) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some((before, after)) = rest.split_once('<') {
        out.push_str(before);
        let tag = after.find(['<', '>']).filter(|end| after.get(*end..).is_some_and(|t| t.starts_with('>')));
        if let Some((tag, tail)) = tag.and_then(|end| Some((after.get(..end)?, after.get(end + 1..)?))) {
            if tag.eq_ignore_ascii_case("br") {
                out.push('\n');
                rest = tail;
                continue;
            }
            let bare = tag.strip_prefix('/').unwrap_or(tag);
            let name = bare.split_once(char::is_whitespace).map_or(bare, |(n, _)| n);
            if FORMAT_TAGS.iter().any(|t| name.eq_ignore_ascii_case(t)) {
                *tags_dropped = true;
                rest = tail;
                continue;
            }
        }
        out.push('<');
        rest = after;
    }
    out.push_str(rest);
    out
}

/// The part of a field name after its last dot: a join's field is read by its own name.
fn read_as(written: &str) -> &str {
    written.rsplit_once('.').map_or(written, |(_, name)| name)
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The first field reference in `e` (`$feature.X`, `$feature["X"]`, `[X]`, `!X!`), anywhere.
fn first_reference(e: &str) -> Option<&str> {
    let mut rest = e;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("$feature")
            && let Some((name, _)) = feature_field(after)
        {
            return Some(name);
        }
        let mut chars = rest.chars();
        let first = chars.next()?;
        rest = chars.as_str();
        if first == '[' || first == '!' {
            let close = if first == '[' { ']' } else { '!' };
            if let Some((name, after)) = name_in(rest, true)
                && after.starts_with(close)
            {
                return Some(name);
            }
        }
    }
    None
}
