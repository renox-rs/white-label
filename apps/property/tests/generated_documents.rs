//! BL-PRP-31 (#476): lease `submit` generates the bilingual tenancy contract
//! and sends it to the tenant for signature; owner statement `publish`
//! generates the statement PDF and sends it to the owner; a PDF that cannot be
//! generated refuses `publish`; a revision gets its own PDF.
//!
//! Rendering and storing the PDF is the kernel's generator (wl-documents,
//! BL-DOC-11); a recording generator stands for it here. The messages carry
//! the stored file as an attachment (BL-NTF-10); the message log of wl-notify
//! lists it when the e-mail goes out.

mod support;

use std::sync::{Arc, Mutex};

use property::agreement::{AgreementStatus, ManagementAgreement};
use property::generated::{
    self, GeneratedDocument, OWNER_STATEMENT_TEMPLATE, PdfDocuments, PdfFuture, PdfRequest,
    TENANCY_CONTRACT_TEMPLATE,
};
use property::notices::{CONTRACT_FOR_SIGNATURE, OWNER_STATEMENT};
use property::statement::{self, OwnerStatement, StatementStatus};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::parties::PartyKind;
use wl_kernel::{PartyId, Subject};

use support::{ACCOUNTANT, AGENT, Booted, MANAGER, boot};

const OWNER: PartyId = PartyId(31);

/// A generator that records what it was asked and answers with document ids
/// from 900, or refuses everything.
#[derive(Clone, Default)]
struct Pdfs(Arc<Mutex<Vec<PdfRequest>>>, Arc<Mutex<bool>>);

impl Pdfs {
    fn asked(&self) -> Vec<PdfRequest> {
        self.0.lock().unwrap().clone()
    }

    fn fail(&self, fail: bool) {
        *self.1.lock().unwrap() = fail;
    }
}

impl PdfDocuments for Pdfs {
    fn generate<'a>(
        &'a self,
        _state: &'a AppState,
        request: PdfRequest,
    ) -> PdfFuture<'a, Option<i64>> {
        Box::pin(async move {
            if *self.1.lock().unwrap() {
                return Err(Error::Status(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "The PDF could not be rendered: the template is missing.".to_owned(),
                ));
            }
            let mut asked = self.0.lock().unwrap();
            asked.push(request);
            Ok(Some(899 + i64::try_from(asked.len()).unwrap()))
        })
    }
}

fn ymd(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

async fn booted_with(pdfs: &Pdfs) -> Booted {
    let t = boot().await;
    generated::set_generator(t.state(), Arc::new(pdfs.clone()));
    t
}

/// Done when 1: given lease L-0042 in draft, `submit` stores *Tenancy contract
/// L-0042 v1* (generated, bilingual) and the tenant gets it with
/// `lease.contract_for_signature`. It is not generated again.
#[renox::test]
async fn bl_prp_31_submit_generates_the_contract_and_sends_it_for_signature() {
    let pdfs = Pdfs::default();
    let t = booted_with(&pdfs).await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let lease = t.lease(&unit, 30).await;

    t.submit(&agent, &lease).await.assert_ok();

    let lease = t.reload(&lease).await;
    assert_eq!(lease.status, property::LeaseStatus::PendingSignature);
    let contract = GeneratedDocument::of(t.db(), &lease.subject(), TENANCY_CONTRACT_TEMPLATE)
        .await
        .unwrap()
        .expect("the contract");
    let title = format!("Tenancy contract {}", lease.number_with_version());
    assert_eq!(contract.title, title);
    assert!(title.ends_with(" v1"));
    let asked = pdfs.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].languages, vec!["en", "ar"]);
    assert_eq!(asked[0].kind, generated::TENANCY_CONTRACT_KIND);
    assert_eq!(asked[0].data["lease"], lease.number_with_version());
    assert!(
        asked[0].data["title"]["ar"]
            .as_str()
            .unwrap()
            .contains("عقد")
    );
    let sent = t.notices.of_type(CONTRACT_FOR_SIGNATURE);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].data["document"], contract.document_id);
    let attached = wl_kernel::document_store::attachments_in(&sent[0].data);
    assert_eq!(attached.len(), 1, "the contract is attached");
    assert_eq!(attached[0].document.0, contract.document_id);
    assert_eq!(sent[0].subject, lease.subject());

    // A submitted version's contract is never generated again.
    let again = generated::contract_for(t.state(), &lease).await.unwrap();
    assert_eq!(again.map(|c| c.id), Some(contract.id));
    assert_eq!(pdfs.asked().len(), 1);
}

/// When the contract cannot be generated, nothing is submitted.
#[renox::test]
async fn bl_prp_31_a_contract_that_cannot_be_generated_refuses_submit() {
    let pdfs = Pdfs::default();
    pdfs.fail(true);
    let t = booted_with(&pdfs).await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let lease = t.lease(&unit, 30).await;

    let refused = t.submit(&agent, &lease).await;
    refused.assert_status(422);
    assert_eq!(t.status(&lease).await, property::LeaseStatus::Draft);
    assert!(t.notices.of_type(CONTRACT_FOR_SIGNATURE).is_empty());
}

struct Scene {
    t: Booted,
    pdfs: Pdfs,
    agreement: ManagementAgreement,
    accountant: User,
    manager: User,
}

