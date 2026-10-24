//! The customers of the demo company, their credit accounts, the invoice that
//! is overdue and the sales orders the work follows.

use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, VatCategory, invoicer};
use wl_kernel::{PartyId, PartyKind, Subject};
use wl_parties::{
    Channel, CompanyDetails, ContactPoint, ContactType, Language, NewParty, PartyRole,
    PersonDetails,
};
use wl_portal::{AccessGrant, GrantStatus};

use super::{ACCOUNTANT_EMAIL, ADMIN_EMAIL, BUYER_EMAIL, SALES_EMAIL, Seed, day, fixture};

/// A customer: key, name, Arabic name, e-mail slug, credit limit in AED,
/// payment terms.
type CustomerRow = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
);

const CUSTOMERS: &[CustomerRow] = &[
    (
        "noor",
        "Al Noor Trading LLC",
        "شركة النور للتجارة",
        "alnoor",
        "250000",
        "30",
    ),
    (
        "fresh",
        "Gulf Fresh Foods LLC",
        "شركة الخليج للأغذية الطازجة",
        "gulffresh",
        "400000",
        "45",
    ),
    (
        "eph",
        "Emirates Print House LLC",
        "دار الإمارات للطباعة",
        "emiratesprint",
        "120000",
        "30",
    ),
    (
        "sahara",
        "Sahara Retail LLC",
        "شركة الصحراء للتجزئة",
        "saharaeven",
        "80000",
        "30",
    ),
    (
        "atlas",
        "Atlas Gulf Export FZE",
        "أطلس الخليج للتصدير",
        "atlasgulf",
        "600000",
        "60",
    ),
];

impl Seed {
    async fn company(&self, name: &str, name_ar: &str, slug: &str) -> Result<PartyId> {
        let contact = |contact_type, value: String| ContactPoint {
            contact_type,
            value,
            is_primary: true,
            ..Default::default()
        };
        Ok(wl_parties::create(
            &self.state,
            NewParty {
                kind: PartyKind::Company,
                legal_name_en: name.to_owned(),
                legal_name_ar: Some(name_ar.to_owned()),
                company: Some(CompanyDetails::default()),
                roles: vec![PartyRole {
                    role_key: "customer".to_owned(),
                    ..PartyRole::default()
                }],
                default_language: Language::En,
                preferred_channel: Channel::Email,
                contacts: vec![
                    contact(ContactType::Email, format!("purchasing@{slug}.example")),
                    contact(ContactType::Mobile, "+97144000000".to_owned()),
                ],
                source: "seed".to_owned(),
                ..Default::default()
            },
        )
        .await?
        .party_id())
    }

    /// The customers with their credit accounts (sales manager), and the
    /// buyer of Al Noor in the portal.
    pub(crate) async fn customers(&mut self) -> Result {
        for (key, name, name_ar, slug, limit, terms) in CUSTOMERS {
            let party = self.company(name, name_ar, slug).await?;
            self.driver
                .post(
                    ADMIN_EMAIL,
                    "/credit/accounts",
                    &[
                        ("party_id", &party.0.to_string()),
                        ("credit_limit", limit),
                        ("payment_terms_days", terms),
                    ],
                )
                .await?;
            self.customers.insert(key, party);
        }
        let noor = self.customers["noor"];
        let nadia = wl_parties::create(
            &self.state,
            NewParty {
                kind: PartyKind::Person,
                legal_name_en: "Nadia Al Noor".to_owned(),
                person: Some(PersonDetails::default()),
                default_language: Language::En,
                preferred_channel: Channel::Email,
                contacts: vec![ContactPoint {
                    contact_type: ContactType::Email,
                    value: BUYER_EMAIL.to_owned(),
                    is_primary: true,
                    ..Default::default()
                }],
                source: "seed".to_owned(),
                ..Default::default()
            },
        )
        .await?
        .party_id();
        let user = self.user(BUYER_EMAIL).await?;
        fixture!(
            self.db(),
            AccessGrant {
                user_id: Some(user.id),
                person_party_id: nadia.0,
                party_id: noor.0,
                audience: "customer".into(),
                roles: "buyer".into(),
                source: "invitation".into(),
                valid_from: day(-120),
                status: GrantStatus::Active,
                ..AccessGrant::default()
            },
            GrantStatus::Active
        );
        Ok(())
    }

