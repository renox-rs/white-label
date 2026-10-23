//! Reading an aggregator's payout file (06 F&B §4.3 step 1, `FNB.REC.01`,
//! decided 2026-10-15: CSV or Excel with a column-mapping preset per
//! aggregator).
//!
//! **CSV only.** The platform has no Excel reader yet (the bank statement
//! import of wl-payments says the same): an Excel file is saved as CSV from
//! the partner portal's download. The preset of the aggregator
//! ([`ImportPreset`]) names the header of each value and the separator; a
//! header is matched trimmed and without case. A field may be in double quotes
//! (with `""` for a quote). Amounts are dirhams as printed (`1,250.50`,
//! `AED 25.00`, `(25.00)`, `-25.00`); commission, promotion share and refund
//! are kept positive, whatever sign the file prints them with. A file with no
//! net column gets `net = gross - commission - promo - refund`.

use crate::money::parse_dirhams;
use crate::payout::ImportPreset;

/// One line of the file, in fils.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedLine {
    /// The position in the file, from 1.
    pub line_no: i64,
    /// The aggregator's order number.
    pub order_number: String,
    /// The gross.
    pub gross: i64,
    /// The commission (positive).
    pub commission: i64,
    /// The promotion share (positive).
    pub promo: i64,
    /// The refund (positive).
    pub refund: i64,
    /// The net payout of the line.
    pub net: i64,
    /// The aggregator's refund reason.
    pub reason: String,
}

/// Splits `content` into rows of fields with `delimiter`; quotes protect the
/// delimiter and line breaks.
pub fn rows(content: &str, delimiter: char) -> Vec<Vec<String>> {
    let content = content.trim_start_matches('\u{feff}');
    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = content.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    let _ = chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
        } else if c == '"' {
            quoted = true;
        } else if c == delimiter {
            row.push(std::mem::take(&mut field));
        } else if c == '\n' || c == '\r' {
            if c == '\r' && chars.peek() == Some(&'\n') {
                let _ = chars.next();
            }
            row.push(std::mem::take(&mut field));
            if row.iter().any(|f| !f.trim().is_empty()) {
                rows.push(std::mem::take(&mut row));
            } else {
                row.clear();
            }
        } else {
            field.push(c);
        }
    }
    row.push(field);
    if row.iter().any(|f| !f.trim().is_empty()) {
        rows.push(row);
    }
    rows
}

/// An amount as printed, in fils, with its sign. `None` for text that is not
/// an amount; an empty field is 0.
pub fn amount(text: &str, delimiter: char) -> Option<i64> {
    let mut text = text.trim().to_owned();
    if text.is_empty() {
        return Some(0);
    }
    let mut negative = false;
    if text.starts_with('(') && text.ends_with(')') {
        negative = true;
        text = text[1..text.len() - 1].to_owned();
    }
    let text = text
        .replace("AED", "")
        .replace("aed", "")
        .replace(['\u{a0}', ' '], "");
    let text = if let Some(rest) = text.strip_prefix('-') {
        negative = true;
        rest.to_owned()
    } else {
        text
    };
    // A comma is a thousands separator unless it is the file's delimiter's
    // alternative decimal comma (`12,50` in a semicolon file).
    let text = if delimiter != ',' && text.matches(',').count() == 1 && !text.contains('.') {
        text.replace(',', ".")
    } else {
        text.replace(',', "")
    };
    let fils = parse_dirhams(&text)?;
    Some(if negative { -fils } else { fils })
}

