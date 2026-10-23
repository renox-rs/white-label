//! What happens to the batches and their students, told day by day on the
//! seed's clock: sessions are held and marked, a student gets sick and gives
//! notice, a student withdraws and the centre manager approves the refund, a
//! payment is charged back, a student is blocked, a batch is cancelled before
//! it starts, another is terminated after three sessions, certificates are
//! issued, revoked and reissued, and the nightly runs remind the centre of
//! the documents that end.

use renox::chrono::{Datelike, Weekday};
use renox::prelude::*;
use wl_kernel::approvals::{ApprovalRequest, ApprovalStatus};
use wl_payments::Payment;

use super::super::demo_data::{at, day};
use super::hold::offset;
use super::{
    ACCOUNTANT_LOGIN, ADMIN_LOGIN, COORDINATOR, COUNSELLOR_LOGIN, MANAGER, Seed,
};

impl Seed {
    /// A request to the router on day `on` as `who`.
    pub(crate) async fn act(
        &self,
        on: i64,
        who: &str,
        path: &str,
        form: &[(&str, &str)],
    ) -> Result {
        at!(self, day(on), { self.driver.post(who, path, form).await })?;
        Ok(())
    }

    /// The centre manager decides every request of the app that waits.
    pub(crate) async fn approve_waiting(&self, on: i64) -> Result {
        loop {
            let waiting = ApprovalRequest::query()
                .where_(ApprovalRequest::STATUS.eq(ApprovalStatus::Pending))
                .order_by(ApprovalRequest::ID)
                .get(self.db())
                .await?;
            let Some(request) = waiting
                .into_iter()
                .find(|r| r.subject_type.starts_with("edtech."))
            else {
                return Ok(());
            };
            self.act(on, MANAGER, &format!("/approvals/{}/approve", request.id), &[])
                .await?;
        }
    }

    /// The nightly run of wl-documents on day `on`: the reminders and the
    /// events of the documents that end or ended.
    pub(crate) async fn nightly_documents(&self, on: i64) -> Result {
        at!(self, day(on), {
            wl_documents::expiry::run(&self.state, day(on)).await
        })?;
        Ok(())
    }

    /// The day after the `n`th session of the batch, from today.
    async fn after_session(&self, key: &str, n: usize) -> Result<i64> {
        let sessions = self.sessions_of(key).await?;
        Ok(offset(sessions[n - 1].session_date) + 1)
    }

    /// The system completes the active enrolments of `keys` the day after
    /// the last session; a certificate is drafted for each that attended
    /// enough.
    async fn complete_all(&self, keys: &[&str], on: i64) -> Result {
        for key in keys {
            let id = self.enrolment(key);
            at!(self, day(on), { edtech::enrolments::complete(&self.state, id).await })?;
        }
        Ok(())
    }

    async fn certificate_of(&self, key: &str) -> Result<edtech::Certificate> {
        let id = self.enrolment(key);
        edtech::Certificate::of_enrolment(self.db(), id)
            .await?
            .into_iter()
            .last()
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: {key} has no certificate").into())
    }

    async fn certificate_action(
        &self,
        key: &str,
        action: &str,
        on: i64,
        who: &str,
        form: &[(&str, &str)],
    ) -> Result {
        let certificate = self.certificate_of(key).await?;
        self.act(
            on,
            who,
            &format!("/edtech/certificates/{}/{action}", certificate.id),
            form,
        )
        .await
    }

    pub(crate) async fn stories(&mut self) -> Result {
        // Sixty days before the centre permit ends, the owner and the centre
        // manager are told; thirty days before the clearance of Omar ends.
        self.nightly_documents(-15).await?;
        self.english_done().await?;
        self.excel_done().await?;
        self.coding_terminated().await?;
        self.excel_terminated().await?;
        self.french_cancelled().await?;
        self.cancel_planned().await?;
        self.ielts().await?;
        self.python().await?;
        self.arabic_kids().await?;
        self.gmat().await?;
        self.french_running().await?;
        self.open_batches().await?;
        self.finish().await?;
        Ok(())
    }

