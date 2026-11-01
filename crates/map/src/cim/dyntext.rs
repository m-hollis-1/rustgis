//! Pro's text markup as RustGIS text. A Pro text string holds `<dyn type="…" property="…" …/>`
//! tags (dynamic text, which RustGIS writes as `{token|prefix|suffix}`), inline formatting tags
//! (`<BOL>`, `<CLR …>`…, which a one-style RustGIS text element can't hold) and XML entities.
//!
//! The text is hostile: it is cut at [`MAX_TEXT`] characters, one loop walks it with every step
//! going forward, a tag is read only as far as the next `<` or `>` (and at most [`MAX_TAG`]
//! characters), and the notes about repeated problems are bounded.

use super::Report;
use super::layout::shown;

/// The most characters of a text that are read.
const MAX_TEXT: usize = 100_000;
/// The longest tag (between `<` and `>`) that is read as one.
const MAX_TAG: usize = 2_000;
/// The longest prefix or suffix kept: RustGIS reads a token of about 200 characters.
const MAX_AFFIX: usize = 60;
/// The most attributes read from one tag.
const MAX_ATTRS: usize = 32;
/// The most different unsupported tags that are each named in a note.
const MAX_NAMED: usize = 32;
/// Pro's inline formatting tags (also read with a `/` in front, as the closer).
const FORMATTING: [&str; 10] = ["BOL", "ITA", "UND", "CLR", "FNT", "SUP", "SUB", "ACP", "LCS", "UCS"];
const ENTITIES: [(&str, char); 5] = [("&amp;", '&'), ("&lt;", '<'), ("&gt;", '>'), ("&quot;", '"'), ("&apos;", '\'')];

/// `text` as RustGIS text, with the name of the map frame its tokens read (the `name` of the
/// first `mapFrame` tag). What is dropped or approximated is said in `report` under `what`.
pub(crate) fn translate(text: &str, report: &mut Report, what: &str) -> (String, Option<String>) {
    let mut st = State { report, what, formatting: false, empty: false, affix: false, other_frame: false, named: Vec::new(), frame: None };
    let text = match text.char_indices().nth(MAX_TEXT) {
        Some((i, _)) => {
            st.report.approximated(what, format!("text past {MAX_TEXT} characters, cut"));
            text.get(..i).unwrap_or_default()
        }
        None => text,
    };
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(i) = rest.find('<') {
        out.push_str(&unescape(rest.get(..i).unwrap_or_default()));
        let tail = rest.get(i + 1..).unwrap_or_default();
        let body = tail.find(['<', '>']).filter(|&j| j <= MAX_TAG && tail.as_bytes().get(j) == Some(&b'>')).and_then(|j| tail.get(..j));
        match body.filter(|b| st.tag(b, &mut out)) {
            Some(b) => rest = tail.get(b.len() + 1..).unwrap_or_default(),
            None => {
                out.push('<');
                rest = tail;
            }
        }
    }
    out.push_str(&unescape(rest));
    (out, st.frame)
}

/// What one text's translation has said and found so far.
struct State<'a> {
    report: &'a mut Report,
    what: &'a str,
    /// Each of these notes is made once per text.
    formatting: bool,
    empty: bool,
    affix: bool,
    other_frame: bool,
    /// The unsupported tags already named.
    named: Vec<String>,
    frame: Option<String>,
}

