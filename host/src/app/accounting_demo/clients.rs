//! The firm's client companies with their UBOs and directors, the KYC
//! documents on file, the sanctions screenings (current lists, every party
//! clear except one re-screening hit the MLRO has not decided) and the
//! client people in the portal (Marina Bay Trading's owner and finance
//! contact, Desert Rose's owner).

use renox::prelude::*;
use wl_documents::{Document, DocumentKind, DocumentStatus};
use wl_kernel::{PartyId, PartyKind};
use wl_parties::{
    Channel, CompanyDetails, ContactPoint, ContactType, Language, ListStatus, ListUsed,
    NewCandidate, NewParty, Party, PartyRole, PersonDetails, SanctionsList, ScreeningRun,
    ScreeningTrigger, record_run,
};

use super::{CLIENT_FINANCE_EMAIL, CLIENT_OWNER_EMAIL, Client, ROSE_OWNER_EMAIL, Seed, day};

/// A client: key, name, Arabic name, e-mail slug, UBO, UBO's Arabic name,
/// director.
type ClientRow = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
);

pub(crate) const CLIENTS: &[ClientRow] = &[
    (
        "marina",
        "Marina Bay Trading LLC",
        "شركة مارينا باي للتجارة",
        "marinabay",
        "Tariq Al Marri",
        "طارق المري",
        "Sunil Varghese",
    ),
    (
        "rose",
        "Desert Rose Interiors LLC",
        "شركة وردة الصحراء للديكور",
        "desertrose",
        "Layla Haddad",
        "ليلى حداد",
        "Marco Bianchi",
    ),
    (
        "oasis",
        "Oasis Fresh Foods LLC",
        "شركة الواحة للأغذية الطازجة",
        "oasisfresh",
        "Ahmed Al Suwaidi",
        "أحمد السويدي",
        "Rina Santos",
    ),
    (
        "waha",
        "Al Waha General Trading FZE",
        "الواحة للتجارة العامة م.م.ح",
        "alwaha",
        "Viktor Petrenko",
        "فيكتور بيترينكو",
        "Imran Qureshi",
    ),
    (
        "palm",
        "Palm Tech Solutions LLC",
        "شركة بالم للحلول التقنية",
        "palmtech",
        "Nour Khalil",
        "نور خليل",
        "Arjun Nair",
    ),
    (
        "dune",
        "Sand Dune Contracting LLC",
        "شركة الكثبان الرملية للمقاولات",
        "sanddune",
        "Hamad Al Ketbi",
        "حمد الكتبي",
        "Ravi Shankar",
    ),
    (
        "coral",
        "Coral Reef Restaurants LLC",
        "شركة الشعاب المرجانية للمطاعم",
        "coralreef",
        "Elena Popescu",
        "إيلينا بوبيسكو",
        "Samir Aziz",
    ),
    (
        "pearl",
        "Pearl Fashion House LLC",
        "دار اللؤلؤ للأزياء",
        "pearlfashion",
        "Mariam Al Falasi",
        "مريم الفلاسي",
        "Chen Wei",
    ),
    (
        "horizon",
        "Gulf Horizon Logistics LLC",
        "شركة أفق الخليج للخدمات اللوجستية",
        "gulfhorizon",
        "Omar Farouk",
        "عمر فاروق",
        "Daniel Mensah",
    ),
];

impl Seed {
    async fn party(
        &self,
        kind: PartyKind,
        name: &str,
        name_ar: Option<&str>,
        email: String,
        phone: usize,
    ) -> Result<PartyId> {
        let contact = |contact_type, value: String| ContactPoint {
            contact_type,
            value,
            is_primary: true,
            ..Default::default()
        };
        let company = kind == PartyKind::Company;
        Ok(wl_parties::create(
            &self.state,
            NewParty {
                kind,
                legal_name_en: name.to_owned(),
                legal_name_ar: name_ar.map(str::to_owned),
                company: company.then(CompanyDetails::default),
                person: (!company).then(PersonDetails::default),
                roles: if company {
                    vec![PartyRole {
                        role_key: "customer".to_owned(),
                        ..PartyRole::default()
                    }]
                } else {
                    Vec::new()
                },
                default_language: Language::En,
                preferred_channel: Channel::Email,
                contacts: vec![
                    contact(ContactType::Email, email),
                    contact(ContactType::Mobile, format!("+97150000{phone:04}")),
                ],
                source: "seed".to_owned(),
                ..Default::default()
            },
        )
        .await?
        .party_id())
    }

    /// An accepted document of kind `key` about `party`, valid through
    /// `expires`.
    async fn kyc_document(
        &self,
        party: PartyId,
        key: &str,
        title: &str,
        expires: renox::chrono::NaiveDate,
    ) -> Result {
        let Some(kind) = DocumentKind::by_key(self.db(), key).await? else {
            return Ok(());
        };
        Document::create(
            self.db(),
            Document {
                subject_type: wl_kernel::parties::PARTY_SUBJECT_TYPE.into(),
                subject_id: party.0,
                kind_id: kind.id,
                title: title.into(),
                expiry_date: Some(expires),
                current_version: 1,
                status: DocumentStatus::Accepted,
                ..Default::default()
            },
        )
        .await?;
        Ok(())
    }