    /// Completed: six sessions, the second instalment of Amal, a student who
    /// missed too many for a certificate, and the certificates.
    async fn english_done(&mut self) -> Result {
        let key = "eng_done";
        // The second instalment of Amal falls due a month after she enrolled.
        let due = edtech::Instalment::of_enrolment(self.db(), self.enrolment("eng.amal"))
            .await?
            .into_iter()
            .find(|i| i.number == 2)
            .map_or(-27, |i| offset(i.due_date));
        at!(self, day(due), {
            edtech::fees::invoice_due(&self.state, day(due)).await
        })?;
        let second = edtech::Instalment::of_enrolment(self.db(), self.enrolment("eng.amal"))
            .await?
            .into_iter()
            .find(|i| i.number == 2)
            .and_then(|i| i.invoice_id)
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no second instalment"))?;
        self.pay_invoice(
            second,
            wl_kernel::payments::PaymentMethod::Cash,
            None,
            due + 1,
            "",
        )
        .await?;
        self.hold_until(
            key,
            -1,
            &[
                (2, "eng.fatima", "excused"),
                (4, "eng.elena", "absent"),
                (5, "eng.elena", "absent"),
            ],
        )
        .await?;
        let last = self.after_session(key, 6).await?;
        self.complete_all(
            &["eng.amal", "eng.priya", "eng.james", "eng.fatima", "eng.elena"],
            last,
        )
        .await?;
        for student in ["eng.amal", "eng.priya", "eng.james"] {
            self.certificate_action(student, "issue", last + 2, MANAGER, &[])
                .await?;
        }
        // A name corrected after the print: the certificate is superseded and
        // the new draft is issued; another is corrected and waits as a draft.
        self.certificate_action(
            "eng.amal",
            "reissue",
            last + 4,
            COORDINATOR,
            &[
                ("reason", "The name is spelt in full on the passport"),
                ("student_name", "Amal Mohammed Al Hammadi"),
            ],
        )
        .await?;
        self.certificate_action("eng.amal", "issue", last + 4, MANAGER, &[])
            .await?;
        self.certificate_action(
            "eng.priya",
            "reissue",
            last + 4,
            COORDINATOR,
            &[
                ("reason", "The middle initial is added"),
                ("student_name", "Priya R. Nair"),
            ],
        )
        .await?;
        // Withdrawn for an error.
        self.certificate_action(
            "eng.james",
            "revoke",
            last + 6,
            ADMIN_LOGIN,
            &[("reason", "Issued in error: the final assessment was not sat")],
        )
        .await?;
        Ok(())
    }

    /// Completed: three sessions, paid by the employer; both certificates are
    /// issued and one of them revoked.
    async fn excel_done(&mut self) -> Result {
        let key = "xl_done";
        self.hold_until(key, -1, &[]).await?;
        let last = self.after_session(key, 3).await?;
        self.complete_all(&["xl.khalid", "xl.daniel"], last).await?;
        for student in ["xl.khalid", "xl.daniel"] {
            self.certificate_action(student, "issue", last + 1, COORDINATOR, &[])
                .await?;
        }
        self.certificate_action(
            "xl.daniel",
            "revoke",
            last + 3,
            ADMIN_LOGIN,
            &[("reason", "The employer withdrew the nomination: duplicate record")],
        )
        .await
    }

