//! Parsing an official sanctions list file that was uploaded by hand
//! (wl-parties §5.4, issue #42).
//!
//! A [`ListParser`] turns the bytes of a file into a [`ParsedList`], or says
//! why it cannot. [`XmlListParser`] is built in; a vertical adds another
//! format (Excel, say) with `WlParties::new().list_parser(format, parser)`.
//! No matching is built here: the entries are counted and their names kept.
//!
//! The XML reader is hand-written and small: `quick-xml` is not in the
//! lockfile and dependencies are not changed for this. It checks that the
//! document is well-formed (one root, properly nested and closed tags,
//! quoted attributes) and reads the entry elements.

use std::fmt;

use crate::sanctions_lists::ListFormat;

/// What a parser found in a list file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedList {
    /// How many entries the file has.
    pub entry_count: usize,
    /// The names of the entries that have one, in file order.
    pub entry_names: Vec<String>,
}

/// A file that cannot be read as a list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseError {}

/// Reads the bytes of an official list file.
pub trait ListParser: fmt::Debug + Send + Sync + 'static {
    /// Parses `bytes`; fails when they are not a valid list.
    fn parse(&self, bytes: &[u8]) -> Result<ParsedList, ParseError>;
}

/// The parsers of an app: the built-in XML parser plus the ones a vertical
/// adds. Provided on the app state by the module.
#[derive(Debug, Clone, Default)]
pub struct ListParsers {
    extra: Vec<(ListFormat, std::sync::Arc<dyn ListParser>)>,
}

impl ListParsers {
    /// No added parsers; usable in a `const`.
    pub const EMPTY: ListParsers = ListParsers { extra: Vec::new() };

    /// Adds (or replaces) the parser of `format`.
    pub fn with(
        mut self,
        format: ListFormat,
        parser: std::sync::Arc<dyn ListParser>,
    ) -> ListParsers {
        self.extra.retain(|(f, _)| *f != format);
        self.extra.push((format, parser));
        self
    }

    /// The parser of `format`, if there is one. `pdf` and `excel` have none
    /// unless a vertical adds it.
    pub fn get(&self, format: ListFormat) -> Option<std::sync::Arc<dyn ListParser>> {
        if let Some((_, p)) = self.extra.iter().find(|(f, _)| *f == format) {
            return Some(std::sync::Arc::clone(p));
        }
        match format {
            ListFormat::Xml => Some(std::sync::Arc::new(XmlListParser)),
            ListFormat::Pdf | ListFormat::Excel => None,
        }
    }
}

/// The element names that are one entry of a list, compared in lower case
/// without a namespace prefix (the UN, UAE and OFAC files use these).
const ENTRY_ELEMENTS: &[&str] = &[
    "individual",
    "entity",
    "entry",
    "record",
    "person",
    "designation",
    "sanctionentity",
    "target",
];

/// The element names that hold the name of an entry.
const NAME_ELEMENTS: &[&str] = &[
    "name",
    "whole_name",
    "wholename",
    "full_name",
    "fullname",
    "first_name",
    "firstname",
];

/// The built-in parser of XML lists.
#[derive(Debug, Clone, Copy, Default)]
pub struct XmlListParser;

impl ListParser for XmlListParser {
    fn parse(&self, bytes: &[u8]) -> Result<ParsedList, ParseError> {
        let text = std::str::from_utf8(bytes)
            .map_err(|_| ParseError("the file is not valid UTF-8 text".to_owned()))?;
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let parsed = read_entries(text)?;
        if parsed.entry_count == 0 {
            return Err(ParseError("the file has no entries".to_owned()));
        }
        Ok(parsed)
    }
}

fn bad(msg: impl Into<String>) -> ParseError {
    ParseError(format!("the file is not well-formed XML: {}", msg.into()))
}

fn local_name(name: &str) -> String {
    name.rsplit(':').next().unwrap_or(name).to_ascii_lowercase()
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | ':')
}

/// An open element.
struct Open {
    raw: String,
    /// Index into the entries when this element is an entry.
    entry: Option<usize>,
    /// Whether the text of this element is an entry name to keep.
    name_of: Option<usize>,
}

