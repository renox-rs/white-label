//! The return note of a dispatched Return to supplier (wl-inventory §4
//! *Return to supplier* `dispatch`): a one-page PDF the supplier's driver
//! signs for, with the number, the supplier, the source, the collection
//! proof and the lines. The document is plain ASCII (Helvetica, no embedded
//! font), so it is kept as text on the return and served as it is; letters
//! outside ASCII are written as `?`.

use crate::item::quantity_text;

/// One line of the note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteLine {
    /// The item's code.
    pub item: String,
    /// The lot number, if any.
    pub lot: Option<String>,
    /// The quantity, in thousandths of the base unit.
    pub quantity_milli: i64,
    /// The base unit: `kg`.
    pub unit: String,
    /// Why the goods go back.
    pub reason: String,
    /// `credit` or `replace`.
    pub outcome: String,
}

/// What the note says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    /// The return's number: `RTS-2026-0001`.
    pub number: String,
    /// The supplier, as `Supplier 7`.
    pub supplier: String,
    /// The record the goods came from: `inventory.goods_receipt:5`.
    pub source: String,
    /// The collection proof.
    pub proof: String,
    /// The day it was dispatched, `2026-10-21`.
    pub date: String,
    /// The lines.
    pub lines: Vec<NoteLine>,
}

/// `text` with letters outside ASCII written as `?` and the characters a PDF
/// string treats specially escaped.
fn pdf_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '(' | ')' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_ascii() && !c.is_ascii_control() => out.push(c),
            _ => out.push('?'),
        }
    }
    out
}

/// The lines of text of the note, top to bottom.
fn text_lines(note: &Note) -> Vec<(i64, String)> {
    let mut lines = vec![
        (16, format!("Return note {}", note.number)),
        (11, format!("Date: {}", note.date)),
        (11, format!("Supplier: {}", note.supplier)),
        (11, format!("Goods from: {}", note.source)),
        (11, format!("Collection proof: {}", note.proof)),
        (11, String::new()),
        (12, "Goods returned".to_owned()),
    ];
    for (position, line) in (1..).zip(&note.lines) {
        let lot = line
            .lot
            .as_deref()
            .map(|lot| format!(" lot {lot}"))
            .unwrap_or_default();
        lines.push((
            11,
            format!(
                "{position}. {}{lot}: {} {} - {} ({})",
                line.item,
                quantity_text(line.quantity_milli),
                line.unit,
                line.reason,
                line.outcome
            ),
        ));
    }
    lines.push((11, String::new()));
    lines.push((
        11,
        "Received by (driver signature): ________________".to_owned(),
    ));
    lines
}

/// The PDF document of `note`.
pub fn render(note: &Note) -> String {
    document(&text_lines(note))
}

/// A one-page PDF document of `lines`, each a font size and its text,
/// top to bottom; shared with the purchase order document.
pub(crate) fn document(lines: &[(i64, String)]) -> String {
    let mut content = String::new();
    let mut y = 800;
    for (size, line) in lines {
        let (size, line) = (*size, line.as_str());
        if !line.is_empty() {
            content.push_str(&format!(
                "BT /F1 {size} Tf 50 {y} Td ({}) Tj ET\n",
                pdf_text(line)
            ));
        }
        y -= size + 8;
    }
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R >> >> >>"
            .to_owned(),
        format!(
            "<< /Length {} >>\nstream\n{content}endstream",
            content.len()
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    ];
    let mut pdf = String::from("%PDF-1.4\n");
    let mut offsets = Vec::new();
    for (number, body) in (1..).zip(&objects) {
        offsets.push(pdf.len());
        pdf.push_str(&format!("{number} 0 obj\n{body}\nendobj\n"));
    }
    let xref = pdf.len();
    pdf.push_str(&format!(
        "xref\n0 {}\n0000000000 65535 f \n",
        objects.len() + 1
    ));
    for offset in offsets {
        pdf.push_str(&format!("{offset:010} 00000 n \n"));
    }
    pdf.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    ));
    pdf
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note() -> Note {
        Note {
            number: "RTS-2026-0001".into(),
            supplier: "Supplier 7".into(),
            source: "inventory.goods_receipt:5".into(),
            proof: "driver_signature:SIG-1".into(),
            date: "2026-10-21".into(),
            lines: vec![NoteLine {
                item: "CHICKEN (frozen)".into(),
                lot: Some("L-1".into()),
                quantity_milli: 10_500,
                unit: "kg".into(),
                reason: "damaged".into(),
                outcome: "credit".into(),
            }],
        }
    }

    #[test]
    fn the_note_is_a_pdf_with_its_number_and_lines() {
        let pdf = render(&note());
        assert!(pdf.starts_with("%PDF-1.4\n"));
        assert!(pdf.ends_with("%%EOF\n"));
        assert!(pdf.contains("Return note RTS-2026-0001"));
        assert!(pdf.contains("1. CHICKEN \\(frozen\\) lot L-1: 10.5 kg - damaged \\(credit\\)"));
        assert!(pdf.contains("driver_signature:SIG-1"));
    }

    #[test]
    fn the_cross_reference_points_at_every_object() {
        let pdf = render(&note());
        let table = pdf.split("xref\n").nth(1).unwrap();
        let entries: Vec<&str> = table.lines().skip(2).take(5).collect();
        for (number, entry) in (1..).zip(entries) {
            let offset: usize = entry[..10].parse().unwrap();
            assert!(
                pdf[offset..].starts_with(&format!("{number} 0 obj")),
                "{number}"
            );
        }
        let start: usize = pdf
            .split("startxref\n")
            .nth(1)
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(pdf[start..].starts_with("xref\n"));
    }

    #[test]
    fn letters_outside_ascii_do_not_break_the_document() {
        let mut other = note();
        other.supplier = "Al Noor \u{0645}".into();
        let pdf = render(&other);
        assert!(pdf.is_ascii());
        assert!(pdf.contains("Al Noor ?"));
    }
}