    /// An invoice for `amount` fils (VAT included in the line), due `due`.
    pub(crate) async fn invoice(
        &self,
        customer: PartyId,
        what: &str,
        amount: i64,
        vat: VatCategory,
        due: i64,
        source: Subject,
    ) -> Result<i64> {
        let draft = InvoiceDraft::new(customer, source.clone())
            .line(DraftLine::new(what, amount, vat).origin(source))
            .due_date(day(due));
        let id = invoicer(&self.state)
            .draft(self.db(), draft)
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("invoicing is off"))?;
        self.driver
            .post(ACCOUNTANT_EMAIL, &format!("/invoices/{}/issue", id.0), &[])
            .await?;
        Ok(id.0)
    }

    /// The receivables: Sahara's September invoice is 31 days overdue, so its
    /// next order is held; Al Noor has an open one.
    pub(crate) async fn money(&mut self) -> Result {
        let sahara = self.customers["sahara"];
        let noor = self.customers["noor"];
        self.invoice(
            sahara,
            "Corrugated packaging, September",
            4_200_000,
            VatCategory::Standard,
            -31,
            Subject::new("manufacturing_demo.opening", 1),
        )
        .await?;
        self.invoice(
            noor,
            "Corrugated boxes, October",
            1_890_000,
            VatCategory::Standard,
            12,
            Subject::new("manufacturing_demo.opening", 2),
        )
        .await?;
        wl_invoicing::overdue::run(&self.state).await?;
        Ok(())
    }

    /// One order on account with `lines` (code, quantity, price in AED),
    /// entered by the sales person; the id of the order and of its first line.
    async fn order(&self, customer: &str, lines: &[(&str, &str, &str)]) -> Result<(i64, i64)> {
        let party = self.customers[customer];
        self.driver
            .post(
                SALES_EMAIL,
                "/sales-orders",
                &[
                    ("party_id", &party.0.to_string()),
                    ("channel", "phone"),
                    ("payment_mode", "on_account"),
                ],
            )
            .await?;
        let order = wl_sales::SalesOrder::query()
            .where_(wl_sales::SalesOrder::PARTY_ID.eq(party.0))
            .order_by_desc(wl_sales::SalesOrder::ID)
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the order of {customer}"))?;
        for (code, quantity, price) in lines {
            self.driver
                .post(
                    SALES_EMAIL,
                    &format!("/sales-orders/{}/lines", order.id),
                    &[
                        ("item_code", code),
                        ("quantity", quantity),
                        ("unit_price", price),
                        ("vat", "standard"),
                    ],
                )
                .await?;
        }
        self.driver
            .post(
                SALES_EMAIL,
                &format!("/sales-orders/{}/submit", order.id),
                &[("x", "")],
            )
            .await?;
        let order = wl_sales::SalesOrder::find_or_404(self.db(), order.id).await?;
        if order.status == wl_sales::SalesOrderStatus::PendingAcceptance {
            self.driver
                .post(
                    SALES_EMAIL,
                    &format!("/sales-orders/{}/transitions/accept", order.id),
                    &[("x", "")],
                )
                .await?;
        }
        let line = wl_sales::SaleLine::of(self.db(), &order.subject())
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the lines of {customer}"))?;
        Ok((order.id, line.id))
    }

    /// The sales orders the production follows.
    pub(crate) async fn orders(&mut self) -> Result {
        for (key, customer, lines) in [
            (
                "noor",
                "noor",
                vec![("BOX-FOOD", "1000", "1.80"), ("BOX-RTL", "500", "2.40")],
            ),
            ("fresh", "fresh", vec![("BOX-FOOD", "2400", "1.80")]),
            ("eph", "eph", vec![("SLV-EPH", "50000", "0.08")]),
            ("sahara", "sahara", vec![("BOX-RTL", "800", "2.40")]),
            ("atlas", "atlas", vec![("BOX-RTL", "600", "2.40")]),
        ] {
            let made = self.order(customer, &lines).await?;
            self.orders.insert(key, made);
        }
        Ok(())
    }
}