async fn statement_scene() -> Scene {
    let pdfs = Pdfs::default();
    let t = booted_with(&pdfs).await;
    t.parties
        .add(OWNER.0, "Hessa Al Mansoori", PartyKind::Person);
    let mut agreement = ManagementAgreement {
        owner_party_id: OWNER.0,
        fee_bp: 500,
        payout_frequency: "monthly".to_owned(),
        notice_days: 90,
        start_date: ymd(2026, 1, 1),
        status: AgreementStatus::Active,
        ..Default::default()
    };
    let mut tx = t.db().begin().await.unwrap();
    agreement
        .insert_in_state(&mut tx, AgreementStatus::Active, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let accountant = t.user_with("Amal", ACCOUNTANT).await;
    let manager = t.user_with("Mona", MANAGER).await;
    Scene {
        t,
        pdfs,
        agreement,
        accountant,
        manager,
    }
}

impl Scene {
    /// An approved statement for October 2026 with a balance of AED 18,450.
    async fn approved(&self) -> OwnerStatement {
        let draft = statement::generate(
            self.t.state(),
            self.agreement.id,
            ymd(2026, 10, 1),
            ymd(2026, 10, 31),
        )
        .await
        .unwrap()
        .expect("a draft");
        self.t
            .post(
                &self.accountant,
                &format!("/property/statements/{}/lines", draft.id),
                &[
                    ("kind", "collected"),
                    ("description", "Rent October"),
                    ("amount", "18450"),
                ],
            )
            .await
            .assert_ok();
        self.run(&self.manager, &draft, "approve", "")
            .await
            .assert_status(303);
        let approved = self.reload(&draft).await;
        assert_eq!(approved.status, StatementStatus::Approved);
        assert_eq!(approved.balance(), 1_845_000);
        approved
    }

    async fn run(
        &self,
        user: &User,
        statement: &OwnerStatement,
        name: &str,
        reason: &str,
    ) -> renox::testing::TestResponse {
        let pairs: Vec<(&str, &str)> = if reason.is_empty() {
            Vec::new()
        } else {
            vec![("reason", reason)]
        };
        self.t
            .post_plain(
                user,
                &format!("/property/statements/{}/transitions/{name}", statement.id),
                &pairs,
            )
            .await
    }

    async fn publish(&self, statement: &OwnerStatement) -> renox::testing::TestResponse {
        self.t
            .post(
                &self.accountant,
                &format!("/property/statements/{}/publish", statement.id),
                &[],
            )
            .await
    }

    async fn reload(&self, statement: &OwnerStatement) -> OwnerStatement {
        OwnerStatement::find_or_404(self.t.db(), statement.id)
            .await
            .unwrap()
    }

    async fn pdf_of(&self, statement: &OwnerStatement) -> Option<GeneratedDocument> {
        GeneratedDocument::of(
            self.t.db(),
            &Subject::new("property.owner_statement", statement.id),
            OWNER_STATEMENT_TEMPLATE,
        )
        .await
        .unwrap()
    }
}

/// Done when 2: `publish` stores the PDF of the approved October statement
/// (balance AED 18,450) and the owner's message carries it.
#[renox::test]
async fn bl_prp_31_publish_generates_the_statement_pdf_and_sends_it() {
    let s = statement_scene().await;
    let approved = s.approved().await;

    s.publish(&approved).await.assert_ok();

    let published = s.reload(&approved).await;
    assert_eq!(published.status, StatementStatus::Published);
    let pdf = s.pdf_of(&published).await.expect("the PDF");
    assert_eq!(pdf.title, format!("Owner statement {}", published.number()));
    let asked = s.pdfs.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].data["balance"], "AED 18,450.00");
    assert_eq!(asked[0].languages, vec!["en", "ar"]);
    let sent = s.t.notices.of_type(OWNER_STATEMENT);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].data["document"], pdf.document_id);
    let attached = wl_kernel::document_store::attachments_in(&sent[0].data);
    assert_eq!(attached.len(), 1, "the PDF is attached");
    assert_eq!(attached[0].document.0, pdf.document_id);
    assert_eq!(sent[0].data["balance"], "AED 18,450.00");
}

/// Done when 3: when the PDF cannot be generated, `publish` is refused and the
/// statement stays `approved`; nothing is sent.
#[renox::test]
async fn bl_prp_31_publish_is_refused_when_the_pdf_cannot_be_generated() {
    let s = statement_scene().await;
    let approved = s.approved().await;
    s.pdfs.fail(true);

    let refused = s.publish(&approved).await;

    refused.assert_status(422);
    assert!(refused.text().contains("could not be rendered"));
    assert_eq!(s.reload(&approved).await.status, StatementStatus::Approved);
    assert!(s.pdf_of(&approved).await.is_none());
    assert!(s.t.notices.of_type(OWNER_STATEMENT).is_empty());
}

/// Done when 4: a revision published later has its own PDF; the first one is
/// still there.
#[renox::test]
async fn bl_prp_31_a_revision_has_its_own_pdf_and_the_first_stays() {
    let s = statement_scene().await;
    let approved = s.approved().await;
    s.publish(&approved).await.assert_ok();
    let first = s.pdf_of(&approved).await.expect("the first PDF");

    s.run(&s.accountant, &approved, "revise", "AMC bill missing")
        .await
        .assert_status(303);
    let revision = OwnerStatement::of_agreement(s.t.db(), s.agreement.id)
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.revision == 2)
        .expect("revision 2");
    s.run(&s.manager, &revision, "approve", "")
        .await
        .assert_status(303);
    s.publish(&revision).await.assert_ok();

    let second = s.pdf_of(&revision).await.expect("the revision's PDF");
    assert_ne!(second.document_id, first.document_id);
    let still = s.pdf_of(&approved).await.expect("the first PDF stays");
    assert_eq!(still.document_id, first.document_id);
    assert_eq!(s.pdfs.asked().len(), 2);
}
