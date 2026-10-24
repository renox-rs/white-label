//! The base of the 08 Auto supplier warranty claim (base for #407;
//! `AUT.WAR.02`, 08 §4.6 step 4, §7.1 *Supplier warranty claim*): a claim is
//! raised from a warranty ticket closed as accepted, judged against the
//! supplier's terms (the period, the mileage of a part that was fitted), kept
//! with what the supplier says, and ended when the supplier credits or
//! replaces the part or the claim is written off. The guard slot the issue
//! fills passes for now.

mod support;

use auto::supplier_claim_machine::{RECOVER, RECOVER_SYSTEM, WRITE_OFF};
use auto::supplier_claims::{self, NewSupplierClaim, SUPPLIER_RETURN_SUBJECT_TYPE};
use auto::{JobCardStatus, SupplierAnswer, SupplierClaim, SupplierClaimStatus};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use support::{Garage, boot, day};
use wl_kernel::inventory::SupplierReturnCredited;
use wl_kernel::{PartyId, Subject};

/// The purchaser Pia: raises and recovers supplier claims.
async fn pia(t: &Garage) -> AuthUser {
    let user = t
        .user(
            "Pia",
            &[
                auto::permissions::SUPPLIER_CLAIMS_MANAGE,
                auto::permissions::JOB_CARDS_VIEW,
            ],
        )
        .await;
    t.auth(&user).await
}

/// The job card the warranty ticket is about, and the warranty ticket
/// `TKT-2026-0012` (id 12) closed on it.
async fn closed_ticket(t: &Garage) -> Subject {
    t.people.add(40, "Gulf Auto Spares");
    let vehicle = t.hilux().await;
    let card = t.card_in(&vehicle, JobCardStatus::Delivered).await;
    let about = card.subject();
    t.tickets
        .add(&about, "warranty", 12, "TKT-2026-0012", "closed");
    about
}

/// An alternator bought on 2026-05-01 (12 months, AED 450) that failed on
/// 2026-11-02.
fn alternator(about: &Subject) -> NewSupplierClaim {
    NewSupplierClaim::new(
        12,
        about.clone(),
        40,
        "Alternator",
        45_000,
        renox::chrono::NaiveDate::from_ymd_opt(2026, 5, 1).unwrap(),
        12,
        renox::chrono::NaiveDate::from_ymd_opt(2026, 10, 20).unwrap(),
    )
}

/// A claim is raised from the warranty ticket that was closed as accepted: the
/// supplier, the part, what is claimed and the terms are kept, the supplier has
/// not answered, and the history names how it began.
#[renox::test]
async fn aut_base_a_claim_is_raised_from_a_closed_accepted_warranty_ticket() {
    let t = boot().await;
    let about = closed_ticket(&t).await;
    let pia = pia(&t).await;

    let claim =
        supplier_claims::raise(t.state(), &pia, alternator(&about).item(" alt-9 ", "Bosch"))
            .await
            .expect("the claim is raised");
    assert_eq!(claim.status, SupplierClaimStatus::Raised);
    assert_eq!(claim.number(), format!("SWC-{:04}", claim.id));
    assert_eq!(
        (claim.ticket_id, claim.ticket_number.as_str()),
        (12, "TKT-2026-0012")
    );
    assert_eq!(claim.supplier_party_id, 40);
    assert_eq!(
        claim.about().map(|s| s.to_string()),
        Some(about.to_string())
    );
    assert_eq!((claim.part_cost, claim.claim_amount), (45_000, 45_000));
    assert_eq!(claim.item_code.as_deref(), Some("ALT-9"));
    assert_eq!(claim.supplier_answer, SupplierAnswer::Pending);
    assert!(claim.raised_at.is_some());
    assert!(claim.part_kept);
    let history = renox::state_machine::history(t.db(), SupplierClaim::TABLE, claim.id)
        .await
        .unwrap();
    assert_eq!(history.len(), 1, "the raise is the first change");
}