    /// The batch of kids' coding stops after three sessions: one child moves
    /// to the next batch, two are refunded for the sessions they did not have.
    async fn coding_terminated(&mut self) -> Result {
        let key = "code_term";
        for index in 0..3 {
            let marks: &[(&str, &str)] = if index == 1 { &[("code.kofi", "late")] } else { &[] };
            self.hold(key, index, marks).await?;
        }
        let on = self.after_session(key, 3).await?;
        let id = self.batches[key];
        let target = self.batches["code_open"].to_string();
        let choices = [
            (format!("choice_{}", self.enrolment("code.yousef")), target),
            (format!("choice_{}", self.enrolment("code.aarav")), "withdraw".into()),
            (format!("choice_{}", self.enrolment("code.kofi")), "withdraw".into()),
        ];
        let mut form: Vec<(&str, &str)> = vec![
            ("reason", "tutor_unavailable"),
            ("note", "The tutor has left the centre; no cover with the same clearance."),
        ];
        form.extend(choices.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        self.act(on, COORDINATOR, &format!("/edtech/batches/{id}/terminate"), &form)
            .await?;
        self.approve_waiting(on).await
    }

    /// Stopped after two sessions for too few students; both are refunded.
    async fn excel_terminated(&mut self) -> Result {
        let key = "xl_term";
        for index in 0..2 {
            self.hold(key, index, &[]).await?;
        }
        let on = self.after_session(key, 2).await?;
        let id = self.batches[key];
        self.act(
            on,
            COORDINATOR,
            &format!("/edtech/batches/{id}/terminate"),
            &[
                ("reason", "too_few_students"),
                ("note", "Two of eight seats were taken."),
            ],
        )
        .await?;
        self.approve_waiting(on).await
    }

    /// Cancelled before its first session: a student moves to the open
    /// batch, one is refunded in full, one never paid.
    async fn french_cancelled(&mut self) -> Result {
        let id = self.batches["fr_cancel"];
        let target = self.batches["fr_open"].to_string();
        let moves = format!("choice_{}", self.enrolment("frc.arjun"));
        let leaves = format!("choice_{}", self.enrolment("frc.lucas"));
        self.act(
            -12,
            COORDINATOR,
            &format!("/edtech/batches/{id}/cancel"),
            &[
                ("reason", "Too few students: three of eight seats"),
                (moves.as_str(), target.as_str()),
                (leaves.as_str(), "withdraw"),
            ],
        )
        .await?;
        self.approve_waiting(-12).await
    }

    /// A planned batch that never opened.
    async fn cancel_planned(&mut self) -> Result {
        let id = self.batches["eng_cancel"];
        self.act(
            -3,
            COORDINATOR,
            &format!("/edtech/batches/{id}/cancel"),
            &[("reason", "The tutor is on leave in that term")],
        )
        .await
    }

    /// IELTS: absences and a notice, a withdrawal with a refund, a transfer
    /// to the weekend batch, a freeze, a block, a chargeback and a student
    /// with a package of private sessions.
    async fn ielts(&mut self) -> Result {
        let key = "ielts_run";
        let marks = [
            (1, "ielts.maria", "excused"),
            (3, "ielts.maria", "absent"),
            (2, "ielts.james", "late"),
            (1, "ielts.daniel", "absent"),
            (4, "ielts.rania", "absent"),
            (3, "pkg.hassan", "add"),
            (5, "pkg.hassan", "add_absent"),
        ];
        self.hold_until(key, -9, &marks).await?;
        let daniel = self.enrolment("ielts.daniel");
        self.act(
            -8,
            COUNSELLOR_LOGIN,
            &format!("/edtech/enrolments/{daniel}/withdraw"),
            &[("reason", "Moving to another emirate for work")],
        )
        .await?;
        self.approve_waiting(-8).await?;
        self.hold_until(key, -7, &marks).await?;
        let khalid = self.enrolment("ielts.khalid");
        let target = self.batches["ielts_open"].to_string();
        self.act(
            -6,
            COUNSELLOR_LOGIN,
            &format!("/edtech/enrolments/{khalid}/transfer"),
            &[("target", target.as_str())],
        )
        .await?;
        self.hold_until(key, -5, &marks).await?;
        let nadia = self.enrolment("ielts.nadia");
        let until = day(10).to_string();
        self.act(
            -5,
            COUNSELLOR_LOGIN,
            &format!("/edtech/enrolments/{nadia}/freeze"),
            &[("until", until.as_str())],
        )
        .await?;
        self.hold_until(key, -4, &marks).await?;
        self.block_rania_on(-4).await?;
        self.hold_until(key, -1, &marks).await?;
        // The cardholder disputes the online payment of Fatima.
        let payment = Payment::query()
            .where_(Payment::REFERENCE.eq("GW-88231"))
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no gateway payment"))?;
        let deadline = day(10).to_string();
        self.act(
            -3,
            ACCOUNTANT_LOGIN,
            &format!("/payments/{}/chargeback", payment.id),
            &[
                ("reference", "CB-2026-0412"),
                ("deadline", deadline.as_str()),
                ("fee", "50.00"),
            ],
        )
        .await
    }

    async fn block_rania_on(&self, on: i64) -> Result {
        at!(self, day(on), { self.block_rania().await })
    }

    async fn python(&mut self) -> Result {
        // Thirty days before the clearance of Omar ends, the nightly run
        // reminds the coordinator and flags what he teaches after it.
        self.nightly_documents(-10).await?;
        self.hold_until("py_run", -1, &[(1, "py.priya", "absent")])
            .await
    }

    /// Arabic for kids: a session cancelled with a make-up on a Sunday, two
    /// missed sessions in a row, a notice that earns a make-up right.
    async fn arabic_kids(&mut self) -> Result {
        let key = "arb_run";
        let sessions = self.sessions_of(key).await?;
        let second = &sessions[1];
        let mut sunday = day(1);
        while sunday.weekday() != Weekday::Sun {
            sunday = sunday.succ_opt().unwrap_or(sunday);
        }
        let make_up = sunday.to_string();
        let path = format!("/edtech/sessions/{}/cancel", second.id);
        self.act(
            offset(second.session_date) - 1,
            COORDINATOR,
            &path,
            &[
                ("reason", "The tutor is unwell"),
                ("make_up_on", make_up.as_str()),
            ],
        )
        .await?;
        self.hold_until(
            key,
            -1,
            &[
                (0, "arb.mona", "absent"),
                (2, "arb.mona", "absent"),
                (0, "arb.aarav", "excused"),
                (3, "arb.yousef", "late"),
            ],
        )
        .await
    }

    async fn gmat(&mut self) -> Result {
        self.hold_until("gmat_run", -1, &[(1, "gmat.amal", "late")])
            .await
    }

    /// French with a tutor whose clearance ended 10 days ago: the sessions
    /// taught since are accepted but flagged, and the coming ones too.
    async fn french_running(&mut self) -> Result {
        let key = "fr_run";
        self.hold_until(key, -10, &[]).await?;
        self.nightly_documents(-9).await?;
        self.hold_until(key, -1, &[]).await
    }

    /// The open batches: the full one has a waiting list, a seat that was
    /// held 48 hours lapses, a discount waits for the centre manager.
    async fn open_batches(&mut self) -> Result {
        let id = self.batches["fr_open"];
        let hassan = self.students[super::people::HASSAN].0.to_string();
        self.act(
            -1,
            COUNSELLOR_LOGIN,
            &format!("/edtech/batches/{id}/waiting"),
            &[("student", hassan.as_str())],
        )
        .await?;
        // The daily run of the seat holds.
        at!(self, day(-2), {
            edtech::lapse::lapse_due(&self.state, renox::db::now()).await
        })?;
        Ok(())
    }

    /// The nightly runs of today: documents, the instalments, the batches.
    async fn finish(&mut self) -> Result {
        self.approve_waiting(0).await?;
        // The accountant issues the invoices still in draft (the seats held
        // for the students who have not paid yet).
        let drafts = wl_invoicing::Invoice::query()
            .where_(wl_invoicing::Invoice::STATUS.eq(wl_kernel::invoicing::InvoiceStatus::Draft))
            .order_by(wl_invoicing::Invoice::ID)
            .get(self.db())
            .await?;
        for invoice in drafts {
            self.issue_invoice(invoice.id, -1).await?;
        }
        // The refunds the withdrawals asked for: the accountant approves
        // some, rejects one and leaves the rest for tomorrow.
        let refunds = wl_payments::refund::Refund::query()
            .where_(
                wl_payments::refund::Refund::STATUS
                    .eq(wl_payments::refund::RefundStatus::Requested),
            )
            .order_by(wl_payments::refund::Refund::ID)
            .get(self.db())
            .await?;
        for (index, refund) in refunds.iter().enumerate() {
            let path = format!("/refunds/{}/transitions/", refund.id);
            match index {
                0..=2 => {
                    self.act(-1, ACCOUNTANT_LOGIN, &format!("{path}approve"), &[])
                        .await?;
                }
                3 => {
                    self.act(
                        -1,
                        ACCOUNTANT_LOGIN,
                        &format!("{path}reject"),
                        &[("reason", "The card was refunded by the bank already")],
                    )
                    .await?;
                }
                _ => {}
            }
        }
        self.nightly_documents(0).await?;
        edtech::fees::invoice_due(&self.state, day(0)).await?;
        edtech::batches::run_daily(&self.state).await?;
        Ok(())
    }
}
