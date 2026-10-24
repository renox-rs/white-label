//! The party's deposit in the portal (wl-payments §4 *Deposit*, §5.4;
//! BL-PAY-27): the deposits held for the party with their amount and status,
//! the latest published deductions with their evidence and the last day of
//! the dispute window, the refund status (*In review* while the settlement is
//! pending, *Paid* once refunded) and, while the window is open, *Dispute*
//! on the lines not disputed yet.
//!
//! The deductions are read and disputed through the kernel's
//! [`DepositDeductions`](wl_kernel::deposits::DepositDeductions)
//! (`published_for`, `dispute`), as any portal would; unpublished drafts,
//! internal notes and another party's deposit are never shown. The routes
//! are mounted by the host, which checks the portal grant first: a `read_only`
//! grant keeps this one write action during the window (BL-POR-03), and a
//! blocked party may dispute (it moves no money) with no block text.

use std::collections::{BTreeMap, HashMap};

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::Serialize;
use wl_kernel::PartyId;
use wl_kernel::deposits::{
    DeductionKind, DisputeDeductions, PublishedDeductions, deposit_deductions,
};
use wl_kernel::payments::DepositId;

use crate::deposit::{Deposit, DepositStatus};
use crate::followup::money_text;
use crate::texts;

/// The key of the portal section.
pub const SECTION: &str = "deposit";

/// The portal permission that shows the section (the lease's).
pub const VIEW_PERMISSION: &str = "portal.leases.view";

/// The package text `key` (without the `wl_payments.` prefix).
fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_payments.{key}"))
}

/// `date` as `16 Nov 2026`.
pub fn date_text(date: NaiveDate) -> String {
    date.format("%-d %b %Y").to_string()
}

/// The refund status in words: *In review* while the settlement is pending,
/// *Paid* once refunded; the deposit's own status otherwise.
pub fn refund_status(lang: &Lang, status: DepositStatus) -> String {
    match status {
        DepositStatus::SettlementPending => text(lang, "portal_deposit.in_review"),
        DepositStatus::Refunded => text(lang, "portal_deposit.paid"),
        other => text(lang, &format!("portal_deposit.status.{}", other.as_str())),
    }
}

/// A deduction line as the portal shows it.
#[derive(Debug, Clone, Serialize)]
pub struct PortalLine {
    /// The checkbox name, `line_<id>`.
    pub name: String,
    /// What it is for.
    pub description: String,
    /// The amount.
    pub amount: String,
    /// The evidence reference (inspection photos, a quote).
    pub evidence: String,
    /// Disputed and not answered yet: shown as *Disputed*, not selectable.
    pub disputed: bool,
}

/// A deposit as the portal shows it.
#[derive(Debug, Clone, Serialize)]
pub struct PortalDeposit {
    /// The deposit id.
    pub id: i64,
    /// The amount held.
    pub amount: String,
    /// The refund status in words.
    pub refund_status: String,
    /// Whether deductions are published.
    pub published: bool,
    /// The published version.
    pub version: i32,
    /// `Version 2, published 2 Nov 2026`.
    pub published_text: String,
    /// The forfeiture reason, if it is one.
    pub forfeiture: Option<String>,
    /// `until 16 Nov 2026`, when a dispute is allowed.
    pub window_text: Option<String>,
    /// The lines.
    pub lines: Vec<PortalLine>,
    /// What the lines deduct in all.
    pub total: String,
    /// Whether *Dispute* shows: the window is open and a line is free.
    pub can_dispute: bool,
}