/// Only a warranty ticket of that record that is closed and was decided as
/// accepted is claimed, once; only the purchaser raises.
#[renox::test]
async fn aut_base_a_claim_needs_a_closed_accepted_ticket_once() {
    let t = boot().await;
    let about = closed_ticket(&t).await;
    let pia = pia(&t).await;
    t.tickets
        .add(&about, "warranty", 13, "TKT-2026-0013", "resolved");

    let other = supplier_claims::raise(
        t.state(),
        &pia,
        NewSupplierClaim::new(
            99,
            about.clone(),
            40,
            "Alternator",
            45_000,
            day(-170),
            12,
            day(-10),
        ),
    )
    .await
    .unwrap_err();
    assert!(
        other.to_string().contains("not a warranty ticket"),
        "{other}"
    );
    let open = supplier_claims::raise(
        t.state(),
        &pia,
        NewSupplierClaim::new(
            13,
            about.clone(),
            40,
            "Alternator",
            45_000,
            day(-170),
            12,
            day(-10),
        ),
    )
    .await
    .unwrap_err();
    assert!(open.to_string().contains("not closed"), "{open}");
    let rejected = supplier_claims::raise(
        t.state(),
        &pia,
        NewSupplierClaim::new(
            12,
            about.clone(),
            40,
            "Alternator",
            45_000,
            day(-170),
            12,
            day(-10),
        )
        .rejected_decision(),
    )
    .await
    .unwrap_err();
    assert!(
        rejected.to_string().contains("closed as accepted"),
        "{rejected}"
    );

    let base = || {
        NewSupplierClaim::new(
            12,
            about.clone(),
            40,
            "Alternator",
            45_000,
            day(-170),
            12,
            day(-10),
        )
    };
    supplier_claims::raise(t.state(), &pia, base())
        .await
        .unwrap();
    let twice = supplier_claims::raise(t.state(), &pia, base())
        .await
        .unwrap_err();
    assert!(
        twice.to_string().contains("raised for that ticket already"),
        "{twice}"
    );

    let stranger = Subject::new("auto.job_card", 9_999);
    let elsewhere = supplier_claims::raise(
        t.state(),
        &pia,
        NewSupplierClaim::new(
            12,
            stranger,
            40,
            "Alternator",
            45_000,
            day(-170),
            12,
            day(-10),
        ),
    )
    .await
    .unwrap_err();
    assert!(
        elsewhere.to_string().contains("not a warranty ticket"),
        "{elsewhere}"
    );
    let sami = t.advisor().await;
    assert!(matches!(
        supplier_claims::raise(t.state(), &sami, base()).await,
        Err(Error::Forbidden)
    ));
}

/// The fields are checked: a part, amounts above zero, a supplier that is a
/// party, dates that make sense.
#[renox::test]
async fn aut_base_a_claim_is_refused_for_what_does_not_make_sense() {
    let t = boot().await;
    let about = closed_ticket(&t).await;
    let pia = pia(&t).await;
    let base = || {
        NewSupplierClaim::new(
            12,
            about.clone(),
            40,
            "Alternator",
            45_000,
            day(-170),
            12,
            day(-10),
        )
    };

    let mut blank = base();
    blank.part_description = "  ".to_owned();
    assert!(
        supplier_claims::raise(t.state(), &pia, blank)
            .await
            .is_err()
    );
    let mut free = base();
    free.part_cost = 0;
    assert!(supplier_claims::raise(t.state(), &pia, free).await.is_err());
    let mut no_warranty = base();
    no_warranty.warranty_months = 0;
    assert!(
        supplier_claims::raise(t.state(), &pia, no_warranty)
            .await
            .is_err()
    );
    let mut future = base();
    future.failure_date = day(3);
    assert!(
        supplier_claims::raise(t.state(), &pia, future)
            .await
            .is_err()
    );
    let mut before = base();
    before.failure_date = day(-200);
    assert!(
        supplier_claims::raise(t.state(), &pia, before)
            .await
            .is_err()
    );
    let nobody = NewSupplierClaim::new(
        12,
        about.clone(),
        999,
        "Alternator",
        45_000,
        day(-170),
        12,
        day(-10),
    );
    let unknown = supplier_claims::raise(t.state(), &pia, nobody)
        .await
        .unwrap_err();
    assert!(unknown.to_string().contains("supplier"), "{unknown}");
    assert!(
        SupplierClaim::of_ticket(t.db(), 12)
            .await
            .unwrap()
            .is_none()
    );
}

