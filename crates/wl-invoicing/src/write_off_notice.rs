//! The customer write-off notice (wl-invoicing §10, `⚠ open: accountant
//! (A-09)` for its content). When an invoice is written off and the setting
//! [`InvoicingConfig::write_off_notice`] is on, a letter goes to the party's
//! billing e-mail (from the invoice's party snapshot) with the invoice
//! number, issue date, date of supply (the issue date when none), gross
//! total, amount written off, its VAT share, the write-off date and the
//! statement that the amount was written off as a bad debt. Its date is
//! evidence for VAT bad-debt relief (`FIN.RUL.14`).
//!
//! The mail is made from the template [`TEMPLATE`] and the package texts.
//! The queued job [`SendWriteOffNotice`] sends it (retried by the queue) and
//! then stores [`Invoice::write_off_notice_on`] = the Asia/Dubai date it was
//! sent. With the setting off, or without a billing e-mail, nothing is sent
//! and the notice date stays empty ([`NoNotice`] says why).

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::{Value, json};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::dubai_date;
use wl_kernel::invoicing::InvoiceStatus;
use wl_kernel::notify::fill;

use crate::allocate::amount_text;
use crate::config::InvoicingConfig;
use crate::config_of;
use crate::invoice::Invoice;
use crate::invoicer::totals_of;
use crate::texts::t;
use crate::vat::bad_debt_vat_share;
use crate::vat_adjustment::VatAdjustment;

/// The notice's mail template; an app replaces it with its own file of
/// that name.
pub const TEMPLATE: &str = "wl-invoicing/write_off_notice.html";

/// [`TEMPLATE`] as `mail_view` names it (without `.html`).
const VIEW: &str = "wl-invoicing/write_off_notice";

/// The currency of VAT amounts.
const AED: &str = "AED";

/// Why a written-off invoice gets no notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoNotice {
    /// The client turned the notice off.
    Off,
    /// The party snapshot has no billing e-mail.
    NoBillingEmail,
}

impl NoNotice {
    /// The key of its explanation in the texts, for the invoice page.
    pub fn text_key(self) -> &'static str {
        match self {
            NoNotice::Off => "wl_invoicing.write_off_notice.off",
            NoNotice::NoBillingEmail => "wl_invoicing.write_off_notice.no_email",
        }
    }
}

/// Where the notice for `invoice` goes, or why it gets none.
pub fn recipient(config: &InvoicingConfig, invoice: &Invoice) -> Result<String, NoNotice> {
    if !config.write_off_notice {
        return Err(NoNotice::Off);
    }
    invoice
        .party_snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.0.billing_email.as_deref())
        .map(str::trim)
        .filter(|email| !email.is_empty())
        .map(str::to_owned)
        .ok_or(NoNotice::NoBillingEmail)
}

/// `date` as `18 Oct 2026`.
fn date_text(date: Option<NaiveDate>) -> String {
    date.map(|d| d.format("%-d %b %Y").to_string())
        .unwrap_or_default()
}

/// The filled texts of a notice: what [`TEMPLATE`] shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NoticeContent {
    /// The mail subject.
    pub subject: String,
    /// The salutation.
    pub greeting: String,
    /// The first paragraph.
    pub intro: String,
    /// The figures, as `[label, value]` pairs.
    pub rows: Vec<[String; 2]>,
    /// The bad-debt statement.
    pub statement: String,
    /// The last paragraph.
    pub closing: String,
}

impl NoticeContent {
    /// The notice for `invoice`, whose written-off amount carries
    /// `vat_share` AED fils of VAT, with the texts of `text` (a key to its
    /// text).
    pub fn build(text: impl Fn(&str) -> String, invoice: &Invoice, vat_share: i64) -> Self {
        let data = Self::data(invoice, vat_share);
        let filled = |key: &str| {
            fill(
                &text(&format!("wl_invoicing.write_off_notice.{key}")),
                &data,
            )
        };
        let value = |key: &str| data[key].as_str().unwrap_or_default().to_owned();
        let row = |key: &str| [filled(key), value(key)];
        NoticeContent {
            subject: filled("subject"),
            greeting: filled("greeting"),
            intro: filled("intro"),
            rows: vec![
                row("number"),
                row("issue_date"),
                row("date_of_supply"),
                row("gross_total"),
                row("written_off"),
                row("vat_share"),
                row("write_off_date"),
            ],
            statement: filled("statement"),
            closing: filled("closing"),
        }
    }