/// The deposits of `party` with their latest published deductions.
pub async fn deposits_of(
    state: &AppState,
    lang: &Lang,
    party: PartyId,
) -> Result<Vec<PortalDeposit>> {
    let published: Vec<PublishedDeductions> =
        deposit_deductions(state).published_for(state, party).await?;
    let mut out = Vec::new();
    for deposit in Deposit::of_party(&state.db, party).await? {
        let shown = published.iter().find(|p| p.deposit.0 == deposit.id);
        let mut row = PortalDeposit {
            id: deposit.id,
            amount: money_text(
                &deposit.currency,
                if deposit.held_amount > 0 {
                    deposit.held_amount
                } else {
                    deposit.amount
                },
            ),
            refund_status: refund_status(lang, deposit.status),
            published: shown.is_some(),
            version: 0,
            published_text: String::new(),
            forfeiture: None,
            window_text: None,
            lines: Vec::new(),
            total: String::new(),
            can_dispute: false,
        };
        if let Some(p) = shown {
            row.version = p.version;
            row.published_text = text(lang, "portal_deposit.published")
                .replace(":version", &p.version.to_string())
                .replace(":date", &date_text(p.published_on));
            row.forfeiture = (p.kind == DeductionKind::Forfeiture)
                .then(|| p.reason.clone().unwrap_or_default());
            row.window_text = p.window_ends_on.map(|end| {
                text(lang, "portal_deposit.window").replace(":date", &date_text(end))
            });
            row.lines = p
                .lines
                .iter()
                .map(|line| PortalLine {
                    name: format!("line_{}", line.id),
                    description: line.description.clone(),
                    amount: money_text(&p.currency, line.amount),
                    evidence: line.evidence.clone(),
                    disputed: p.disputed_lines.contains(&line.id),
                })
                .collect();
            row.total = money_text(&p.currency, p.lines.iter().map(|l| l.amount).sum());
            row.can_dispute = p.can_dispute && row.lines.iter().any(|l| !l.disputed);
        }
        out.push(row);
    }
    Ok(out)
}

/// What the host tells the page about the viewer.
#[derive(Debug, Clone, Default)]
pub struct PortalContext {
    /// The portal home of the party, `/portal/12`.
    pub home_url: String,
    /// The notice of a `read_only` grant, if any.
    pub read_only_notice: Option<String>,
}

/// What the deposit page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-payments/portal_deposit.html")]
pub struct PortalDepositPage {
    tx: BTreeMap<&'static str, String>,
    deposit_rows: Vec<PortalDeposit>,
    home_url: String,
    dispute_base: String,
    read_only_notice: Option<String>,
}

/// The *Deposit* page of `party`.
pub async fn deposit_page(
    state: &AppState,
    lang: &Lang,
    party: PartyId,
    context: PortalContext,
) -> Result<View> {
    let deposit_rows = deposits_of(state, lang, party).await?;
    Ok(View::typed(PortalDepositPage {
        tx: texts::all(lang),
        deposit_rows,
        dispute_base: format!("{}/{SECTION}", context.home_url),
        home_url: context.home_url,
        read_only_notice: context.read_only_notice,
    }))
}

/// The party disputes lines of deposit `id` (`version`, `comment`,
/// `line_<id>` for each ticked line) through the kernel contract. 404 for
/// another party's deposit; 422 with the refusal under `comment` otherwise.
pub async fn dispute(
    state: &AppState,
    party: PartyId,
    id: i64,
    form: &HashMap<String, String>,
) -> Result<()> {
    let mut lines: Vec<i64> = form
        .iter()
        .filter(|(_, value)| !value.is_empty() && *value != "0" && *value != "false")
        .filter_map(|(key, _)| key.strip_prefix("line_")?.parse().ok())
        .collect();
    lines.sort_unstable();
    let request = DisputeDeductions {
        deposit: DepositId(id),
        version: form
            .get("version")
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or_default(),
        lines,
        comment: form.get("comment").cloned().unwrap_or_default(),
    };
    match deposit_deductions(state).dispute(state, party, request).await {
        Ok(()) => Ok(()),
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message)) => {
            let mut errors = renox::validation::Errors::new();
            errors.add("comment", message);
            Err(Error::Validation(
                renox::validation::ValidationError::new(errors).with_input(form),
            ))
        }
        Err(other) => Err(other),
    }
}