/// The supplier's terms: a failure after the period is refused with the term
/// that failed; a part fitted on a job card is held to the mileage too (12
/// months / 20,000 km, fitted at 40,000 km: 65,000 km is out, 58,000 km is
/// in); a part sold over the counter is checked on the period only.
#[renox::test]
async fn aut_base_the_supplier_warranty_applies_on_the_period_and_the_mileage() {
    let t = boot().await;
    let about = closed_ticket(&t).await;
    let pia = pia(&t).await;
    let vehicle = t.hilux().await;
    let fitted_on = t.card_in(&vehicle, JobCardStatus::Delivered).await;

    let late = supplier_claims::raise(
        t.state(),
        &pia,
        NewSupplierClaim::new(
            12,
            about.clone(),
            40,
            "Alternator",
            45_000,
            day(-400),
            12,
            day(-10),
        ),
    )
    .await
    .unwrap_err();
    assert!(
        late.to_string().contains("after the warranty period"),
        "{late}"
    );

    let base = || {
        NewSupplierClaim::new(
            12,
            about.clone(),
            40,
            "Alternator",
            45_000,
            day(-170),
            12,
            day(-10),
        )
        .mileage(20_000)
    };
    let too_far = supplier_claims::raise(
        t.state(),
        &pia,
        base().fitted_on(fitted_on.id, 40_000, 65_000),
    )
    .await
    .unwrap_err();
    assert!(
        too_far.to_string().contains("mileage out of terms"),
        "{too_far}"
    );
    let unknown_km = supplier_claims::raise(t.state(), &pia, {
        let mut claim = base();
        claim.fitted_job_card_id = Some(fitted_on.id);
        claim.fitted_odometer = Some(40_000);
        claim
    })
    .await
    .unwrap_err();
    assert!(
        unknown_km.to_string().contains("mileage at the failure"),
        "{unknown_km}"
    );

    let in_terms = supplier_claims::raise(
        t.state(),
        &pia,
        base().fitted_on(fitted_on.id, 40_000, 58_000),
    )
    .await
    .unwrap();
    assert_eq!(in_terms.status, SupplierClaimStatus::Raised);
    assert_eq!(
        (
            in_terms.warranty_km,
            in_terms.fitted_odometer,
            in_terms.failure_odometer
        ),
        (Some(20_000), Some(40_000), Some(58_000))
    );

    // A part sold over the counter with a mileage in the terms: the period only.
    t.tickets
        .add(&about, "warranty", 14, "TKT-2026-0014", "closed");
    let counter = supplier_claims::raise(
        t.state(),
        &pia,
        NewSupplierClaim::new(
            14,
            about.clone(),
            40,
            "Starter motor",
            38_000,
            day(-170),
            12,
            day(-10),
        )
        .mileage(20_000),
    )
    .await
    .unwrap();
    assert_eq!(counter.fitted_odometer, None);
}

/// Labour included adds the labour of the replacement job card to the claim
/// (the part cost and AED 180); without it only the part is claimed.
#[renox::test]
async fn aut_base_labour_included_adds_the_labour_of_the_replacement_to_the_claim() {
    let t = boot().await;
    let about = closed_ticket(&t).await;
    let pia = pia(&t).await;
    let vehicle = t.hilux().await;
    let replacement = t.card_in(&vehicle, JobCardStatus::Delivered).await;
    t.approved_labour(&replacement, "Fit the alternator", 1_000, 18_000)
        .await;
    t.approved_part(&replacement, "Alternator", None, 1_000, 45_000)
        .await;

    let with_labour = supplier_claims::raise(
        t.state(),
        &pia,
        NewSupplierClaim::new(
            12,
            about.clone(),
            40,
            "Alternator",
            45_000,
            day(-170),
            12,
            day(-10),
        )
        .with_labour_of(replacement.id),
    )
    .await
    .unwrap();
    assert!(with_labour.labour_included);
    assert_eq!(
        (with_labour.labour_amount, with_labour.claim_amount),
        (18_000, 63_000),
        "the part cost and AED 180"
    );

    t.tickets
        .add(&about, "warranty", 15, "TKT-2026-0015", "closed");
    let part_only = supplier_claims::raise(
        t.state(),
        &pia,
        NewSupplierClaim::new(
            15,
            about.clone(),
            40,
            "Starter motor",
            38_000,
            day(-170),
            12,
            day(-10),
        ),
    )
    .await
    .unwrap();
    assert!(!part_only.labour_included);
    assert_eq!(
        (part_only.labour_amount, part_only.claim_amount),
        (0, 38_000)
    );
    t.tickets
        .add(&about, "warranty", 16, "TKT-2026-0016", "closed");
    let typed = supplier_claims::raise(
        t.state(),
        &pia,
        NewSupplierClaim::new(
            16,
            about.clone(),
            40,
            "Wiper motor",
            12_000,
            day(-170),
            12,
            day(-10),
        )
        .with_labour(5_000),
    )
    .await
    .unwrap();
    assert_eq!(typed.claim_amount, 17_000);
}

