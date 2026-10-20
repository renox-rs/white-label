//! Reading a statement file (wl-payments §4 *Bank statement import*, `parse`):
//! a small, documented subset of CSV and of MT940, enough for a UAE bank's
//! daily or monthly statement. Excel files are not read here (they come with
//! the importer of the platform, R23); `parse` fails such an import.
//!
//! **CSV**: a header row naming the columns (any order, case does not
//! matter), then one row per bank transaction, separated by commas, a field
//! may be in double quotes. Required columns `date` (`YYYY-MM-DD`),
//! `description` and `amount` (dirhams, signed: a minus sign is money out);
//! optional `reference` and `counterparty`. A CSV file carries no closing
//! balance: the accountant enters it at upload.
//!
//! **MT940**: the tags `:25:` (the account), `:61:` (a transaction: value
//! date `YYMMDD`, optional entry date `MMDD`, `C`/`D`/`RC`/`RD`, the amount
//! with a comma for the decimals, `N`, the transaction type and the
//! customer reference up to `//`), `:86:` (its description, continued over
//! several lines) and `:62F:`/`:62M:` (the closing balance: `C` or `D`, the
//! date `YYMMDD`, the currency and the amount). Other tags are ignored.
//! The closing balance and its date come from the file.

use renox::chrono::NaiveDate;

/// One transaction of a statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedLine {
    /// The day of the transaction.
    pub date: NaiveDate,
    /// The amount in fils: above 0 is money in, below 0 money out.
    pub amount: i64,
    /// The bank's description.
    pub description: String,
    /// The reference the bank prints.
    pub reference: String,
    /// The other side, when the file names it.
    pub counterparty: String,
}

/// What a statement file holds.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedStatement {
    /// The account as the file names it (MT940 `:25:`).
    pub account: Option<String>,
    /// The transactions, in file order.
    pub lines: Vec<ParsedLine>,
    /// The closing balance in fils and the day it is as of, when the file
    /// carries one.
    pub closing: Option<(NaiveDate, i64)>,
}

/// Reads `content` as `format` (`csv` or `mt940`).
///
/// # Errors
///
/// The reason the file is not recognised, in words for the uploader.
pub fn parse(format: &str, content: &str) -> Result<ParsedStatement, String> {
    match format {
        "csv" => parse_csv(content),
        "mt940" => parse_mt940(content),
        _ => Err("This file format cannot be read yet.".to_owned()),
    }
}

/// Splits a CSV row; a field may be in double quotes (`""` is a quote).
fn split_row(row: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = row.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => fields.push(std::mem::take(&mut field).trim().to_owned()),
            _ => field.push(c),
        }
    }
    fields.push(field.trim().to_owned());
    fields
}

fn parse_csv(content: &str) -> Result<ParsedStatement, String> {
    let mut rows = content.lines().map(str::trim).filter(|row| !row.is_empty());
    let header: Vec<String> = split_row(rows.next().ok_or("The file is empty.")?)
        .into_iter()
        .map(|name| name.to_lowercase())
        .collect();
    let column = |name: &str| header.iter().position(|h| h == name);
    let (Some(date), Some(description), Some(amount)) =
        (column("date"), column("description"), column("amount"))
    else {
        return Err("The header needs the columns date, description and amount.".to_owned());
    };
    let (reference, counterparty) = (column("reference"), column("counterparty"));
    let mut lines = Vec::new();
    for (index, row) in rows.enumerate() {
        let number = index + 2;
        let fields = split_row(row);
        let field = |at: usize| fields.get(at).cloned().unwrap_or_default();
        let day = NaiveDate::parse_from_str(&field(date), "%Y-%m-%d")
            .map_err(|_| format!("Row {number}: the date must be YYYY-MM-DD."))?;
        let fils = crate::bank_statement::parse_balance(&field(amount))
            .filter(|fils| *fils != 0)
            .ok_or_else(|| format!("Row {number}: the amount is not a non-zero number."))?;
        lines.push(ParsedLine {
            date: day,
            amount: fils,
            description: field(description),
            reference: reference.map(field).unwrap_or_default(),
            counterparty: counterparty.map(field).unwrap_or_default(),
        });
    }
    Ok(ParsedStatement {
        account: None,
        lines,
        closing: None,
    })
}

