//! The built-in PDF renderer (wl-documents §5.3 point 3).
//!
//! The workspace does not depend on `renox-pdf` (R3) yet: it is not in the
//! lock file, and the generation pipeline must not wait for it. This
//! renderer writes a small, uncompressed PDF 1.4 by hand with what the
//! pipeline needs today: the client's logo and accent colour in the header,
//! the body in English, Arabic or both side by side, and a footer with the
//! document number and "Page n of N" on every page.
//!
//! Arabic lines are laid out right to left: each line is put in visual order
//! ([`visual_order`]: the runs reversed, Arabic letters reversed, numbers
//! and Latin words kept left to right) and aligned to the right margin.
//! Glyph shaping (joining forms) and an embedded Arabic font are the job of
//! the font engine; they come with the `renox-pdf` renderer, which replaces
//! this one through [`PdfRendering`](super::PdfRendering) without changing
//! the pipeline.
//!
//! Logos: JPEG files are embedded as they are; PNG files without
//! transparency (greyscale, RGB or palette, 8 bits, not interlaced) are
//! embedded with their compressed data. Other formats are left out and the
//! company name stands alone.

use std::fmt::Write as _;

use wl_kernel::generation::LanguageMode;

use super::{PaperSize, RenderError, RenderJob};

/// Points between two lines of body text.
const LINE_HEIGHT: f32 = 14.0;
/// The body font size.
const FONT_SIZE: f32 = 10.0;
/// The page margin.
const MARGIN: f32 = 40.0;
/// The logo's height in the header.
const LOGO_HEIGHT: f32 = 40.0;

/// The page size in points.
fn page_size(paper: PaperSize) -> (f32, f32) {
    match paper {
        PaperSize::A4 => (595.0, 842.0),
        PaperSize::Letter => (612.0, 792.0),
    }
}

/// Whether `c` is written right to left (Hebrew, Arabic and their
/// presentation forms).
fn is_rtl(c: char) -> bool {
    matches!(u32::from(c), 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF)
}

/// Whether `c` is a strong left-to-right character in an Arabic line: Latin
/// letters and digits (Arabic-Indic digits too, which read left to right).
fn is_ltr(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(u32::from(c), 0x0660..=0x0669 | 0x06F0..=0x06F9)
        || (c.is_alphabetic() && !is_rtl(c))
}

/// The mirror of a bracket in a right-to-left run.
fn mirror(c: char) -> char {
    match c {
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        '{' => '}',
        '}' => '{',
        '<' => '>',
        '>' => '<',
        '«' => '»',
        '»' => '«',
        other => other,
    }
}

/// An Arabic `line` in visual order, as it is drawn from left to right: the
/// runs in reverse order, letters and neutral characters of each
/// right-to-left run reversed (brackets mirrored), numbers and Latin words
/// kept as they are. Separators between two left-to-right characters
/// (`2026-10`, `1,250.00`, `A/12`) stay inside the number.
pub fn visual_order(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let ltr_at = |i: usize| -> bool {
        let c = chars[i];
        if is_ltr(c) {
            return true;
        }
        matches!(c, '.' | ',' | '-' | '/' | ':' | '%' | '+')
            && i > 0
            && i + 1 < chars.len()
            && is_ltr(chars[i - 1])
            && is_ltr(chars[i + 1])
    };
    // Runs of (left-to-right?, text).
    let mut runs: Vec<(bool, String)> = Vec::new();
    for i in 0..chars.len() {
        let ltr = ltr_at(i);
        match runs.last_mut() {
            Some((kind, text)) if *kind == ltr => text.push(chars[i]),
            _ => runs.push((ltr, chars[i].to_string())),
        }
    }
    let mut out = String::with_capacity(line.len());
    for (ltr, text) in runs.into_iter().rev() {
        if ltr {
            out.push_str(&text);
        } else {
            out.extend(text.chars().rev().map(mirror));
        }
    }
    out
}

/// Whether `line` is mostly right to left (its first strong character).
fn starts_rtl(line: &str) -> bool {
    line.chars()
        .find(|c| is_rtl(*c) || is_ltr(*c))
        .is_some_and(is_rtl)
}