impl State<'_> {
    /// Handles the tag whose text between `<` and `>` is `body`: writes what stands for it to
    /// `out`. False when it isn't one of ours, so it stays text.
    fn tag(&mut self, body: &str, out: &mut String) -> bool {
        let body = body.trim();
        let closing = body.starts_with('/');
        let body = body.trim_start_matches('/');
        let end = body.find(|c: char| c.is_whitespace() || c == '/').unwrap_or(body.len());
        let (name, attrs) = (body.get(..end).unwrap_or_default(), body.get(end..).unwrap_or_default());
        if name.eq_ignore_ascii_case("dyn") {
            if !closing {
                self.dynamic(&attributes(attrs), out);
            }
        } else if FORMATTING.iter().any(|f| name.eq_ignore_ascii_case(f)) {
            if !std::mem::replace(&mut self.formatting, true) {
                self.report.approximated(self.what, "formatting tags, left out");
            }
        } else {
            return false;
        }
        true
    }

    /// A `<dyn …/>` tag as a token, or nothing (and a note) when it is one RustGIS has no token for.
    fn dynamic(&mut self, attrs: &[(String, String)], out: &mut String) {
        let get = |k: &str| attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        let (ty, prop) = (get("type").unwrap_or_default(), get("property").unwrap_or_default());
        let token = match (ty, prop) {
            ("mapFrame", "scale") => Some("scale"),
            ("mapFrame", "center") => Some("center"),
            ("mapFrame", "extent") => Some("extent"),
            ("mapFrame", "sr") => match get("srProperty") {
                Some("name") => Some("crsName"),
                Some("projection") => Some("crsProjection"),
                Some("units") => Some("crsUnits"),
                Some("wkid") => Some("crsWkid"),
                other => return self.unknown(&format!("{ty}/{prop}/{}", other.unwrap_or_default())),
            },
            ("project", "name") => Some("project"),
            // Whatever layout the tag names (its stale `name` is ignored), it reads this one's.
            ("layout", "serviceLayerCredits") => Some("credits"),
            ("layout", "name") => Some("layout"),
            ("date", _) => Some("date"),
            ("time", _) => Some("time"),
            ("page", "number" | "pageNumber") => Some("page"),
            ("page", "name" | "pageName") => Some("pageName"),
            ("page", "count" | "pageCount") => Some("pages"),
            _ => None,
        };
        let Some(token) = token else { return self.unknown(&format!("{ty}/{prop}")) };
        if ty == "mapFrame" {
            self.read_frame(get("name"));
        }
        if let (true, Some(f)) = (token == "date", get("format")) {
            self.report.approximated(self.what, format!("date format {}, drawn as RustGIS's", shown(f)));
        }
        if let Some(e) = get("emptyStr").filter(|e| !e.is_empty())
            && !std::mem::replace(&mut self.empty, true)
        {
            self.report.approximated(self.what, format!("empty text '{}' not kept", shown(e)));
        }
        let mut pre = self.affix_of(get("preStr").unwrap_or_default());
        // Pro's scale is the bare number and the template supplies "1:"; RustGIS's `{scale}` has it.
        if token == "scale"
            && let Some(p) = pre.trim_end().strip_suffix("1:").filter(|p| !p.ends_with(|c: char| c.is_ascii_digit()))
        {
            pre = p.to_string();
        }
        // `newLine` starts the text on a line of its own: the break comes before it, inside the
        // prefix, so an empty value leaves no blank line ("Spatial Reference<dyn … preStr="Name: "
        // newLine="true"/>" is a heading with "Name: …" under it).
        if get("newLine").is_some_and(|v| v.eq_ignore_ascii_case("true")) {
            pre.insert_str(0, "\\n");
        }
        let post = self.affix_of(get("postStr").unwrap_or_default());
        out.push('{');
        out.push_str(token);
        if !pre.is_empty() || !post.is_empty() {
            out.push('|');
            out.push_str(&pre);
        }
        if !post.is_empty() {
            out.push('|');
            out.push_str(&post);
        }
        out.push('}');
    }

    /// Notes (once per different tag, up to [`MAX_NAMED`]) a tag that is left out.
    fn unknown(&mut self, label: &str) {
        if self.named.len() < MAX_NAMED && !self.named.iter().any(|n| n == label) {
            self.report.skipped(self.what, format!("dynamic text {}, left out", shown(label)));
            self.named.push(label.to_string());
        }
    }

    /// Records the frame a `mapFrame` tag names: the first is the text's, a different later one
    /// is noted (once).
    fn read_frame(&mut self, name: Option<&str>) {
        let Some(name) = name.map(str::trim).filter(|n| !n.is_empty()) else { return };
        match &self.frame {
            None => self.frame = Some(name.to_string()),
            Some(first) if first != name && !std::mem::replace(&mut self.other_frame, true) => {
                self.report.approximated(self.what, format!("tokens also read frame {}; {} is used", shown(name), shown(first)));
            }
            Some(_) => {}
        }
    }

    /// A prefix or suffix as the token syntax holds it: a line break written `\n`, no `|` or `}`
    /// (which end a part or the token), and at most [`MAX_AFFIX`] characters.
    fn affix_of(&mut self, s: &str) -> String {
        let mut out = String::new();
        for (n, c) in s.chars().enumerate() {
            if n >= MAX_AFFIX {
                self.note_affix(format!("a prefix or suffix cut to {MAX_AFFIX} characters"));
                break;
            }
            match c {
                '\n' => out.push_str("\\n"),
                '|' | '}' => self.note_affix("a '|' or '}' in a prefix or suffix, left out".into()),
                c => out.push(c),
            }
        }
        out
    }

    fn note_affix(&mut self, text: String) {
        if !std::mem::replace(&mut self.affix, true) {
            self.report.approximated(self.what, text);
        }
    }
}