/// `YYMMDD` as a date in 20YY.
fn short_date(text: &str) -> Option<NaiveDate> {
    if text.len() != 6 || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let year: i32 = text[0..2].parse().ok()?;
    NaiveDate::from_ymd_opt(
        2000 + year,
        text[2..4].parse().ok()?,
        text[4..6].parse().ok()?,
    )
}

/// An MT940 amount (`50000,00`, `12,5`) as fils.
fn mt940_amount(text: &str) -> Option<i64> {
    crate::bank_statement::parse_balance(&text.replace(',', "."))
}

/// A `:61:` value: the date, the signed amount and the reference.
fn statement_line(value: &str) -> Result<(NaiveDate, i64, String), String> {
    let bad = || format!("The transaction line `:61:{value}` is not understood.");
    let date = short_date(value.get(0..6).ok_or_else(bad)?).ok_or_else(bad)?;
    let mut rest = &value[6..];
    // The optional entry date MMDD.
    if rest.len() >= 4 && rest.as_bytes()[..4].iter().all(u8::is_ascii_digit) {
        rest = &rest[4..];
    }
    let (sign, rest) = if let Some(r) = rest.strip_prefix("RC") {
        (-1, r)
    } else if let Some(r) = rest.strip_prefix("RD") {
        (1, r)
    } else if let Some(r) = rest.strip_prefix('C') {
        (1, r)
    } else if let Some(r) = rest.strip_prefix('D') {
        (-1, r)
    } else {
        return Err(bad());
    };
    // The optional funds code (a letter before the amount).
    let rest = match rest.chars().next() {
        Some(c) if c.is_ascii_alphabetic() => &rest[1..],
        _ => rest,
    };
    let split = rest
        .find(|c: char| !(c.is_ascii_digit() || c == ','))
        .ok_or_else(bad)?;
    let fils = mt940_amount(&rest[..split])
        .filter(|fils| *fils != 0)
        .ok_or_else(bad)?;
    // `N`, three characters of transaction type, then the reference.
    let tail = rest[split..].get(4..).ok_or_else(bad)?;
    let reference = tail.split("//").next().unwrap_or_default().trim();
    Ok((date, sign * fils, reference.to_owned()))
}

/// A `:62F:` value: the day and the signed balance.
fn closing_balance(value: &str) -> Result<(NaiveDate, i64), String> {
    let bad = || format!("The closing balance `:62F:{value}` is not understood.");
    let sign = match value.chars().next() {
        Some('C') => 1,
        Some('D') => -1,
        _ => return Err(bad()),
    };
    let date = short_date(value.get(1..7).ok_or_else(bad)?).ok_or_else(bad)?;
    // Three letters of currency, then the amount.
    let fils = mt940_amount(value.get(10..).ok_or_else(bad)?).ok_or_else(bad)?;
    Ok((date, sign * fils))
}

