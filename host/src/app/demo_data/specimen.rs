//! The files of the demo's documents: a one-page PDF marked "SPECIMEN - DEMO"
//! (DEMO-DATA.md 1.4: sample documents are clearly watermarked, and never a
//! real person's).

/// A valid one-page PDF that says `title` and "SPECIMEN - DEMO".
pub fn pdf(title: &str) -> Vec<u8> {
    let safe: String = title
        .chars()
        .filter(|c| c.is_ascii() && !matches!(c, '(' | ')' | '\\'))
        .collect();
    let content = format!(
        "BT /F1 22 Tf 72 740 Td ({safe}) Tj ET\nBT /F1 40 Tf 72 400 Td (SPECIMEN - DEMO) Tj ET\n"
    );
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
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}