/// The attributes of a tag (`key="value"`, either quote), values unescaped. Reading stops at
/// anything that isn't one.
fn attributes(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = s;
    while out.len() < MAX_ATTRS {
        rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '/');
        let Some(eq) = rest.find('=') else { break };
        let key = rest.get(..eq).unwrap_or_default().rsplit(char::is_whitespace).next().unwrap_or_default();
        let after = rest.get(eq + 1..).unwrap_or_default().trim_start();
        let Some(quote) = after.chars().next().filter(|c| matches!(c, '"' | '\'')) else { break };
        let inner = after.get(1..).unwrap_or_default();
        let Some(end) = inner.find(quote) else { break };
        out.push((key.to_string(), unescape(inner.get(..end).unwrap_or_default())));
        rest = inner.get(end + 1..).unwrap_or_default();
    }
    out
}

/// `s` with `&amp;`, `&lt;`, `&gt;`, `&quot;` and `&apos;` as the characters they stand for, in one pass.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(rest.get(..i).unwrap_or_default());
        let tail = rest.get(i..).unwrap_or_default();
        match ENTITIES.iter().find(|(e, _)| tail.starts_with(e)) {
            Some((e, c)) => {
                out.push(*c);
                rest = tail.get(e.len()..).unwrap_or_default();
            }
            None => {
                out.push('&');
                rest = tail.get(1..).unwrap_or_default();
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str) -> (String, Option<String>, Vec<String>) {
        let mut report = Report::default();
        let (t, f) = translate(text, &mut report, "T");
        (t, f, report.notes.iter().map(|n| format!("{}: {}", n.layer, n.text)).collect())
    }

    #[test]
    fn every_tag_of_the_table_becomes_its_token() {
        let tags = [
            (r#"<dyn type="mapFrame" name="M" property="center"/>"#, "{center}"),
            (r#"<dyn type="mapFrame" name="M" property="extent"/>"#, "{extent}"),
            (r#"<dyn type="mapFrame" name="M" property="sr" srProperty="projection"/>"#, "{crsProjection}"),
            (r#"<dyn type="mapFrame" name="M" property="sr" srProperty="units"/>"#, "{crsUnits}"),
            (r#"<dyn type="mapFrame" name="M" property="sr" srProperty="wkid"/>"#, "{crsWkid}"),
            (r#"<dyn type="project" property="name"/>"#, "{project}"),
            (r#"<dyn type="layout" name="8x11 OLD" property="serviceLayerCredits"/>"#, "{credits}"),
            (r#"<dyn type="layout" property="name"/>"#, "{layout}"),
            (r#"<dyn type="time"/>"#, "{time}"),
            (r#"<dyn type="page" property="number"/>"#, "{page}"),
            (r#"<dyn type="page" property="name"/>"#, "{pageName}"),
            (r#"<dyn type="page" property="count"/>"#, "{pages}"),
        ];
        for (tag, token) in tags {
            let (t, _, notes) = run(tag);
            assert_eq!(t, token, "{tag}");
            assert!(notes.is_empty(), "{tag}: {notes:?}");
        }
    }

    #[test]
    fn attributes_in_any_order_either_quote_and_a_suffix_alone() {
        let (t, f, n) = run(r#"<dyn property='sr' postStr=' (end)' srProperty='units' type='mapFrame' name='A'/>"#);
        assert_eq!(t, "{crsUnits|| (end)}");
        assert_eq!(f.as_deref(), Some("A"));
        assert!(n.is_empty(), "{n:?}");
    }

    #[test]
    fn the_scale_loses_the_one_colon_the_token_already_prints() {
        let scale = |pre: &str| run(&format!(r#"<dyn type="mapFrame" name="M" property="scale" preStr="{pre}"/>"#)).0;
        assert_eq!(scale("1:"), "{scale}");
        assert_eq!(scale("Scale 1:"), "{scale|Scale }");
        assert_eq!(scale("Scale 1: "), "{scale|Scale }");
        assert_eq!(scale("Scale: "), "{scale|Scale: }");
        // A "1:" that ends a longer number isn't the ratio's.
        assert_eq!(scale("Plan 11:"), "{scale|Plan 11:}");
        assert_eq!(scale("21: "), "{scale|21: }");
        // Another token keeps a "1:".
        assert_eq!(run(r#"<dyn type="page" property="number" preStr="1:"/>"#).0, "{page|1:}");
    }

    #[test]
    fn a_second_frame_a_date_format_and_an_empty_text_are_noted() {
        let (t, f, n) = run(concat!(
            r#"<dyn type="mapFrame" name="A" property="scale"/><dyn type="mapFrame" name="B" property="extent" emptyStr="n/a"/>"#,
            r#"<dyn type="mapFrame" name="C" property="center" emptyStr="none"/><dyn type="date" format="long"/>"#
        ));
        assert_eq!(t, "{scale}{extent}{center}{date}");
        assert_eq!(f.as_deref(), Some("A"));
        assert_eq!(
            n,
            ["T: tokens also read frame B; A is used", "T: empty text 'n/a' not kept", "T: date format long, drawn as RustGIS's"]
        );
    }

    #[test]
    fn unsupported_tags_formatting_and_entities() {
        let (t, f, n) = run(
            r#"<BOL>A &amp; B</BOL> &lt;x&gt; <CLR red="1"/>"q"<fnt size='9'>z</FNT><dyn type="user" property="name"/><dyn type="user" property="name"/><dyn type="mapFrame" property="sr" srProperty="x"/> 5 < 6 <b>"#,
        );
        assert_eq!(t, r#"A & B <x> "q"z 5 < 6 <b>"#);
        assert_eq!(f, None);
        assert_eq!(n, ["T: formatting tags, left out", "T: dynamic text user/name, left out", "T: dynamic text mapFrame/sr/x, left out"]);
        // An entity is unescaped once, and not read as a tag.
        assert_eq!(run("&lt;dyn type=&quot;project&quot;/&gt; &amp;amp;").0, "<dyn type=\"project\"/> &amp;");
    }

    #[test]
    fn prefix_and_suffix_keep_line_breaks_and_lose_what_the_token_syntax_cant_hold() {
        let (t, _, n) = run("<dyn type=\"project\" property=\"name\" preStr=\"a|b}c\nd\" postStr=\"\" newLine=\"true\"/>");
        assert_eq!(t, "{project|\\nabc\\nd}");
        // A line break with no prefix is the whole prefix; one with a suffix keeps it.
        let (t, _, _) = run("<dyn type=\"project\" property=\"name\" postStr=\" ft\" newLine=\"true\"/>");
        assert_eq!(t, "{project|\\n| ft}");
        assert_eq!(n, ["T: a '|' or '}' in a prefix or suffix, left out"]);
        let long = "x".repeat(200);
        let (t, _, n) = run(&format!("<dyn type=\"project\" property=\"name\" preStr=\"{long}\"/>"));
        assert_eq!(t, format!("{{project|{}}}", "x".repeat(MAX_AFFIX)));
        assert_eq!(n.len(), 1);
    }

    #[test]
    fn hostile_text_is_bounded_and_never_panics() {
        for text in [
            "<".repeat(MAX_TEXT + 5),
            "<dyn ".repeat(50_000),
            "<dyn type=\"".repeat(20_000),
            "&".repeat(MAX_TEXT),
            "<é>é&é;<".repeat(20_000),
        ] {
            let (t, _, _) = run(&text);
            assert!(t.chars().count() <= MAX_TEXT + 1, "{}", t.chars().count());
        }
        let (t, _, n) = run(&"é".repeat(MAX_TEXT + 10));
        assert_eq!(t.chars().count(), MAX_TEXT);
        assert_eq!(n, [format!("T: text past {MAX_TEXT} characters, cut")]);
        // A tag that never closes, or runs past the longest tag, stays text.
        assert_eq!(run("a <dyn type=\"project\"").0, "a <dyn type=\"project\"");
        let long = format!("<dyn type=\"project\" property=\"name\" x=\"{}\"/>", "y".repeat(MAX_TAG));
        assert_eq!(run(&long).0, long);
        // Many different unknown tags are named a bounded number of times.
        let many: String = (0..100).map(|i| format!("<dyn type=\"u{i}\" property=\"p\"/>")).collect();
        assert_eq!(run(&many).2.len(), MAX_NAMED);
        // Line ends are made "\n".
        assert_eq!(run("a\r\nb\rc").0, "a\nb\nc");
    }
}