fn parse_mt940(content: &str) -> Result<ParsedStatement, String> {
    let mut statement = ParsedStatement::default();
    // The open line of the file: tag and value, continued by plain lines.
    let mut tags: Vec<(String, String)> = Vec::new();
    for row in content.lines().map(str::trim_end) {
        if row.trim().is_empty() || row.trim() == "-" {
            continue;
        }
        if let Some(rest) = row.strip_prefix(':')
            && let Some((tag, value)) = rest.split_once(':')
        {
            tags.push((tag.to_owned(), value.to_owned()));
        } else if let Some((_, value)) = tags.last_mut() {
            value.push('\n');
            value.push_str(row.trim());
        }
    }
    if tags.is_empty() {
        return Err("The file has no MT940 tags.".to_owned());
    }
    for (tag, value) in &tags {
        match tag.as_str() {
            "25" => statement.account = Some(value.trim().to_owned()),
            "61" => {
                let (date, amount, reference) = statement_line(value.lines().next().unwrap_or(""))?;
                statement.lines.push(ParsedLine {
                    date,
                    amount,
                    description: String::new(),
                    reference,
                    counterparty: String::new(),
                });
            }
            "86" => {
                if let Some(line) = statement.lines.last_mut()
                    && line.description.is_empty()
                {
                    line.description = value.replace('\n', " ").trim().to_owned();
                }
            }
            "62F" | "62M" => {
                statement.closing = Some(closing_balance(value.lines().next().unwrap_or(""))?);
            }
            _ => {}
        }
    }
    Ok(statement)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2027, m, d).unwrap()
    }

    #[test]
    fn a_csv_statement_has_signed_amounts() {
        let file = "Date,Description,Reference,Amount\n\
                    2027-01-03,\"ATM, Marina\",ATM-1,-1200.00\n\
                    2027-01-04,Rent Unit 4,TRX-7,5000\n";
        let parsed = parse("csv", file).unwrap();
        assert_eq!(parsed.lines.len(), 2);
        assert_eq!(parsed.lines[0].amount, -120_000);
        assert_eq!(parsed.lines[0].description, "ATM, Marina");
        assert_eq!(parsed.lines[0].date, day(1, 3));
        assert_eq!(parsed.lines[1].amount, 500_000);
        assert_eq!(parsed.lines[1].reference, "TRX-7");
        assert_eq!(parsed.closing, None);
    }

    #[test]
    fn a_csv_file_needs_its_columns_and_valid_rows() {
        assert!(parse("csv", "").is_err());
        assert!(parse("csv", "date,amount\n2027-01-01,5").is_err());
        let bad_date = "date,description,amount\n03/01/2027,x,5";
        assert!(parse("csv", bad_date).unwrap_err().contains("Row 2"));
        let bad_amount = "date,description,amount\n2027-01-03,x,abc";
        assert!(parse("csv", bad_amount).is_err());
        // A header alone is a statement without transactions.
        assert!(
            parse("csv", "date,description,amount")
                .unwrap()
                .lines
                .is_empty()
        );
    }

    const MT940: &str = ":20:STMT1\n:25:ENBD-1\n:28C:1/1\n:60F:C270101AED10000,00\n\
        :61:2701030103D1200,00NMSCATM-1//BANK1\n:86:ATM Marina\nWithdrawal\n\
        :61:270104C5000,00NTRFTRX-7\n:86:Rent Unit 4\n\
        :62F:C270110AED50000,00\n-\n";

    #[test]
    fn an_mt940_statement_gives_its_lines_and_closing_balance() {
        let parsed = parse("mt940", MT940).unwrap();
        assert_eq!(parsed.account.as_deref(), Some("ENBD-1"));
        assert_eq!(parsed.lines.len(), 2);
        assert_eq!(parsed.lines[0].date, day(1, 3));
        assert_eq!(parsed.lines[0].amount, -120_000);
        assert_eq!(parsed.lines[0].reference, "ATM-1");
        assert_eq!(parsed.lines[0].description, "ATM Marina Withdrawal");
        assert_eq!(parsed.lines[1].amount, 500_000);
        assert_eq!(parsed.lines[1].reference, "TRX-7");
        assert_eq!(parsed.closing, Some((day(1, 10), 5_000_000)));
    }

    #[test]
    fn a_debit_closing_balance_is_negative_and_garbage_is_refused() {
        let file = ":25:X\n:62F:D270110AED120,50\n";
        assert_eq!(
            parse("mt940", file).unwrap().closing,
            Some((day(1, 10), -12_050))
        );
        assert!(parse("mt940", "hello").is_err());
        assert!(parse("mt940", ":61:garbage\n").is_err());
        assert!(parse("excel", "x").is_err());
    }
}
