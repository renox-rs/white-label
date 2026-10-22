//! Insurance of the demo clinic: the payers' agreements and contracts with
//! their windows and tariffs, the patients' policies (one expired), the
//! services that need a prior authorisation, and the authorisations in their
//! states.

use clinics::contracts::{self, NewContract};
use clinics::policies::{self, NewPolicy};
use renox::prelude::*;
use wl_invoicing::{PayerAgreement, VatRecipient};

use super::super::demo_data::day;
use super::people::{AHMED, FATIMA, JOHN, KHALID, LAYAN, PRIYA, YOUSEF};
use clinics::authorisations::{self, NewAuthorisation};
use super::Seed;

/// The payers' prices of the services, in fils.
const TARIFF: &[(&str, i64)] = &[
    ("consultation", 35_000),
    ("follow_up", 20_000),
    ("procedure", 120_000),
    ("dental_checkup", 25_000),
    ("physio_session", 30_000),
    ("dental_xray", 15_000),
    ("GLOVES-M", 1_500),
    ("ANAES-CART", 4_000),
];

/// A policy: patient, payer, plan, member id, first day, last day.
type PolicySpec = (usize, usize, &'static str, &'static str, i64, Option<i64>);

const POLICIES: &[PolicySpec] = &[
    (AHMED, 0, "Gold", "GS-100234", -300, Some(65)),
    (YOUSEF, 0, "Gold", "GS-100234-D1", -300, Some(65)),
    (KHALID, 0, "Gold", "GS-100998", -200, Some(160)),
    (FATIMA, 1, "Basic", "CH-55120", -200, Some(160)),
    (PRIYA, 2, "Select", "OM-77001", -100, Some(260)),
    (LAYAN, 2, "Select", "OM-77002", -90, Some(270)),
    // Ended 30 days ago: the card is still in the wallet.
    (JOHN, 1, "Basic", "CH-55999", -400, Some(-30)),
];

impl Seed {
    /// The policy of `patient`, if the clinic has one in force.
    pub(crate) fn policy_of(&self, patient: usize) -> Option<i64> {
        self.policies
            .iter()
            .find(|(known, _)| *known == patient)
            .map(|(_, id)| *id)
    }

    pub(crate) async fn insurance(&mut self) -> Result {
        let billing = self.user("billing@palmgrove.example").id;
        // (name, copay bp, deductible, plan, windows, late claims)
        let terms: [(&str, i64, i64, Option<&str>, (i64, i64, i64), bool); 3] = [
            ("Gulf Shield Gold", 2_000, 5_000, Some("Gold"), (30, 30, 2), false),
            ("Crescent Basic", 1_000, 0, Some("Basic"), (14, 21, 1), true),
            ("Oasis Select", 3_000, 0, None, (21, 14, 1), false),
        ];
        for (index, (name, copay, deductible, plan, windows, late)) in terms.iter().enumerate() {
            let payer = self.payers[index];
            let mut agreement = PayerAgreement {
                legal_entity_id: self.entity,
                payer_party_id: payer.0,
                name: (*name).to_owned(),
                active: true,
                vat_recipient: VatRecipient::Customer,
                first_submission_days: windows.0,
                resubmission_days: windows.1,
                resubmission_limit: windows.2,
                write_off_limit: 100_000,
                accepts_late_submissions: *late,
                ..PayerAgreement::default()
            };
            agreement.insert(self.db()).await?;
            let mut new = NewContract::new(payer, day(-365))
                .agreement(agreement.id)
                .copay_bp(*copay)
                .deductible(*deductible)
                .windows(windows.0, windows.1, windows.2);
            if let Some(plan) = plan {
                new = new.plan(plan);
            }
            if *late {
                new = new.accepts_late_claims();
            }
            let contract = contracts::create(&self.state, Some(billing), new).await?;
            for (code, price) in TARIFF {
                // Oasis does not cover procedures: the patient pays them.
                let covered = !(index == 2 && *code == "procedure");
                contracts::set_tariff(&self.state, contract.id, code, Some(*price), covered)
                    .await?;
            }
            self.contracts.push(contract.id);
        }
        // An older Crescent contract that ended: the windows were longer.
        let old = NewContract::new(self.payers[1], day(-730))
            .valid_to(day(-366))
            .windows(30, 30, 2);
        contracts::create(&self.state, Some(billing), old).await?;
        for (patient, payer, plan, member, from, to) in POLICIES {
            let mut new = NewPolicy::new(
                self.patients[*patient],
                self.payers[*payer],
                plan,
                member,
                day(*from),
            );
            if let Some(to) = to {
                new = new.valid_to(day(*to)).card_valid_to(day(*to));
            }
            let policy = policies::create(&self.state, Some(billing), new).await?;
            self.policies.push((*patient, policy.id));
        }
        // Gulf Shield and Oasis ask for an authorisation for a procedure.
        for payer in [0, 2] {
            clinics::authorisations::require_for(&self.state, self.payers[payer], "procedure")
                .await?;
        }
        Ok(())
    }

    /// The prior authorisations: Khalid's is approved, Priya's waits for the
    /// payer (her procedure is flagged until it is approved).
    pub(crate) async fn authorisations(&mut self) -> Result {
        let billing = self.user("billing@palmgrove.example");
        let (billing_id, email) = (billing.id, "billing@palmgrove.example");
        for (patient, approve, reference) in [(KHALID, true, "GS-PA-90417"), (PRIYA, false, "")] {
            let policy = self.policy_of(patient).unwrap_or_default();
            let created = authorisations::create(
                &self.state,
                Some(billing_id),
                NewAuthorisation::new(policy, 1).service("procedure"),
            )
            .await?;
            let id = created.id;
            self.driver
                .post(email, &format!("/clinics/authorisations/{id}/request"), &[("reference", "")])
                .await?;
            if approve {
                self.driver
                    .post(
                        email,
                        &format!("/clinics/authorisations/{id}/approve"),
                        &[("reference", reference), ("valid_until", &day(30).to_string())],
                    )
                    .await?;
            }
        }
        Ok(())
    }
}
