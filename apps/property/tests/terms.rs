//! BL-PRP-04 (#254): `PRP.ACL.17`, a renewal or amendment version that changes
//! terms against the tenant is submitted only with a notice delivered in time
//! (renewal) or the tenant's signed acceptance attached to the draft.

mod support;

use property::notices::NOTICE_OF_CHANGED_TERMS;
use property::renewal;
use property::{Lease, LeaseStatus, NewLease, NewVersion};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine;
use wl_kernel::notify::NoticeStatus;

use support::{AGENT, Booted, MANAGER, TENANT, aed, boot, d};

/// A date in 2028.
fn y28(month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2028, month, day).unwrap()
}

struct Scene {
    t: Booted,
    v1: Lease,
    agent: User,
}

/// An `active` lease 2027-01-01 to 2027-12-31 at AED 100,000 in 4
/// instalments, no deposit, that is `renewal_due` (2027-09-02).
async fn scene() -> Scene {
    let t = boot().await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    let new = NewLease::new(unit.id, TENANT, d(1, 1), d(12, 31), aed(100_000), 4);
    let draft = property::leases::create_draft(t.state(), None, new)
        .await
        .unwrap();
    t.submit(&agent, &draft).await.assert_ok();
    t.activate(&manager, &draft).await.assert_ok();
    renewal::run(t.state(), d(9, 2)).await.unwrap();
    let v1 = t.reload(&draft).await;
    assert_eq!(v1.status, LeaseStatus::RenewalDue);
    Scene { t, v1, agent }
}

impl Scene {
    /// A renewal draft for 2028 at `rent` in `instalments` instalments with
    /// `deposit`.
    async fn renewal(&self, rent: i64, deposit: i64, instalments: u32) -> Lease {
        property::leases::create_version(
            self.t.state(),
            Some(self.agent.id),
            self.v1.id,
            NewVersion::renewal(y28(1, 1), y28(12, 31), rent, deposit, instalments),
        )
        .await
        .unwrap()
    }

    /// The notice of changed terms of the current version was delivered on
    /// `day`, as a staff proof says.
    fn proof(&self, day: NaiveDate) {
        self.t.notices.set_notice(
            &self.v1.subject(),
            NOTICE_OF_CHANGED_TERMS,
            NoticeStatus::ProofRecorded { delivered_on: day },
        );
    }

    /// Drops a draft that was refused, to try another.
    async fn drop_draft(&self, draft: &Lease) {
        self.t
            .set(draft, &[("status", &LeaseStatus::Cancelled)])
            .await;
    }
}

/// AC1: a renewal raising the rent from AED 100,000 to AED 105,000, with proof
/// of delivery dated 2027-09-25 (deadline 2027-10-02), is submitted.
#[renox::test]
async fn bl_prp_04_a_timely_notice_lets_a_higher_rent_be_submitted() {
    let s = scene().await;
    s.proof(d(9, 25));
    let v2 = s.renewal(aed(105_000), 0, 4).await;
    s.t.submit(&s.agent, &v2).await.assert_ok();
    assert_eq!(s.t.status(&v2).await, LeaseStatus::PendingSignature);
}

/// The deadline day counts.
#[renox::test]
async fn bl_prp_04_a_notice_delivered_on_the_deadline_day_is_timely() {
    let s = scene().await;
    s.proof(d(10, 2));
    let v2 = s.renewal(aed(105_000), 0, 4).await;
    s.t.submit(&s.agent, &v2).await.assert_ok();
}

/// AC2: proof dated 2027-10-10 is refused with the deadline; after attaching
/// the tenant's signed acceptance `submit` succeeds.
#[renox::test]
async fn bl_prp_04_a_late_notice_is_refused_until_the_acceptance_is_attached() {
    let s = scene().await;
    s.proof(d(10, 10));
    let v2 = s.renewal(aed(105_000), 0, 4).await;
    let refused = s.t.submit(&s.agent, &v2).await;
    refused.assert_status(422);
    let text = refused.text();
    assert!(text.contains("notice not timely (deadline 2027-10-02)"), "{text}");
    assert!(text.contains("2027-10-10"), "the proof status: {text}");
    assert!(text.contains("current terms"), "the first way forward: {text}");
    assert!(text.contains("signed acceptance"), "the second way forward: {text}");
    assert_eq!(s.t.status(&v2).await, LeaseStatus::Draft);

    // The draft shows what is missing.
    s.t.app.acting_as(&s.agent);
    s.t.app
        .get(&format!("/property/leases/{}", v2.id))
        .await
        .assert_ok()
        .assert_see("Not attached");

    s.t.accept(&v2);
    s.t.submit(&s.agent, &v2).await.assert_ok();
    assert_eq!(s.t.status(&v2).await, LeaseStatus::PendingSignature);
}