    /// The lists as a screening now records them.
    async fn lists_used(&self) -> Result<Vec<ListUsed>> {
        Ok(SanctionsList::all(self.db())
            .await?
            .into_iter()
            .map(|list| ListUsed {
                list_id: list.id,
                name: list.name.clone(),
                version: list.version.clone(),
                last_updated_at: Some(renox::db::now()),
                stale: false,
            })
            .collect())
    }

    /// A screening of `party` on current lists, with `candidates`.
    pub(crate) async fn screen(
        &self,
        party: PartyId,
        trigger: ScreeningTrigger,
        candidates: Vec<NewCandidate>,
    ) -> Result {
        let mut run = ScreeningRun::automatic(trigger, candidates);
        run.lists_used = self.lists_used().await?;
        let row = Party::find(self.db(), party.0)
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the party {}", party.0))?;
        record_run(&self.state, &row, run, None)
            .await
            .map_err(|e| renox::anyhow::anyhow!("demo seed: the screening failed: {e:?}"))?;
        Ok(())
    }

    /// The re-screening hit on Al Waha's UBO the MLRO has not decided yet
    /// (fictional list entry).
    pub(crate) async fn rescreening_hit(&self) -> Result {
        let ubo = self.clients["waha"].ubo;
        let list = SanctionsList::all(self.db())
            .await?
            .into_iter()
            .next()
            .map(|l| l.name)
            .unwrap_or_else(|| "UN Consolidated List".to_owned());
        self.screen(
            ubo,
            ScreeningTrigger::ListUpdate,
            vec![NewCandidate {
                name: "Viktor A. Petrenko".to_owned(),
                list_name: list,
                score: 0.91,
                list_entry: Some("DEMO.0042".to_owned()),
            }],
        )
        .await
    }

    /// The clients, their people, documents and screenings, and the portal
    /// users of Marina Bay Trading.
    pub(crate) async fn clients(&mut self) -> Result {
        // The lists were updated this morning.
        for mut list in SanctionsList::all(self.db()).await? {
            list.last_updated_at = Some(renox::db::now());
            list.status = ListStatus::Current;
            list.stale_since = None;
            list.last_error = None;
            list.save(self.db()).await?;
        }
        let mut phone = 100;
        for (key, name, name_ar, slug, ubo, ubo_ar, director) in CLIENTS {
            phone += 3;
            let company = self
                .party(
                    PartyKind::Company,
                    name,
                    Some(name_ar),
                    format!("accounts@{slug}.example"),
                    phone,
                )
                .await?;
            let ubo_email = if *key == "marina" {
                CLIENT_OWNER_EMAIL.to_owned()
            } else if *key == "rose" {
                ROSE_OWNER_EMAIL.to_owned()
            } else {
                format!("{}@{slug}.example", first_name(ubo))
            };
            let ubo = self
                .party(PartyKind::Person, ubo, Some(ubo_ar), ubo_email, phone + 1)
                .await?;
            let director = self
                .party(
                    PartyKind::Person,
                    director,
                    None,
                    format!("{}@{slug}.example", first_name(director)),
                    phone + 2,
                )
                .await?;
            // The KYC documents: the trade licence of Desert Rose expires in
            // three weeks (the MLRO's reminder), the rest in a year or more.
            let licence_expiry = if *key == "rose" { day(21) } else { day(290) };
            self.kyc_document(company, "trade_licence", "Trade licence", licence_expiry)
                .await?;
            self.kyc_document(ubo, "passport", "Passport", day(800))
                .await?;
            self.kyc_document(ubo, "emirates_id", "Emirates ID", day(500))
                .await?;
            for party in [company, ubo, director] {
                self.screen(party, ScreeningTrigger::Onboarding, Vec::new())
                    .await?;
            }
            self.clients.insert(
                key,
                Client {
                    company,
                    ubo,
                    director,
                },
            );
        }

        // Marina Bay's owner approves returns; its finance contact reads.
        let marina = self.clients["marina"];
        let owner = self.user(CLIENT_OWNER_EMAIL).await?;
        wl_portal::PortalAccess::register(
            &self.state,
            owner.id,
            wl_portal::NewGrant::new(marina.ubo, marina.company, "client", &["owner"]),
        )
        .await?;
        let rose = self.clients["rose"];
        let layla = self.user(ROSE_OWNER_EMAIL).await?;
        wl_portal::PortalAccess::register(
            &self.state,
            layla.id,
            wl_portal::NewGrant::new(rose.ubo, rose.company, "client", &["owner"]),
        )
        .await?;
        let finance_person = self
            .party(
                PartyKind::Person,
                "Leena Joseph",
                None,
                CLIENT_FINANCE_EMAIL.to_owned(),
                90,
            )
            .await?;
        let finance = self.user(CLIENT_FINANCE_EMAIL).await?;
        wl_portal::PortalAccess::register(
            &self.state,
            finance.id,
            wl_portal::NewGrant::new(finance_person, marina.company, "client", &["finance"]),
        )
        .await?;
        Ok(())
    }
}

/// The lower-case first name of `name`, for an e-mail.
fn first_name(name: &str) -> String {
    name.split_whitespace()
        .next()
        .unwrap_or(name)
        .to_lowercase()
}
