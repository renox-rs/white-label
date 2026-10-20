//! The demo's documents: identity papers of the tenants, the signed contract
//! and the Ejari certificate of a lease. Each is attached by the system, as an
//! upload from the portal would be, and accepted by the company admin, so it
//! passes through the document module's review like a real one.

use renox::Upload;
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_documents::{Documents, NewDocument};
use wl_kernel::Subject;
use wl_kernel::parties::PARTY_SUBJECT_TYPE;

use super::people::{Who, login_of};
use super::roles::ADMIN;
use super::seed_people::{emirates_id, passport, visa};
use super::{Seed, day, specimen};

/// How complete a tenant's papers are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Papers {
    /// Valid for a long time.
    Full,
    /// The Emirates ID expires in this many days (the rest is valid).
    IdExpiresIn(i64),
    /// No passport on file: the compliance block refuses the lease.
    NoPassport,
    /// The residence visa expired this many days ago: the compliance block
    /// refuses the lease.
    VisaExpired(i64),
}

impl Seed {
    /// Attaches a document of `kind` to `subject` and has the company admin
    /// accept it; returns its id.
    pub(crate) async fn document(
        &mut self,
        subject: Subject,
        kind: &str,
        title: &str,
        number: Option<String>,
        expires: Option<NaiveDate>,
        issued: Option<NaiveDate>,
    ) -> Result<i64> {
        let id = self
            .attach_document(subject, kind, title, number, expires, issued)
            .await?;
        self.driver
            .post(
                login_of(ADMIN).email,
                &format!("/documents/{id}/accept"),
                &[],
            )
            .await?;
        Ok(id)
    }

    /// Attaches a document and leaves it waiting for review.
    pub(crate) async fn attach_document(
        &mut self,
        subject: Subject,
        kind: &str,
        title: &str,
        number: Option<String>,
        expires: Option<NaiveDate>,
        issued: Option<NaiveDate>,
    ) -> Result<i64> {
        let mut new = NewDocument::new(
            subject,
            kind,
            Upload::new(
                format!("{kind}.pdf"),
                "application/pdf",
                specimen::pdf(title),
            ),
        )
        .title(title);
        if let Some(number) = &number {
            new = new.number(number);
        }
        if let Some(date) = expires {
            new = new.expires_on(date);
        }
        if let Some(date) = issued {
            new = new.issued_on(date);
        }
        let document = match Documents::attach(&self.state, None, new).await? {
            Ok(document) => document,
            Err(refused) => {
                return Err(renox::anyhow::anyhow!(
                    "demo seed: the {kind} document was refused: {refused}"
                )
                .into());
            }
        };
        self.report.add("documents", 1);
        Ok(document.id)
    }

    /// The identity documents of tenant `name` that the lease check asks for:
    /// a person's Emirates ID, passport and residence visa; a company's trade
    /// licence and TRN certificate. Made once per party.
    pub(crate) async fn papers(&mut self, name: &str, papers: Papers) -> Result {
        if self.papered.contains(name) {
            return Ok(());
        }
        self.papered.insert(name.to_owned());
        let party = self.party(name).clone();
        let n = party_number(&party.code);
        let subject = Subject::new(PARTY_SUBJECT_TYPE, party.id);
        let known = super::people::PARTIES.iter().find(|p| p.en == name);
        let person = known.map(|p| p.who);
        let born = match person {
            Some(Who::Person(_, born)) => born,
            _ => 1985 + (n as i32 % 12),
        };
        let company = party.kind == wl_kernel::PartyKind::Company;
        if company {
            let licence = self
                .document(
                    subject.clone(),
                    "trade_licence",
                    "Trade licence",
                    Some(format!("DEMO-{:06}", 200 + n)),
                    Some(day(120 + (n as i64 * 29) % 240)),
                    Some(day(-245)),
                )
                .await;
            licence?;
            self.document(
                subject,
                "trn_certificate",
                "TRN certificate",
                Some(format!("100000000{:06}", 200 + n)),
                None,
                Some(day(-700)),
            )
            .await?;
            return Ok(());
        }
        let long = |step: i64| day(180 + (n as i64 * step) % 600);
        let id_expiry = match papers {
            Papers::IdExpiresIn(days) => day(days),
            _ => long(37),
        };
        self.document(
            subject.clone(),
            "emirates_id",
            "Emirates ID",
            Some(emirates_id(n, born)),
            Some(id_expiry),
            Some(day(-900)),
        )
        .await?;
        if papers != Papers::NoPassport {
            self.document(
                subject.clone(),
                "passport",
                "Passport",
                Some(passport(n)),
                Some(day(400 + (n as i64 * 53) % 1500)),
                Some(day(-1200)),
            )
            .await?;
        }
        let visa_expiry = match papers {
            Papers::VisaExpired(days) => day(-days),
            _ => long(23),
        };
        self.document(
            subject,
            "residence_visa",
            "Residence visa",
            Some(visa(n)),
            Some(visa_expiry),
            Some(day(-800)),
        )
        .await?;
        Ok(())
    }
}

/// The number of a party code, `P-00012` is 12.
pub fn party_number(code: &str) -> u32 {
    code.trim_start_matches("P-").parse().unwrap_or(0)
}
