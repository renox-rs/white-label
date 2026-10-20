//! Issuing an invoice (wl-invoicing §4 `issue`).
//!
//! `issue` moves a `draft` to `issued` for a user holding `invoices.issue`.
//! The machine has no app state, so [`issue`] gathers what the guards need
//! into an [`IssueInput`]: the number of lines, whether one is `standard`,
//! the issuing entity's facts, the party's billing identity (through the
//! kernel's [`PartyDirectory`](wl_kernel::PartyDirectory)) and the payment
//! terms. A run without that input (a bare POST) is refused by the guards.
//!
//! The guards: at least one line; the issuing entity is active, and VAT
//! registered with a TRN when a line is `standard`; the party is found; a
//! foreign-currency invoice has its exchange rate. The effect, in the
//! transition's transaction: the number from the kernel's `invoice` series
//! (gapless, per entity, taken only here, so a rolled-back issue gives it
//! back), the issuer and party snapshots, the issue date (today in
//! Asia/Dubai), the due date (the draft's, else the issue date plus the
//! payment terms) and the owner. `InvoiceIssued` is emitted after the commit
//! ([`crate::events`]).

use renox::chrono::Days;
use renox::db::Json;
use renox::prelude::*;
use renox::state_machine::{StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::LegalEntity;
use wl_kernel::documents::today;
use wl_kernel::entities::is_trn;
use wl_kernel::invoicing::{InvoiceStatus, VatCategory, invoice_series};

use crate::invoice::{Invoice, InvoiceKind, PartySnapshot};
use crate::invoicer::lines_of;
use crate::permissions;

/// The transition's name.
pub const ISSUE: &str = "issue";
/// The system twin of [`ISSUE`]: no user, and only for a source the module is
/// set to issue automatically.
pub const ISSUE_SYSTEM: &str = "issue_system";

/// What an issue run carries to its guards and effect, gathered by
/// [`issue`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct IssueInput {
    /// How many lines the invoice has.
    pub lines: usize,
    /// Whether a line is `standard` (carries VAT).
    pub standard_line: bool,
    /// Whether the issuing entity is found and active.
    pub entity_active: bool,
    /// Whether the issuing entity is VAT registered with a valid TRN.
    pub entity_vat_registered: bool,
    /// The party's billing identity, when the party is found.
    pub party: Option<PartySnapshot>,
    /// Days from the issue date to the due date when the draft has none.
    pub payment_terms_days: u32,
    /// What e-invoicing says about this invoice: whether it applies to the
    /// entity today, whether the recipient is in scope and whether the party
    /// has a recipient identifier (`FIN.INV.01`).
    #[serde(default)]
    pub einvoice: crate::einvoice_send::IssueFacts,
    /// Whether the module is set to issue this invoice's source type
    /// automatically (`InvoicingConfig::auto_issue_sources`); only the
    /// system twin checks it.
    #[serde(default)]
    pub auto_issue: bool,
}

/// The `issue` transition and its system twin, for the machine.
pub(crate) fn transitions() -> [Transition<Invoice>; 2] {
    [
        rules(
            Transition::new(ISSUE)
                .label("Issue")
                .tone("primary")
                .permission(permissions::ISSUE),
        ),
        rules(
            Transition::new(ISSUE_SYSTEM)
                .label("Issue (system)")
                .system(),
        )
        .guard("auto_issue", |_invoice: &Invoice, ctx| {
            Box::pin(async move {
                match input(ctx) {
                    Some(input) if input.auto_issue => Ok(()),
                    _ => Err(
                        "The system issues no invoice of this source: a user issues it.".to_owned(),
                    ),
                }
            })
        }),
    ]
}