    /// The notice for `invoice` in `lang` (the app's translation, else
    /// English).
    pub fn of(lang: &Lang, invoice: &Invoice, vat_share: i64) -> Self {
        Self::build(|key| t(lang, key), invoice, vat_share)
    }

    /// The values the texts are filled with.
    fn data(invoice: &Invoice, vat_share: i64) -> Value {
        let party_name = invoice
            .party_snapshot
            .as_ref()
            .map(|snapshot| snapshot.0.legal_name_en.clone())
            .unwrap_or_default();
        json!({
            "number": invoice.number.clone().unwrap_or_default(),
            "party_name": party_name,
            "issue_date": date_text(invoice.issue_date),
            "date_of_supply": date_text(invoice.date_of_supply.or(invoice.issue_date)),
            "gross_total": amount_text(&invoice.currency, invoice.gross_total),
            "written_off": amount_text(&invoice.currency, invoice.write_off_amount.unwrap_or_default()),
            "vat_share": amount_text(AED, vat_share),
            "write_off_date": date_text(invoice.write_off_date),
        })
    }
}

/// The VAT share of `invoice`'s write-off, in AED fils: the amount of its
/// bad-debt relief, else computed from its lines (§5.9 step 3).
async fn vat_share_of(db: &Db, invoice: &Invoice) -> Result<i64> {
    if let Some(relief) = VatAdjustment::relief_of(db, invoice).await? {
        return Ok(relief.amount);
    }
    let totals = totals_of(db, invoice.id).await?;
    Ok(bad_debt_vat_share(invoice.write_off_amount.unwrap_or_default(), &totals).unwrap_or(0))
}

/// Sends the customer write-off notice of an invoice and stores its date.
/// The write-off listener queues it after the commit ([`queue`]).
///
/// It does nothing for an invoice that is not `written_off`, that already
/// has a notice date, or that gets no notice ([`recipient`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendWriteOffNotice {
    /// The written-off invoice.
    pub invoice_id: i64,
}

impl Job for SendWriteOffNotice {
    const NAME: &'static str = "wl-invoicing.send-write-off-notice";
    const MAX_ATTEMPTS: u32 = 5;

    async fn handle(self, ctx: JobContext) -> Result {
        let state = &ctx.state;
        let Some(invoice) = Invoice::find(&state.db, self.invoice_id).await? else {
            return Ok(());
        };
        if invoice.status != InvoiceStatus::WrittenOff || invoice.write_off_notice_on.is_some() {
            return Ok(());
        }
        let Ok(to) = recipient(&config_of(state), &invoice) else {
            return Ok(());
        };
        let vat_share = vat_share_of(&state.db, &invoice).await?;
        let content = NoticeContent::of(&state.current_lang(), &invoice, vat_share);
        let mail = state.mail_view(to, content.subject.clone(), VIEW, &content)?;
        state.mailer.send(mail).await?;
        let sent_on = dubai_date(renox::db::now());
        // Not the status column, so outside the machine's guard; only the
        // first send's date is kept.
        renox::db::sql(format!(
            "UPDATE {} SET write_off_notice_on = ?, updated_at = ? \
             WHERE id = ? AND write_off_notice_on IS NULL",
            Invoice::TABLE
        ))
        .bind(sent_on)
        .bind(renox::db::now())
        .bind(invoice.id)
        .execute(&state.db)
        .await?;
        Ok(())
    }
}