/// The supplier's answer and the claim document are recorded while the claim is
/// raised; the state does not change; a rejection needs its note.
#[renox::test]
async fn aut_base_the_suppliers_answer_is_recorded_without_changing_the_state() {
    let t = boot().await;
    let about = closed_ticket(&t).await;
    let pia = pia(&t).await;
    let claim = supplier_claims::raise(t.state(), &pia, alternator(&about))
        .await
        .unwrap();

    let document = supplier_claims::record_document(
        t.state(),
        &pia,
        claim.id,
        " claim-pdf-1 ",
        Some("GAS-RMA-77"),
    )
    .await
    .unwrap();
    assert_eq!(document.claim_document.as_deref(), Some("claim-pdf-1"));
    assert_eq!(document.supplier_reference.as_deref(), Some("GAS-RMA-77"));

    let no_note = supplier_claims::record_answer(
        t.state(),
        &pia,
        claim.id,
        SupplierAnswer::Rejected,
        None,
        None,
    )
    .await
    .unwrap_err();
    assert!(
        no_note.to_string().contains("why the supplier rejected"),
        "{no_note}"
    );
    let pending = supplier_claims::record_answer(
        t.state(),
        &pia,
        claim.id,
        SupplierAnswer::Pending,
        None,
        None,
    )
    .await
    .unwrap_err();
    assert!(
        pending.to_string().contains("accepted or rejected"),
        "{pending}"
    );
    let accepted = supplier_claims::record_answer(
        t.state(),
        &pia,
        claim.id,
        SupplierAnswer::Accepted,
        None,
        Some("Credit within 30 days"),
    )
    .await
    .unwrap();
    assert_eq!(accepted.supplier_answer, SupplierAnswer::Accepted);
    assert_eq!(accepted.status, SupplierClaimStatus::Raised);
    assert!(accepted.answered_at.is_some());
    assert_eq!(
        accepted.supplier_reference.as_deref(),
        Some("GAS-RMA-77"),
        "kept"
    );
}

/// The supplier credits the claim: it is recovered with the credit note, or
/// with a replacement received; without either it is not; and it ends.
#[renox::test]
async fn aut_base_a_claim_is_recovered_by_a_credit_note_or_a_replacement() {
    let t = boot().await;
    let about = closed_ticket(&t).await;
    let pia = pia(&t).await;
    let claim = supplier_claims::raise(t.state(), &pia, alternator(&about))
        .await
        .unwrap();

    let nothing = supplier_claims::recover(t.state(), &pia, claim.id, None, false)
        .await
        .unwrap_err();
    assert!(nothing.to_string().contains("credit note"), "{nothing}");
    let recovered = supplier_claims::recover(t.state(), &pia, claim.id, Some("CN-2026-31"), false)
        .await
        .unwrap();
    assert_eq!(recovered.status, SupplierClaimStatus::Recovered);
    assert_eq!(recovered.recovery_basis.as_deref(), Some("credit_note"));
    assert_eq!(recovered.recovery_reference.as_deref(), Some("CN-2026-31"));
    assert!(recovered.recovered_at.is_some());
    assert!(
        supplier_claims::write_off(t.state(), &pia, claim.id, "too late")
            .await
            .is_err(),
        "a recovered claim is over"
    );

    t.tickets
        .add(&about, "warranty", 17, "TKT-2026-0017", "closed");
    let second = supplier_claims::raise(
        t.state(),
        &pia,
        NewSupplierClaim::new(
            17,
            about.clone(),
            40,
            "Starter motor",
            38_000,
            day(-170),
            12,
            day(-10),
        ),
    )
    .await
    .unwrap();
    let replaced = supplier_claims::recover(t.state(), &pia, second.id, None, true)
        .await
        .unwrap();
    assert_eq!(replaced.recovery_basis.as_deref(), Some("replacement"));
}