fn read_entries(text: &str) -> Result<ParsedList, ParseError> {
    let mut stack: Vec<Open> = Vec::new();
    let mut roots = 0usize;
    let mut names: Vec<Option<String>> = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let Some(lt) = rest.find('<') else {
            // Trailing text: only whitespace may stand outside the root.
            check_text(rest, stack.is_empty())?;
            break;
        };
        let (before, after) = rest.split_at(lt);
        check_text(before, stack.is_empty())?;
        let t = unescape(before.trim())?;
        if let Some(i) = stack.last().and_then(|top| top.name_of)
            && names[i].is_none()
            && !t.is_empty()
        {
            names[i] = Some(t);
        }
        if let Some(r) = after.strip_prefix("<!--") {
            let end = r
                .find("-->")
                .ok_or_else(|| bad("a comment is not closed"))?;
            rest = &r[end + 3..];
        } else if let Some(r) = after.strip_prefix("<![CDATA[") {
            let end = r
                .find("]]>")
                .ok_or_else(|| bad("a CDATA section is not closed"))?;
            if stack.is_empty() {
                return Err(bad("text outside the root element"));
            }
            if let Some(i) = stack.last().and_then(|t| t.name_of)
                && names[i].is_none()
                && !r[..end].trim().is_empty()
            {
                names[i] = Some(r[..end].trim().to_owned());
            }
            rest = &r[end + 3..];
        } else if let Some(r) = after.strip_prefix("<?") {
            let end = r
                .find("?>")
                .ok_or_else(|| bad("a processing instruction is not closed"))?;
            rest = &r[end + 2..];
        } else if let Some(r) = after.strip_prefix("<!") {
            if !r.to_ascii_uppercase().starts_with("DOCTYPE") || roots > 0 {
                return Err(bad("unexpected declaration"));
            }
            rest = skip_doctype(r)?;
        } else if let Some(r) = after.strip_prefix("</") {
            let end = r.find('>').ok_or_else(|| bad("a tag is not closed"))?;
            let name = r[..end].trim();
            let open = stack
                .pop()
                .ok_or_else(|| bad(format!("unexpected closing tag </{name}>")))?;
            if open.raw != name {
                return Err(bad(format!(
                    "</{name}> closes <{}>, which is still open",
                    open.raw
                )));
            }
            rest = &r[end + 1..];
        } else {
            let r = &after[1..];
            let (raw, self_closing, tail) = read_start_tag(r)?;
            if stack.is_empty() {
                roots += 1;
                if roots > 1 {
                    return Err(bad("more than one root element"));
                }
            }
            let tag = local_name(&raw);
            let in_entry = stack.iter().rev().find_map(|o| o.entry);
            let mut open = Open {
                raw,
                entry: None,
                name_of: None,
            };
            if ENTRY_ELEMENTS.contains(&tag.as_str()) && in_entry.is_none() {
                names.push(None);
                open.entry = Some(names.len() - 1);
            } else if let Some(i) = in_entry
                && names[i].is_none()
                && NAME_ELEMENTS.contains(&tag.as_str())
            {
                open.name_of = Some(i);
            }
            if !self_closing {
                stack.push(open);
            }
            rest = tail;
        }
    }
    if let Some(open) = stack.last() {
        return Err(bad(format!("<{}> is never closed", open.raw)));
    }
    if roots == 0 {
        return Err(bad("there is no root element"));
    }
    let entry_count = names.len();
    Ok(ParsedList {
        entry_count,
        entry_names: names.into_iter().flatten().collect(),
    })
}

fn check_text(text: &str, outside_root: bool) -> Result<(), ParseError> {
    if outside_root && !text.trim().is_empty() {
        return Err(bad("text outside the root element"));
    }
    Ok(())
}

/// Skips a `<!DOCTYPE ...>` (with an internal subset in brackets).
fn skip_doctype(r: &str) -> Result<&str, ParseError> {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    for (i, c) in r.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '[') => depth += 1,
            (None, ']') => depth -= 1,
            (None, '>') if depth <= 0 => return Ok(&r[i + 1..]),
            _ => {}
        }
    }
    Err(bad("the DOCTYPE is not closed"))
}

/// Reads `name attr="v" ... >` or `... />` after a `<`; returns the raw
/// name, whether it is self-closing and the text after the tag.
fn read_start_tag(r: &str) -> Result<(String, bool, &str), ParseError> {
    let name_end = r.find(|c: char| !is_name_char(c)).unwrap_or(r.len());
    let name = &r[..name_end];
    if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == '.') {
        return Err(bad("a tag has no valid name"));
    }
    let mut chars = r[name_end..].char_indices().peekable();
    let base = name_end;
    let mut seen: Vec<String> = Vec::new();
    loop {
        // Skip white space.
        let mut had_space = false;
        while let Some(&(_, c)) = chars.peek() {
            if c.is_whitespace() {
                had_space = true;
                chars.next();
            } else {
                break;
            }
        }
        match chars.next() {
            None => return Err(bad(format!("<{name}> is not closed"))),
            Some((i, '>')) => return Ok((name.to_owned(), false, &r[base + i + 1..])),
            Some((i, '/')) => {
                return match chars.next() {
                    Some((j, '>')) => Ok((name.to_owned(), true, &r[base + j + 1..])),
                    _ => Err(bad(format!("<{name}> has a stray '/' at {i}"))),
                };
            }
            Some((start, c)) if is_name_char(c) => {
                if !had_space {
                    return Err(bad(format!("<{name}> has an attribute without a space")));
                }
                let mut end = start + c.len_utf8();
                while let Some(&(k, c2)) = chars.peek() {
                    if is_name_char(c2) {
                        end = k + c2.len_utf8();
                        chars.next();
                    } else {
                        break;
                    }
                }
                let attr = r[base + start..base + end].to_owned();
                if seen.contains(&attr) {
                    return Err(bad(format!("<{name}> repeats the attribute {attr}")));
                }
                seen.push(attr.clone());
                while matches!(chars.peek(), Some(&(_, c)) if c.is_whitespace()) {
                    chars.next();
                }
                if !matches!(chars.next(), Some((_, '='))) {
                    return Err(bad(format!("attribute {attr} has no value")));
                }
                while matches!(chars.peek(), Some(&(_, c)) if c.is_whitespace()) {
                    chars.next();
                }
                let quote = match chars.next() {
                    Some((_, q @ ('"' | '\''))) => q,
                    _ => return Err(bad(format!("attribute {attr} is not quoted"))),
                };
                loop {
                    match chars.next() {
                        None => return Err(bad(format!("attribute {attr} is not closed"))),
                        Some((_, '<')) => {
                            return Err(bad(format!("attribute {attr} contains '<'")));
                        }
                        Some((_, c)) if c == quote => break,
                        Some(_) => {}
                    }
                }
            }
            Some(_) => return Err(bad(format!("<{name}> has an unexpected character"))),
        }
    }
}