/// Queues the notice of `invoice` when it gets one; `false` when it gets
/// none ([`recipient`]).
pub async fn queue(state: &AppState, invoice: &Invoice) -> Result<bool> {
    if recipient(&config_of(state), invoice).is_err() {
        return Ok(false);
    }
    state
        .dispatch(SendWriteOffNotice {
            invoice_id: invoice.id,
        })
        .await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::invoice::PartySnapshot;
    use crate::texts::english;
    use renox::db::Json;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).expect("date")
    }

    fn written_off(email: Option<&str>) -> Invoice {
        Invoice {
            id: 7,
            number: Some("INV-PGP-2026-0001".into()),
            currency: "AED".into(),
            issue_date: Some(day(1)),
            gross_total: 105_000,
            write_off_amount: Some(105_000),
            write_off_date: Some(day(18)),
            status: InvoiceStatus::WrittenOff,
            party_snapshot: Some(Json(PartySnapshot {
                legal_name_en: "Blue Dunes LLC".into(),
                legal_name_ar: None,
                trn: None,
                billing_address: None,
                billing_email: email.map(str::to_owned),
                taken_at: renox::db::now(),
            })),
            ..Default::default()
        }
    }

    #[test]
    fn the_notice_goes_to_the_billing_email_when_on() {
        let config = InvoicingConfig::default();
        assert_eq!(
            recipient(&config, &written_off(Some(" ap@bluedunes.ae "))).unwrap(),
            "ap@bluedunes.ae"
        );
        assert_eq!(
            recipient(&config, &written_off(None)).unwrap_err(),
            NoNotice::NoBillingEmail
        );
        assert_eq!(
            recipient(&config, &written_off(Some(""))).unwrap_err(),
            NoNotice::NoBillingEmail
        );
        let off = InvoicingConfig {
            write_off_notice: false,
            ..InvoicingConfig::default()
        };
        assert_eq!(
            recipient(&off, &written_off(Some("ap@bluedunes.ae"))).unwrap_err(),
            NoNotice::Off
        );
        assert_ne!(english(NoNotice::Off.text_key()), NoNotice::Off.text_key());
        assert_ne!(
            english(NoNotice::NoBillingEmail.text_key()),
            NoNotice::NoBillingEmail.text_key()
        );
    }

    #[test]
    fn the_notice_carries_the_invoice_figures() {
        let invoice = written_off(Some("ap@bluedunes.ae"));
        let content = NoticeContent::build(|key| english(key).to_owned(), &invoice, 5_000);
        assert_eq!(
            content.subject,
            "Invoice INV-PGP-2026-0001 written off as a bad debt"
        );
        assert_eq!(content.greeting, "Dear Blue Dunes LLC,");
        assert!(content.intro.contains("issued to you on 1 Oct 2026"));
        let rows: Vec<(&str, &str)> = content
            .rows
            .iter()
            .map(|[label, value]| (label.as_str(), value.as_str()))
            .collect();
        assert_eq!(
            rows,
            [
                ("Invoice number", "INV-PGP-2026-0001"),
                ("Issue date", "1 Oct 2026"),
                // No date of supply: the issue date.
                ("Date of supply", "1 Oct 2026"),
                ("Total including VAT", "AED 1,050.00"),
                ("Amount written off", "AED 1,050.00"),
                ("VAT in the amount written off", "AED 50.00"),
                ("Write-off date", "18 Oct 2026"),
            ]
        );
        assert!(content.statement.contains(
            "AED 1,050.00 still open on this invoice was written off as a bad debt on 18 Oct 2026"
        ));
    }

    #[test]
    fn a_recorded_date_of_supply_is_shown() {
        let invoice = Invoice {
            date_of_supply: Some(day(3)),
            ..written_off(None)
        };
        let content = NoticeContent::build(|key| english(key).to_owned(), &invoice, 0);
        assert_eq!(content.rows[2][1], "3 Oct 2026");
    }

    #[test]
    fn the_template_is_a_mail_with_the_rows() {
        let source = include_str!("../views/wl-invoicing/write_off_notice.html");
        assert!(source.contains("<rx-mail subject=\"{{ subject }}\">"));
        assert!(source.contains(":rows=\"rows\""));
    }
}