/// Splits `text` into lines of at most `width` characters, at spaces when
/// it can.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for raw in text.lines() {
        let raw = raw.trim_end();
        if raw.chars().count() <= width {
            lines.push(raw.to_owned());
            continue;
        }
        let mut current = String::new();
        for word in raw.split(' ') {
            let fits = current.chars().count() + word.chars().count() < width;
            if !current.is_empty() && !fits {
                lines.push(std::mem::take(&mut current));
            }
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
            while current.chars().count() > width {
                let head: String = current.chars().take(width).collect();
                let tail: String = current.chars().skip(width).collect();
                lines.push(head);
                current = tail;
            }
        }
        lines.push(current);
    }
    lines
}

/// `text` as a PDF literal string in WinAnsi: Latin-1 characters escaped in
/// octal, anything else as `?`.
fn literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('(');
    for c in text.chars() {
        match c {
            '(' | ')' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            ' '..='~' => out.push(c),
            c if u32::from(c) <= 0xFF => {
                let _ = write!(out, "\\{:03o}", u32::from(c));
            }
            _ => out.push('?'),
        }
    }
    out.push(')');
    out
}

/// `text` as a PDF hex string of UTF-16BE code units, for the Arabic font.
fn hex_utf16(text: &str) -> String {
    let mut out = String::from("<");
    for unit in text.encode_utf16() {
        let _ = write!(out, "{unit:04X}");
    }
    out.push('>');
    out
}

/// The estimated width of `text` at `size`: half the size per character,
/// enough to align to the right margin.
fn width(text: &str, size: f32) -> f32 {
    #[allow(clippy::cast_precision_loss)]
    let count = text.chars().count() as f32;
    count * size * 0.5
}

/// A colour `#rrggbb` as PDF RGB components.
fn rgb(colour: &str) -> (f32, f32, f32) {
    let hex = colour.trim().trim_start_matches('#');
    let part = |i: usize| {
        hex.get(i..i + 2)
            .and_then(|p| u8::from_str_radix(p, 16).ok())
            .map_or(0.0, |v| f32::from(v) / 255.0)
    };
    if hex.len() == 6 {
        (part(0), part(2), part(4))
    } else {
        (0.04, 0.43, 0.4)
    }
}

/// An image the PDF embeds.
struct Image {
    width: u32,
    height: u32,
    dict: String,
    data: Vec<u8>,
}

/// The logo as a PDF image, when its format can be embedded.
fn logo_image(bytes: &[u8]) -> Option<Image> {
    if bytes.starts_with(&[0xFF, 0xD8]) {
        return jpeg_image(bytes);
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return png_image(bytes);
    }
    None
}

/// A JPEG, embedded as it is (`DCTDecode`).
fn jpeg_image(bytes: &[u8]) -> Option<Image> {
    let mut i = 2;
    while i + 9 < bytes.len() {
        if bytes[i] != 0xFF {
            return None;
        }
        let marker = bytes[i + 1];
        let length = usize::from(u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]));
        // Start of frame: baseline, extended, progressive.
        if matches!(marker, 0xC0..=0xC2) {
            let height = u32::from(u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]));
            let width = u32::from(u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]));
            let components = bytes[i + 9];
            let space = match components {
                1 => "/DeviceGray",
                4 => "/DeviceCMYK",
                _ => "/DeviceRGB",
            };
            return Some(Image {
                width,
                height,
                dict: format!(
                    "/ColorSpace {space} /BitsPerComponent 8 /Filter /DCTDecode"
                ),
                data: bytes.to_vec(),
            });
        }
        i += 2 + length;
    }
    None
}