/// Resolves the five predefined entities and numeric references; any other
/// `&` is not well-formed.
fn unescape(text: &str) -> Result<String, ParseError> {
    if !text.contains('&') {
        return Ok(text.to_owned());
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let r = &rest[i + 1..];
        let end = r.find(';').ok_or_else(|| bad("an '&' is not an entity"))?;
        let ent = &r[..end];
        match ent {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            _ => {
                let code = ent
                    .strip_prefix("#x")
                    .and_then(|h| u32::from_str_radix(h, 16).ok())
                    .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()));
                match code.and_then(char::from_u32) {
                    Some(c) => out.push(c),
                    None => return Err(bad(format!("unknown entity &{ent};"))),
                }
            }
        }
        rest = &r[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(xml: &str) -> Result<ParsedList, ParseError> {
        XmlListParser.parse(xml.as_bytes())
    }

    const UN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- list -->
<CONSOLIDATED_LIST dateGenerated="2026-11-03">
  <INDIVIDUALS>
    <INDIVIDUAL><DATAID>1</DATAID><FIRST_NAME>Omar</FIRST_NAME><SECOND_NAME>K</SECOND_NAME></INDIVIDUAL>
    <INDIVIDUAL><FIRST_NAME>Ali &amp; Sons</FIRST_NAME></INDIVIDUAL>
  </INDIVIDUALS>
  <ENTITIES><ENTITY><FIRST_NAME><![CDATA[Acme]]></FIRST_NAME></ENTITY><ENTITY/></ENTITIES>
</CONSOLIDATED_LIST>"#;

    #[test]
    fn a_well_formed_list_has_its_entries_counted_and_named() {
        let list = parse(UN).unwrap();
        assert_eq!(list.entry_count, 4);
        assert_eq!(list.entry_names, ["Omar", "Ali & Sons", "Acme"]);
    }

    #[test]
    fn a_namespace_prefix_a_doctype_and_a_bom_are_accepted() {
        let xml = "\u{feff}<!DOCTYPE l [<!ELEMENT l ANY>]><l:list xmlns:l=\"urn:x\"><l:Entry><l:Name>A</l:Name></l:Entry></l:list>";
        let list = parse(xml).unwrap();
        assert_eq!((list.entry_count, list.entry_names), (1, vec!["A".into()]));
    }

    #[test]
    fn a_document_without_entries_is_refused() {
        let err = parse("<list><meta/></list>").unwrap_err();
        assert_eq!(err.0, "the file has no entries");
    }

    #[test]
    fn a_document_that_is_not_well_formed_is_refused() {
        for xml in [
            "",
            "   ",
            "not xml",
            "<list><entry></list>",
            "<list><entry></entry>",
            "<list></list><list></list>",
            "<list><entry a=1/></list>",
            "<list><entry a=\"1\" a=\"2\"/></list>",
            "<list><entry a=\"1\"b=\"2\"/></list>",
            "<list><entry/></list> trailing",
            "<list><entry>a & b</entry></list>",
            "<list><!-- open <entry/></list>",
            "<list><entry",
            "</list>",
            "<1list><entry/></1list>",
        ] {
            let err = parse(xml).unwrap_err();
            assert!(err.0.contains("not well-formed"), "{xml:?} gave {err}");
        }
    }

    #[test]
    fn bytes_that_are_not_text_are_refused() {
        let err = XmlListParser.parse(&[0xff, 0xfe, 0x00]).unwrap_err();
        assert!(err.0.contains("UTF-8"));
        let pdf = XmlListParser.parse(b"%PDF-1.7 \x00\x01").unwrap_err();
        assert!(pdf.0.contains("UTF-8") || pdf.0.contains("well-formed"));
    }

    #[test]
    fn the_registry_knows_xml_only_until_a_vertical_adds_a_parser() {
        let parsers = ListParsers::default();
        assert!(parsers.get(ListFormat::Xml).is_some());
        assert!(parsers.get(ListFormat::Pdf).is_none());
        assert!(parsers.get(ListFormat::Excel).is_none());
        let parsers = parsers.with(ListFormat::Excel, std::sync::Arc::new(XmlListParser));
        assert!(parsers.get(ListFormat::Excel).is_some());
    }
}