/// A claim the supplier refused is written off with a reason, which waits for
/// the branch manager (the approval itself is tested in `aut_11_supplier.rs`).
#[renox::test]
async fn aut_base_a_refused_claim_is_written_off_with_a_reason() {
    let t = boot().await;
    let about = closed_ticket(&t).await;
    let pia = pia(&t).await;
    let claim = supplier_claims::raise(t.state(), &pia, alternator(&about))
        .await
        .unwrap();
    supplier_claims::record_answer(
        t.state(),
        &pia,
        claim.id,
        SupplierAnswer::Rejected,
        None,
        Some("Water damage"),
    )
    .await
    .unwrap();

    let blank = supplier_claims::write_off(t.state(), &pia, claim.id, " ")
        .await
        .unwrap_err();
    assert!(
        blank.to_string().to_lowercase().contains("reason"),
        "{blank}"
    );
    let asked =
        supplier_claims::write_off(t.state(), &pia, claim.id, "Supplier refused: water damage")
            .await
            .unwrap();
    assert_eq!(
        asked.claim.status,
        SupplierClaimStatus::Raised,
        "it waits for the branch manager (BL-AUT-11 decides it)"
    );
    assert!(asked.approval_request.is_some());
    assert_eq!(asked.claim.supplier_answer, SupplierAnswer::Rejected);

    let machine = SupplierClaim::machine();
    let guards = |name: &str| {
        machine
            .find(name)
            .unwrap()
            .guard_names()
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    assert_eq!(guards(WRITE_OFF), ["branch_manager_approval"]);
    assert_eq!(guards(RECOVER), ["credit_or_replacement"]);
    assert!(machine.find(RECOVER_SYSTEM).unwrap().is_system());
}

/// The return to supplier of the kept part is linked; when it is credited by a
/// debit note the claim is recovered by itself; a manual credit with the reason
/// *supplier refused* leaves it raised; a part that was not kept has no return.
#[renox::test]
async fn aut_base_the_credit_of_the_return_recovers_the_claim() {
    let t = boot().await;
    let about = closed_ticket(&t).await;
    let pia = pia(&t).await;
    let claim = supplier_claims::raise(t.state(), &pia, alternator(&about))
        .await
        .unwrap();
    let rts = Subject::new(SUPPLIER_RETURN_SUBJECT_TYPE, 4);

    let wrong = supplier_claims::link_return(
        t.state(),
        &pia,
        claim.id,
        &Subject::new("inventory.item", 4),
    )
    .await
    .unwrap_err();
    assert!(wrong.to_string().contains("return to supplier"), "{wrong}");
    let linked = supplier_claims::link_return(t.state(), &pia, claim.id, &rts)
        .await
        .unwrap();
    assert_eq!(linked.supplier_return_id, Some(4));

    // A manual credit because the supplier refused: the claim stays raised.
    t.state()
        .emit(SupplierReturnCredited {
            supplier_return: rts.clone(),
            supplier: PartyId(40),
            basis: "supplier_refused".to_owned(),
            debit_note: None,
            reason: Some("supplier refused".to_owned()),
            wastage: 45_000,
        })
        .await
        .unwrap();
    assert_eq!(
        SupplierClaim::find_or_404(t.db(), claim.id)
            .await
            .unwrap()
            .status,
        SupplierClaimStatus::Raised
    );

    // The debit note settles: the claim is recovered.
    t.state()
        .emit(SupplierReturnCredited {
            supplier_return: rts,
            supplier: PartyId(40),
            basis: "debit_note".to_owned(),
            debit_note: Some(wl_kernel::invoicing::SupplierDebitNoteId(21)),
            reason: None,
            wastage: 0,
        })
        .await
        .unwrap();
    let recovered = SupplierClaim::find_or_404(t.db(), claim.id).await.unwrap();
    assert_eq!(recovered.status, SupplierClaimStatus::Recovered);
    assert_eq!(recovered.recovery_basis.as_deref(), Some("credit_note"));
    assert_eq!(
        recovered.recovery_reference.as_deref(),
        Some("debit note 21")
    );

    // A part that was not kept has no return to link.
    t.tickets
        .add(&about, "warranty", 18, "TKT-2026-0018", "closed");
    let discarded = supplier_claims::raise(
        t.state(),
        &pia,
        NewSupplierClaim::new(
            18,
            about.clone(),
            40,
            "Starter motor",
            38_000,
            day(-170),
            12,
            day(-10),
        )
        .not_kept(),
    )
    .await
    .unwrap();
    assert!(
        supplier_claims::link_return(
            t.state(),
            &pia,
            discarded.id,
            &Subject::new(SUPPLIER_RETURN_SUBJECT_TYPE, 5)
        )
        .await
        .is_err()
    );
}

/// The ticket is found by its number among the warranty tickets of the job card.
#[renox::test]
async fn aut_base_a_ticket_is_found_by_its_number() {
    let t = boot().await;
    let about = closed_ticket(&t).await;
    assert_eq!(
        supplier_claims::ticket_by_number(t.state(), &about, " tkt-2026-0012 ")
            .await
            .unwrap(),
        12
    );
    assert!(
        supplier_claims::ticket_by_number(t.state(), &about, "TKT-2026-0099")
            .await
            .is_err()
    );
}