/// AC3: with the notice message given up and no proof, `submit` is refused;
/// a notice about another version does not count; the acceptance of another
/// version is not this draft's.
#[renox::test]
async fn bl_prp_04_a_notice_that_gave_up_is_not_counted() {
    let s = scene().await;
    let v2 = s.renewal(aed(105_000), 0, 4).await;
    // A proof for the draft itself is not the proof of the version replaced.
    s.t.notices.set_notice(
        &v2.subject(),
        NOTICE_OF_CHANGED_TERMS,
        NoticeStatus::ProofRecorded {
            delivered_on: d(9, 1),
        },
    );
    // An acceptance attached to the lease being replaced is not this draft's.
    s.t.accept(&s.v1);
    let refused = s.t.submit(&s.agent, &v2).await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("notice not delivered (deadline 2027-10-02)"),
        "{}",
        refused.text()
    );
}

/// A provider receipt counts as delivery, on the Asia/Dubai calendar day.
#[renox::test]
async fn bl_prp_04_a_provider_receipt_counts_on_the_dubai_day() {
    let s = scene().await;
    let at = |text: &str| -> renox::db::DateTime { text.parse().expect("an instant") };
    // 19:00 UTC on 2 October is 23:00 in Dubai: the deadline day.
    s.t.notices.set_notice(
        &s.v1.subject(),
        NOTICE_OF_CHANGED_TERMS,
        NoticeStatus::Delivered {
            at: at("2027-10-02T19:00:00Z"),
        },
    );
    let v2 = s.renewal(aed(105_000), 0, 4).await;
    s.t.submit(&s.agent, &v2).await.assert_ok();
}

/// A receipt after midnight in Dubai is the next day, so too late.
#[renox::test]
async fn bl_prp_04_a_receipt_after_midnight_in_dubai_is_late() {
    let s = scene().await;
    s.t.notices.set_notice(
        &s.v1.subject(),
        NOTICE_OF_CHANGED_TERMS,
        NoticeStatus::Delivered {
            at: "2027-10-02T20:30:00Z".parse().expect("an instant"),
        },
    );
    let v2 = s.renewal(aed(105_000), 0, 4).await;
    let refused = s.t.submit(&s.agent, &v2).await;
    refused.assert_status(422);
    assert!(refused.text().contains("not timely"), "{}", refused.text());
}

/// AC4: a renewal on the same rent and terms is submitted without notice or
/// acceptance; so is one that lowers the rent.
#[renox::test]
async fn bl_prp_04_a_renewal_on_the_current_terms_needs_nothing() {
    let s = scene().await;
    let same = s.renewal(aed(100_000), 0, 4).await;
    s.t.submit(&s.agent, &same).await.assert_ok();
    assert_eq!(s.t.status(&same).await, LeaseStatus::PendingSignature);
}

/// A rent decrease alone is not against the tenant.
#[renox::test]
async fn bl_prp_04_a_rent_decrease_alone_is_not_against_the_tenant() {
    let s = scene().await;
    let cheaper = s.renewal(aed(95_000), 0, 4).await;
    s.t.submit(&s.agent, &cheaper).await.assert_ok();
}

/// A higher deposit and fewer cheques are against the tenant, found
/// automatically; a no answer does not excuse them.
#[renox::test]
async fn bl_prp_04_a_higher_deposit_or_fewer_cheques_are_found_automatically() {
    let s = scene().await;
    for (rent, deposit, instalments) in [(aed(100_000), aed(1_000), 4), (aed(100_000), 0, 2)] {
        let draft = s.renewal(rent, deposit, instalments).await;
        // Answering no with a reason changes nothing for these.
        s.t.post(
            &s.agent,
            &format!("/property/leases/{}/terms-answer", draft.id),
            &[("against", "no"), ("reason", "small change")],
        )
        .await
        .assert_ok();
        let refused = s.t.submit(&s.agent, &draft).await;
        refused.assert_status(422);
        assert!(refused.text().contains("not delivered"), "{}", refused.text());
        s.drop_draft(&draft).await;
    }
}