/// A PNG without transparency, embedded with its compressed data
/// (`FlateDecode` with the PNG predictors).
fn png_image(bytes: &[u8]) -> Option<Image> {
    let mut i = 8;
    let mut header: Option<(u32, u32, u8, u8, u8)> = None;
    let mut palette: Vec<u8> = Vec::new();
    let mut data: Vec<u8> = Vec::new();
    while i + 8 <= bytes.len() {
        let length = u32::from_be_bytes(bytes[i..i + 4].try_into().ok()?) as usize;
        let kind = &bytes[i + 4..i + 8];
        let body = bytes.get(i + 8..i + 8 + length)?;
        match kind {
            b"IHDR" if length >= 13 => {
                let width = u32::from_be_bytes(body[0..4].try_into().ok()?);
                let height = u32::from_be_bytes(body[4..8].try_into().ok()?);
                header = Some((width, height, body[8], body[9], body[12]));
            }
            b"PLTE" => palette = body.to_vec(),
            b"IDAT" => data.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
        i += 12 + length;
    }
    let (width, height, depth, colour, interlace) = header?;
    if depth != 8 || interlace != 0 || data.is_empty() {
        return None;
    }
    let (space, colours) = match colour {
        0 => ("/DeviceGray".to_owned(), 1),
        2 => ("/DeviceRGB".to_owned(), 3),
        3 if !palette.is_empty() => {
            let mut hex = String::new();
            for byte in &palette {
                let _ = write!(hex, "{byte:02X}");
            }
            (
                format!("[/Indexed /DeviceRGB {} <{hex}>]", palette.len() / 3 - 1),
                1,
            )
        }
        // Transparency needs the alpha channel split out: left out.
        _ => return None,
    };
    Some(Image {
        width,
        height,
        dict: format!(
            "/ColorSpace {space} /BitsPerComponent 8 /Filter /FlateDecode \
             /DecodeParms << /Predictor 15 /Colors {colours} /BitsPerComponent 8 /Columns {width} >>"
        ),
        data,
    })
}

/// One line of a page: English on the left, Arabic on the right.
struct Row {
    english: Option<String>,
    arabic: Option<String>,
}

/// The rows of the body, wrapped for the job's language mode.
fn rows(job: &RenderJob) -> Vec<Row> {
    let (page_width, _) = page_size(job.paper);
    let columns = if job.mode == LanguageMode::Bilingual {
        2.0
    } else {
        1.0
    };
    let usable = (page_width - 2.0 * MARGIN - (columns - 1.0) * 20.0) / columns;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let chars = (usable / (FONT_SIZE * 0.5)).floor().max(10.0) as usize;
    let english = if job.mode.has_english() {
        wrap(&job.english, chars)
    } else {
        Vec::new()
    };
    let arabic = if job.mode.has_arabic() {
        wrap(&job.arabic, chars)
    } else {
        Vec::new()
    };
    let count = english.len().max(arabic.len());
    (0..count)
        .map(|i| Row {
            english: english.get(i).cloned(),
            arabic: arabic.get(i).cloned(),
        })
        .collect()
}

/// Writes one line of text: Latin in `F1`, a right-to-left line in `F2`
/// in visual order, aligned to `right` when it is Arabic.
fn text_line(content: &mut String, text: &str, x: f32, right: f32, y: f32, size: f32) {
    if text.trim().is_empty() {
        return;
    }
    if starts_rtl(text) || text.chars().any(is_rtl) {
        let visual = visual_order(text);
        let x = (right - width(&visual, size)).max(MARGIN);
        let _ = writeln!(
            content,
            "BT /F2 {size} Tf {x:.2} {y:.2} Td {} Tj ET",
            hex_utf16(&visual)
        );
    } else {
        let _ = writeln!(
            content,
            "BT /F1 {size} Tf {x:.2} {y:.2} Td {} Tj ET",
            literal(text)
        );
    }
}

/// Renders `job` to the bytes of a PDF file.
///
/// # Errors
///
/// [`RenderError`] when the job has no text at all.
pub fn render(job: &RenderJob) -> Result<Vec<u8>, RenderError> {
    let body = rows(job);
    if body.is_empty() {
        return Err(RenderError::new("the template has no text"));
    }
    let (page_width, page_height) = page_size(job.paper);
    let right = page_width - MARGIN;
    let top = page_height - MARGIN - LOGO_HEIGHT - 30.0;
    let footer_lines =
        usize::from(job.footer_en.is_some()) + usize::from(job.footer_ar.is_some());
    #[allow(clippy::cast_precision_loss)]
    let bottom = MARGIN + 20.0 + footer_lines as f32 * LINE_HEIGHT;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let per_page = (((top - bottom) / LINE_HEIGHT).floor().max(1.0)) as usize;
    let pages: Vec<&[Row]> = body.chunks(per_page).collect();
    let page_count = pages.len();
    let logo = job.brand.logo.as_deref().and_then(logo_image);

    // Fixed objects: 1 catalog, 2 pages, 3 Latin font, 4 Arabic font,
    // 5 its descendant, 6 info, 7 the logo (when there is one); then each
    // page and its content.
    let first_page = if logo.is_some() { 8 } else { 7 };
    let mut objects: Vec<Vec<u8>> = Vec::new();
    let kids: Vec<String> = (0..page_count)
        .map(|i| format!("{} 0 R", first_page + 2 * i))
        .collect();
    let lang = if job.mode == LanguageMode::Ar {
        "ar"
    } else {
        "en"
    };
    objects.push(format!("<< /Type /Catalog /Pages 2 0 R /Lang {} >>", literal(lang)).into_bytes());
    objects.push(
        format!(
            "<< /Type /Pages /Kids [{}] /Count {page_count} >>",
            kids.join(" ")
        )
        .into_bytes(),
    );
    objects.push(
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_vec(),
    );
    objects.push(
        b"<< /Type /Font /Subtype /Type0 /BaseFont /NotoNaskhArabic /Encoding /Identity-H /DescendantFonts [5 0 R] >>"
            .to_vec(),
    );
    objects.push(
        b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /NotoNaskhArabic /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> >>"
            .to_vec(),
    );
    objects.push(
        format!(
            "<< /Title {} /Producer (wl-documents) >>",
            literal(&job.title)
        )
        .into_bytes(),
    );
    if let Some(image) = &logo {
        let mut object = format!(
            "<< /Type /XObject /Subtype /Image /Width {} /Height {} {} /Length {} >>\nstream\n",
            image.width,
            image.height,
            image.dict,
            image.data.len()
        )
        .into_bytes();
        object.extend_from_slice(&image.data);
        object.extend_from_slice(b"\nendstream");
        objects.push(object);
    }

    let (r, g, b) = rgb(&job.brand.accent);
    for (index, rows) in pages.iter().enumerate() {
        let mut content = String::new();
        // Header: the logo at the start, the company names, an accent rule.
        let header_y = page_height - MARGIN - LOGO_HEIGHT;
        let mut name_x = MARGIN;
        if let Some(image) = &logo {
            #[allow(clippy::cast_precision_loss)]
            let logo_width = LOGO_HEIGHT * image.width as f32 / image.height.max(1) as f32;
            let _ = writeln!(
                content,
                "q {logo_width:.2} 0 0 {LOGO_HEIGHT:.2} {MARGIN:.2} {header_y:.2} cm /Logo Do Q"
            );
            name_x += logo_width + 10.0;
        }
        text_line(
            &mut content,
            &job.brand.company_name,
            name_x,
            right,
            header_y + 14.0,
            12.0,
        );
        if let Some(name_ar) = job.brand.company_name_ar.as_deref() {
            text_line(&mut content, name_ar, name_x, right, header_y + 14.0, 12.0);
        }
        text_line(&mut content, &job.title, MARGIN, right, header_y - 18.0, 12.0);
        let rule_y = header_y - 26.0;
        let _ = writeln!(
            content,
            "{r:.3} {g:.3} {b:.3} RG 1.5 w {MARGIN:.2} {rule_y:.2} m {right:.2} {rule_y:.2} l S"
        );
        // Body.
        let column_end = if job.mode == LanguageMode::Bilingual {
            page_width / 2.0 - 10.0
        } else {
            right
        };
        let mut y = top;
        for row in *rows {
            if let Some(english) = &row.english {
                text_line(&mut content, english, MARGIN, column_end, y, FONT_SIZE);
            }
            if let Some(arabic) = &row.arabic {
                let start = if job.mode == LanguageMode::Bilingual {
                    page_width / 2.0 + 10.0
                } else {
                    MARGIN
                };
                text_line(&mut content, arabic, start, right, y, FONT_SIZE);
            }
            y -= LINE_HEIGHT;
        }
        // Footer: the template's and entity's footer text, the number, the
        // page count.
        let mut footer_y = MARGIN + 20.0;
        for footer in [&job.footer_ar, &job.footer_en].into_iter().flatten() {
            text_line(&mut content, footer, MARGIN, right, footer_y, 8.0);
            footer_y += LINE_HEIGHT;
        }
        if let Some(number) = &job.number {
            text_line(&mut content, number, MARGIN, right, MARGIN, 8.0);
        }
        let page_text = format!("Page {} of {page_count}", index + 1);
        let page_x = right - width(&page_text, 8.0);
        text_line(&mut content, &page_text, page_x, right, MARGIN, 8.0);

        let resources = if logo.is_some() {
            "<< /Font << /F1 3 0 R /F2 4 0 R >> /XObject << /Logo 7 0 R >> >>"
        } else {
            "<< /Font << /F1 3 0 R /F2 4 0 R >> >>"
        };
        let content_id = first_page + 2 * index + 1;
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {page_width} {page_height}] \
                 /Resources {resources} /Contents {content_id} 0 R >>"
            )
            .into_bytes(),
        );
        let mut stream =
            format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
        stream.extend_from_slice(content.as_bytes());
        stream.extend_from_slice(b"endstream");
        objects.push(stream);
    }

    let mut out: Vec<u8> = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(object);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    let mut table = format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1);
    for offset in offsets {
        let _ = writeln!(table, "{offset:010} 00000 n ");
    }
    let _ = write!(
        table,
        "trailer\n<< /Size {} /Root 1 0 R /Info 6 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    );
    out.extend_from_slice(table.as_bytes());
    Ok(out)
}