/// Reads `content` with `preset`.
///
/// # Errors
///
/// The reason the file cannot be read, in words for the accountant: the order
/// column is missing, a row has no order number or an amount is not an amount.
pub fn parse(content: &str, preset: &ImportPreset) -> Result<Vec<ParsedLine>, String> {
    let delimiter = preset.delimiter_char();
    let table = rows(content, delimiter);
    let Some((header, body)) = table.split_first() else {
        return Err("The file is empty.".to_owned());
    };
    let find = |name: &str| -> Option<usize> {
        let name = name.trim().to_lowercase();
        if name.is_empty() {
            return None;
        }
        header
            .iter()
            .position(|h| h.trim().trim_start_matches('\u{feff}').to_lowercase() == name)
    };
    let Some(order_at) = find(preset.order_column.as_str()) else {
        return Err(format!(
            "The file has no column \"{}\" for the order number.",
            preset.order_column
        ));
    };
    let columns = [
        &preset.gross_column,
        &preset.commission_column,
        &preset.promo_column,
        &preset.refund_column,
        &preset.net_column,
    ];
    for name in columns {
        if !name.trim().is_empty() && find(name.as_str()).is_none() {
            return Err(format!("The file has no column \"{name}\"."));
        }
    }
    let gross_at = find(preset.gross_column.as_str());
    let commission_at = find(preset.commission_column.as_str());
    let promo_at = find(preset.promo_column.as_str());
    let refund_at = find(preset.refund_column.as_str());
    let net_at = find(preset.net_column.as_str());
    let reason_at = find(preset.reason_column.as_str());
    let mut lines = Vec::new();
    for (i, row) in body.iter().enumerate() {
        let number = i64::try_from(i + 1).unwrap_or(i64::MAX);
        let cell =
            |at: Option<usize>| -> &str { at.and_then(|at| row.get(at)).map_or("", |f| f.trim()) };
        let order_number = cell(Some(order_at)).to_owned();
        if order_number.is_empty() {
            return Err(format!("Line {number} has no order number."));
        }
        let money = |at: Option<usize>| -> Result<i64, String> {
            amount(cell(at), delimiter)
                .ok_or_else(|| format!("Line {number}: \"{}\" is not an amount.", cell(at)))
        };
        let gross = money(gross_at)?;
        let commission = money(commission_at)?.abs();
        let promo = money(promo_at)?.abs();
        let refund = money(refund_at)?.abs();
        let net = if net_at.is_some() {
            money(net_at)?
        } else {
            gross - commission - promo - refund
        };
        lines.push(ParsedLine {
            line_no: number,
            order_number,
            gross,
            commission,
            promo,
            refund,
            net,
            reason: cell(reason_at).to_owned(),
        });
    }
    if lines.is_empty() {
        return Err("The file has a header and no lines.".to_owned());
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset() -> ImportPreset {
        ImportPreset {
            delimiter: ",".to_owned(),
            order_column: "Order ID".to_owned(),
            gross_column: "Gross".to_owned(),
            commission_column: "Commission".to_owned(),
            promo_column: "Promo".to_owned(),
            refund_column: "Refund".to_owned(),
            net_column: "Net".to_owned(),
            reason_column: "Refund reason".to_owned(),
            ..ImportPreset::default()
        }
    }

    #[test]
    fn a_quoted_field_keeps_its_comma_and_its_quote() {
        let table = rows("a,\"b,c\",\"d\"\"e\"\r\n1,2,3\n", ',');
        assert_eq!(table[0], vec!["a", "b,c", "d\"e"]);
        assert_eq!(table[1], vec!["1", "2", "3"]);
    }

    #[test]
    fn amounts_are_read_as_printed() {
        assert_eq!(amount("1,250.50", ','), Some(125_050));
        assert_eq!(amount("AED 25.00", ','), Some(2_500));
        assert_eq!(amount("(25.00)", ','), Some(-2_500));
        assert_eq!(amount("-3", ','), Some(-300));
        assert_eq!(amount("12,50", ';'), Some(1_250));
        assert_eq!(amount("", ','), Some(0));
        assert_eq!(amount("abc", ','), None);
    }

    #[test]
    fn a_file_is_read_by_the_headers_of_the_preset() {
        let file = "Order ID,Gross,Commission,Promo,Refund,Net,Refund reason\n\
                    TB-1001,60.00,-12.00,0,25.00,23.00,Food was cold\n\
                    TB-1002,40.00,8.00,2.00,0,30.00,\n";
        let lines = parse(file, &preset()).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].order_number, "TB-1001");
        assert_eq!(lines[0].commission, 1_200);
        assert_eq!(lines[0].refund, 2_500);
        assert_eq!(lines[0].net, 2_300);
        assert_eq!(lines[0].reason, "Food was cold");
        assert_eq!(lines[1].promo, 200);
    }

    #[test]
    fn a_file_without_the_order_column_is_refused() {
        let error = parse("Id,Gross\n1,2\n", &preset()).unwrap_err();
        assert!(error.contains("Order ID"), "{error}");
    }

    #[test]
    fn the_net_is_worked_out_when_the_file_has_no_net_column() {
        let mut preset = preset();
        preset.net_column = String::new();
        let lines = parse(
            "Order ID,Gross,Commission,Promo,Refund\nX,100,15,5,10\n",
            &preset,
        )
        .unwrap();
        assert_eq!(lines[0].net, 7_000);
    }
}