/// AC5: a renewal changing only a clause is refused when the answer is yes
/// (also when it is not given); with no and the reason "typo fix" it is
/// submitted and the reason is in the history.
#[renox::test]
async fn bl_prp_04_another_changed_term_is_answered_with_a_reason() {
    let s = scene().await;
    let mut new = NewVersion::renewal(y28(1, 1), y28(12, 31), aed(100_000), 0, 4);
    new.terms = Some("Pets are not allowed.".to_owned());
    let v2 = property::leases::create_version(s.t.state(), Some(s.agent.id), s.v1.id, new)
        .await
        .unwrap();
    // Not answered counts as yes.
    s.t.submit(&s.agent, &v2).await.assert_status(422);

    let path = format!("/property/leases/{}/terms-answer", v2.id);
    s.t.post(&s.agent, &path, &[("against", "yes")]).await.assert_ok();
    s.t.submit(&s.agent, &v2).await.assert_status(422);

    // A no needs a reason.
    s.t.post(&s.agent, &path, &[("against", "no")])
        .await
        .assert_status(422);
    s.t.post(&s.agent, &path, &[("against", "maybe")])
        .await
        .assert_status(400);
    s.t.post(&s.agent, &path, &[("against", "no"), ("reason", "typo fix")])
        .await
        .assert_ok();
    s.t.submit(&s.agent, &v2).await.assert_ok();
    assert_eq!(s.t.status(&v2).await, LeaseStatus::PendingSignature);

    let history = state_machine::history(s.t.db(), Lease::TABLE, v2.id)
        .await
        .unwrap();
    let submit = history
        .iter()
        .find(|h| h.transition == "submit")
        .expect("the submit is in the history");
    assert_eq!(submit.reason.as_deref(), Some("typo fix"));
}

/// An added charge is another changed term; the answer is the agent's, and a
/// user of another building cannot give it.
#[renox::test]
async fn bl_prp_04_the_answer_is_for_a_draft_version_of_the_users_building() {
    let s = scene().await;
    let v2 = s.renewal(aed(100_000), 0, 4).await;
    let path = format!("/property/leases/{}/terms-answer", v2.id);
    let other = s.t.building_coded("TWR-B").await;
    let outsider = s.t.user_in("Omar", AGENT, other.id).await;
    s.t.post(&outsider, &path, &[("against", "yes")])
        .await
        .assert_forbidden();
    // A new lease has no such answer.
    let unit = s.t.unit(&s.t.building_coded("TWR-C").await, "1").await;
    let new = NewLease::new(unit.id, TENANT, d(1, 1), d(12, 31), aed(100_000), 4);
    let lease = property::leases::create_draft(s.t.state(), None, new)
        .await
        .unwrap();
    let boss = s
        .t
        .user_with("Boss", wl_kernel::COMPANY_ADMIN_ROLE)
        .await;
    s.t.post(
        &boss,
        &format!("/property/leases/{}/terms-answer", lease.id),
        &[("against", "yes")],
    )
    .await
    .assert_status(422);
}

/// AC6: an `active` lease ending 2027-12-31 with proof of a notice delivered
/// on 2027-03-01: an amendment raising the rent from 2027-06-01 is refused
/// without the tenant's acceptance (the notice alone is not enough); with it,
/// `submit` succeeds.
#[renox::test]
async fn bl_prp_04_an_amendment_against_the_tenant_always_needs_the_acceptance() {
    let t = boot().await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    let new = NewLease::new(unit.id, TENANT, d(1, 1), d(12, 31), aed(100_000), 4);
    let v1 = property::leases::create_draft(t.state(), None, new)
        .await
        .unwrap();
    t.submit(&agent, &v1).await.assert_ok();
    t.activate(&manager, &v1).await.assert_ok();
    assert_eq!(t.status(&v1).await, LeaseStatus::Active);
    t.notices.set_notice(
        &v1.subject(),
        NOTICE_OF_CHANGED_TERMS,
        NoticeStatus::ProofRecorded {
            delivered_on: d(3, 1),
        },
    );

    let v2 = property::leases::create_version(
        t.state(),
        Some(agent.id),
        v1.id,
        NewVersion::amendment(d(6, 1), d(12, 31), aed(110_000), 0, 2),
    )
    .await
    .unwrap();
    let refused = t.submit(&agent, &v2).await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("signed acceptance"),
        "{}",
        refused.text()
    );
    assert_eq!(t.status(&v2).await, LeaseStatus::Draft);

    t.accept(&v2);
    t.submit(&agent, &v2).await.assert_ok();
    assert_eq!(t.status(&v2).await, LeaseStatus::PendingSignature);
}

/// An amendment that is not against the tenant (a rent decrease) needs
/// neither notice nor acceptance.
#[renox::test]
async fn bl_prp_04_an_amendment_that_lowers_the_rent_needs_nothing() {
    let t = boot().await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    let new = NewLease::new(unit.id, TENANT, d(1, 1), d(12, 31), aed(100_000), 4);
    let v1 = property::leases::create_draft(t.state(), None, new)
        .await
        .unwrap();
    t.submit(&agent, &v1).await.assert_ok();
    t.activate(&manager, &v1).await.assert_ok();
    let v2 = property::leases::create_version(
        t.state(),
        Some(agent.id),
        v1.id,
        NewVersion::amendment(d(6, 1), d(12, 31), aed(90_000), 0, 2),
    )
    .await
    .unwrap();
    t.submit(&agent, &v2).await.assert_ok();
}