/// What both transitions share: the draft state, the guards and the effect.
fn rules(transition: Transition<Invoice>) -> Transition<Invoice> {
    transition
        .from([InvoiceStatus::Draft])
        .to(InvoiceStatus::Issued)
        .guard("has_lines", |_invoice: &Invoice, ctx| {
            Box::pin(async move {
                match input(ctx) {
                    Some(input) if input.lines > 0 => Ok(()),
                    _ => Err("The invoice has no lines.".to_owned()),
                }
            })
        })
        .guard("entity_active", |_invoice: &Invoice, ctx| {
            Box::pin(async move {
                match input(ctx) {
                    Some(input) if input.entity_active => Ok(()),
                    _ => Err("The issuing legal entity is not active.".to_owned()),
                }
            })
        })
        .guard("entity_vat_registered", |_invoice: &Invoice, ctx| {
            Box::pin(async move {
                match input(ctx) {
                    Some(input) if !input.standard_line || input.entity_vat_registered => Ok(()),
                    _ => Err(
                        "A standard-rated line needs an issuing entity that is VAT registered \
                         with a TRN."
                            .to_owned(),
                    ),
                }
            })
        })
        .guard("party_found", |_invoice: &Invoice, ctx| {
            Box::pin(async move {
                match input(ctx) {
                    Some(IssueInput { party: Some(_), .. }) => Ok(()),
                    _ => Err("The billed party was not found.".to_owned()),
                }
            })
        })
        .guard("no_simplified_invoice", |invoice: &Invoice, ctx| {
            let applies = input(ctx).is_some_and(|i| i.einvoice.applies);
            let simplified = invoice.kind == InvoiceKind::SimplifiedTaxInvoice;
            Box::pin(async move {
                if applies && simplified {
                    Err(
                        "A simplified tax invoice cannot be issued once e-invoicing applies \
                         to the entity; issue a tax invoice."
                            .to_owned(),
                    )
                } else {
                    Ok(())
                }
            })
        })
        .guard("recipient_identifier", |_invoice: &Invoice, ctx| {
            let missing = input(ctx).is_some_and(|i| {
                i.einvoice.applies && i.einvoice.in_scope && i.einvoice.recipient_id.is_none()
            });
            Box::pin(async move {
                if missing {
                    Err(
                        "The party has no e-invoicing recipient identifier (Peppol ID or TRN): \
                         enter it on the party before the invoice is issued."
                            .to_owned(),
                    )
                } else {
                    Ok(())
                }
            })
        })
        .guard("exchange_rate", |invoice: &Invoice, _ctx| {
            Box::pin(async move {
                if invoice.aed_rate().is_some_and(|rate| rate > 0) {
                    Ok(())
                } else {
                    Err(format!(
                        "An invoice in {} needs its exchange rate to AED.",
                        invoice.currency
                    ))
                }
            })
        })
        .effect(|invoice: &Invoice, ctx, tx| {
            let invoice = invoice.clone();
            Box::pin(async move {
                let input = input(ctx).unwrap_or_default();
                let entity_id = invoice.legal_entity_id.ok_or_else(|| {
                    Error::from(renox::anyhow::anyhow!(
                        "invoice {} has no issuing legal entity",
                        invoice.id
                    ))
                })?;
                let entity = LegalEntity::find_or_404(&mut *tx, entity_id).await?;
                let active = LegalEntity::query()
                    .where_(LegalEntity::STATUS.eq("active"))
                    .count(&mut *tx)
                    .await?;
                let issue_date = today();
                // Inside the issue transaction only: a rollback gives the
                // number back, so the series stays gapless.
                let number = invoice_series(usize::try_from(active).unwrap_or(usize::MAX))
                    .next(&mut *tx, None, Some(&entity), issue_date)
                    .await?;
                let due_date = due_date(invoice.due_date, issue_date, input.payment_terms_days);
                let issuer = Json(entity.snapshot());
                let party = input.party.map(Json);
                let owner = ctx.actor().map(|user| user.id);
                // Not the status column, so outside the machine's guard.
                Invoice::query()
                    .where_(Invoice::ID.eq(invoice.id))
                    .update(
                        &mut *tx,
                        &[
                            ("number", &number),
                            ("issuer_snapshot", &issuer),
                            ("party_snapshot", &party),
                            ("issue_date", &issue_date),
                            ("due_date", &due_date),
                            ("owner_id", &owner),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// The run's input, if it is an [`IssueInput`].
fn input(ctx: &TransitionCtx) -> Option<IssueInput> {
    ctx.input_as::<IssueInput>().ok()
}

/// The due date: the draft's, else `terms` days after the issue date.
fn due_date(
    draft: Option<renox::chrono::NaiveDate>,
    issue_date: renox::chrono::NaiveDate,
    terms: u32,
) -> renox::chrono::NaiveDate {
    draft.unwrap_or_else(|| {
        issue_date
            .checked_add_days(Days::new(u64::from(terms)))
            .unwrap_or(issue_date)
    })
}

/// What the guards and the effect of issuing `invoice` need.
async fn gather(state: &AppState, invoice: &Invoice) -> Result<IssueInput> {
    let lines = lines_of(&state.db, invoice.id).await?;
    let entity = match invoice.legal_entity_id {
        Some(id) => LegalEntity::find(&state.db, id).await?,
        None => None,
    };
    let party = wl_kernel::parties::directory(state)
        .billing_identity(&state.db, invoice.party())
        .await?;
    Ok(IssueInput {
        lines: lines.len(),
        standard_line: lines
            .iter()
            .any(|line| line.vat_category == VatCategory::Standard),
        entity_active: entity.as_ref().is_some_and(LegalEntity::is_active),
        entity_vat_registered: entity.as_ref().is_some_and(|e| {
            e.vat_registered && e.trn.as_deref().is_some_and(|trn| is_trn(trn.trim()))
        }),
        party: party.as_ref().map(PartySnapshot::of),
        payment_terms_days: crate::config_of(state).payment_terms_days,
        einvoice: crate::einvoice_send::issue_facts(state, invoice).await?,
        auto_issue: crate::config_of(state)
            .auto_issue_sources
            .contains(&invoice.source_type),
    })
}

/// The system issues the draft `invoice` (`issue_system`), in its own
/// transaction; refused unless the module is set to issue its source type.
///
/// # Errors
///
/// `Guard` (422) for a source type that is not issued automatically, and the
/// guards of [`issue`].
pub async fn issue_by_system(
    state: &AppState,
    invoice: &Invoice,
) -> std::result::Result<(), TransitionError> {
    let input = gather(state, invoice).await?;
    let _outcome = invoice
        .transition(ISSUE_SYSTEM)
        .by_system()
        .input(&input)
        .run(state)
        .await?;
    Ok(())
}

/// `user` issues the draft `invoice` (`issue`), in its own transaction.
///
/// # Errors
///
/// `Forbidden` without `invoices.issue`, `NotFrom` when it is not a draft,
/// `Guard` (422) with no lines, an inactive entity, a `standard` line from an
/// entity that is not VAT registered, an unknown party or a missing
/// exchange rate; nothing is written then and no number is used.
pub async fn issue(
    state: &AppState,
    user: &AuthUser,
    invoice: &Invoice,
) -> std::result::Result<(), TransitionError> {
    let input = gather(state, invoice).await?;
    // `run` emits a done outcome after the commit; `InvoiceIssued` follows
    // from it.
    let _outcome = invoice
        .transition(ISSUE)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::chrono::NaiveDate;

    fn ready() -> IssueInput {
        IssueInput {
            lines: 2,
            standard_line: true,
            entity_active: true,
            entity_vat_registered: true,
            party: Some(PartySnapshot {
                legal_name_en: "Blue Dunes LLC".into(),
                legal_name_ar: None,
                trn: None,
                billing_address: None,
                billing_email: Some("ap@bluedunes.ae".into()),
                taken_at: renox::db::now(),
            }),
            payment_terms_days: 30,
            einvoice: Default::default(),
            auto_issue: false,
        }
    }

    fn run(input: &IssueInput) -> TransitionCtx {
        TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).expect("json"))
    }

    fn aed() -> Invoice {
        Invoice {
            currency: "AED".into(),
            ..Default::default()
        }
    }

    #[test]
    fn issue_goes_from_draft_to_issued() {
        let machine = Invoice::machine();
        let issue = machine.find(ISSUE).expect("transition");
        assert_eq!(issue.from_states(), ["draft"]);
        assert_eq!(issue.to_state(), Some("issued"));
        assert!(!issue.is_system());
        assert_eq!(
            issue.guard_names(),
            [
                "has_lines",
                "entity_active",
                "entity_vat_registered",
                "party_found",
                "no_simplified_invoice",
                "recipient_identifier",
                "exchange_rate"
            ]
        );
    }

    #[renox::test]
    async fn the_guards_pass_a_ready_draft() {
        let machine = Invoice::machine();
        let issue = machine.find(ISSUE).expect("transition");
        assert!(issue.check_guards(&aed(), &run(&ready())).await.is_ok());
        let exempt_only = IssueInput {
            standard_line: false,
            entity_vat_registered: false,
            ..ready()
        };
        assert!(issue.check_guards(&aed(), &run(&exempt_only)).await.is_ok());
    }

    #[renox::test]
    async fn the_guards_refuse_what_cannot_be_issued() {
        let machine = Invoice::machine();
        let issue = machine.find(ISSUE).expect("transition");
        let refused = |input: IssueInput| {
            let ctx = run(&input);
            async move {
                matches!(
                    issue.check_guards(&aed(), &ctx).await,
                    Err(TransitionError::Guard(_))
                )
            }
        };
        assert!(
            refused(IssueInput {
                lines: 0,
                ..ready()
            })
            .await
        );
        assert!(
            refused(IssueInput {
                entity_active: false,
                ..ready()
            })
            .await
        );
        assert!(
            refused(IssueInput {
                entity_vat_registered: false,
                ..ready()
            })
            .await
        );
        assert!(
            refused(IssueInput {
                party: None,
                ..ready()
            })
            .await
        );
        // A bare run, without the gathered input.
        assert!(matches!(
            issue.check_guards(&aed(), &TransitionCtx::new(None)).await,
            Err(TransitionError::Guard(_))
        ));
        let usd = Invoice {
            currency: "USD".into(),
            ..Default::default()
        };
        assert!(matches!(
            issue.check_guards(&usd, &run(&ready())).await,
            Err(TransitionError::Guard(_))
        ));
        let usd_with_rate = Invoice {
            exchange_rate_micro: Some(3_672_500),
            ..usd
        };
        assert!(
            issue
                .check_guards(&usd_with_rate, &run(&ready()))
                .await
                .is_ok()
        );
    }

    #[renox::test]
    async fn e_invoicing_refuses_a_simplified_invoice_and_a_recipient_without_an_identifier() {
        use crate::einvoice_send::IssueFacts;
        let machine = Invoice::machine();
        let issue = machine.find(ISSUE).expect("transition");
        let applies = |in_scope: bool, recipient_id: Option<&str>| IssueInput {
            einvoice: IssueFacts {
                applies: true,
                in_scope,
                recipient_id: recipient_id.map(str::to_owned),
            },
            ..ready()
        };
        let simplified = Invoice {
            kind: InvoiceKind::SimplifiedTaxInvoice,
            ..aed()
        };
        // Before go-live both are fine.
        assert!(
            issue
                .check_guards(&simplified, &run(&ready()))
                .await
                .is_ok()
        );
        // After it a simplified invoice is refused, a tax invoice is not.
        assert!(matches!(
            issue
                .check_guards(&simplified, &run(&applies(false, None)))
                .await,
            Err(TransitionError::Guard(_))
        ));
        assert!(
            issue
                .check_guards(&aed(), &run(&applies(false, None)))
                .await
                .is_ok()
        );
        // A recipient in scope needs an identifier; a B2C one does not.
        assert!(matches!(
            issue.check_guards(&aed(), &run(&applies(true, None))).await,
            Err(TransitionError::Guard(_))
        ));
        assert!(
            issue
                .check_guards(&aed(), &run(&applies(true, Some("0235:100000000000003"))))
                .await
                .is_ok()
        );
    }

    #[test]
    fn the_due_date_is_the_drafts_or_the_terms_after_the_issue_date() {
        let issued = NaiveDate::from_ymd_opt(2026, 10, 18).unwrap();
        assert_eq!(
            due_date(None, issued, 30),
            NaiveDate::from_ymd_opt(2026, 11, 17).unwrap()
        );
        let given = NaiveDate::from_ymd_opt(2026, 12, 1).unwrap();
        assert_eq!(due_date(Some(given), issued, 30), given);
        assert_eq!(due_date(None, issued, 0), issued);
    }
}