/// The text a PDF written by [`render`] shows, one entry per text object in
/// drawing order: literal strings decoded from WinAnsi, hex strings from
/// UTF-16BE (so an Arabic line reads in its visual order). For tests and
/// checks; not a general PDF reader.
pub fn text_lines(pdf: &[u8]) -> Vec<String> {
    let text: String = pdf.iter().map(|b| char::from(*b)).collect();
    let mut lines = Vec::new();
    let mut rest = text.as_str();
    while let Some(start) = rest.find("BT /F") {
        let after = &rest[start..];
        let Some(end) = after.find(" ET") else { break };
        let object = &after[..end];
        if let Some(open) = object.find('(') {
            if let Some(close) = object.rfind(") Tj") {
                lines.push(decode_literal(&object[open + 1..close]));
            }
        } else if let Some(open) = object.find('<')
            && let Some(close) = object.rfind("> Tj")
        {
            lines.push(decode_hex(&object[open + 1..close]));
        }
        rest = &after[end..];
    }
    lines
}

/// A literal string's text (escapes and octal codes read as Latin-1).
fn decode_literal(body: &str) -> String {
    let chars: Vec<char> = body.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            let next = chars[i + 1];
            if next.is_ascii_digit() {
                let digits: String = chars[i + 1..].iter().take(3).collect();
                if let Ok(code) = u32::from_str_radix(&digits, 8) {
                    out.extend(char::from_u32(code));
                }
                i += 4;
            } else {
                out.push(next);
                i += 2;
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// A hex string's text, read as UTF-16BE.
fn decode_hex(body: &str) -> String {
    let units: Vec<u16> = body
        .as_bytes()
        .chunks(4)
        .filter_map(|chunk| std::str::from_utf8(chunk).ok())
        .filter_map(|hex| u16::from_str_radix(hex, 16).ok())
        .collect();
    String::from_utf16_lossy(&units)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_arabic_line_reads_right_to_left_with_numbers_kept() {
        assert_eq!(visual_order("كشف حساب"), "باسح فشك");
        assert_eq!(visual_order("الفترة 2026-10"), "2026-10 ةرتفلا");
        assert_eq!(visual_order("(المالك)"), "(كلاملا)");
    }

    #[test]
    fn long_lines_wrap_at_spaces() {
        assert_eq!(wrap("one two three", 8), vec!["one two", "three"]);
        assert_eq!(wrap("short", 8), vec!["short"]);
    }

    #[test]
    fn literals_escape_and_hex_round_trips() {
        assert_eq!(literal("a (b) \\"), "(a \\(b\\) \\\\)");
        assert_eq!(decode_literal("a \\(b\\) \\351"), "a (b) é");
        let hex = hex_utf16("سلام");
        assert_eq!(decode_hex(hex.trim_matches(['<', '>'])), "سلام");
    }
}
