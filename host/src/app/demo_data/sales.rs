//! The marketing side of the demo: campaigns with their costs, the leads they
//! brought in over 15 months, and the enquiries about units.

/// A campaign.
pub struct CampaignPlan {
    /// The code (also the link's `?c=`).
    pub code: &'static str,
    /// The name.
    pub name: &'static str,
    /// The marketing channel key.
    pub channel: &'static str,
    /// The lead source code the campaign's leads get.
    pub source: &'static str,
    /// The first day, in days from today.
    pub start: i64,
    /// The last day, in days from today.
    pub end: i64,
    /// The budget in dirhams.
    pub budget: i64,
    /// The costs entered, in dirhams, oldest first.
    pub costs: &'static [(&'static str, i64)],
    /// Where it stands: `draft`, `active` or `ended`.
    pub standing: &'static str,
}

/// The campaigns.
pub const CAMPAIGNS: &[CampaignPlan] = &[
    CampaignPlan {
        code: "RAMADAN26",
        name: "Ramadan move-in offer 2026",
        channel: "meta",
        source: "social",
        start: -250,
        end: -205,
        budget: 18_000,
        costs: &[
            ("Social media boosts", 6_450),
            ("Creative and copy", 3_200),
            ("Boosts, last two weeks", 5_900),
        ],
        standing: "ended",
    },
    CampaignPlan {
        code: "SUMSTUDIO26",
        name: "Summer studio offer, Tower B",
        channel: "google",
        source: "paid_ads",
        start: -130,
        end: -50,
        budget: 24_000,
        costs: &[
            ("Search ads, June", 7_800),
            ("Search ads, July", 8_150),
            ("Search ads, August", 7_900),
        ],
        standing: "ended",
    },
    CampaignPlan {
        code: "Q4TOWERS",
        name: "Q4 launch, Towers A and B",
        channel: "other_online",
        source: "website",
        start: -25,
        end: 70,
        budget: 30_000,
        costs: &[
            ("Listing portal packages", 9_600),
            ("Signage printing", 4_350),
            ("Portal top-up (over budget)", 18_500),
        ],
        standing: "active",
    },
    CampaignPlan {
        code: "BBOFFICE",
        name: "Business Bay office month",
        channel: "email",
        source: "partner",
        start: -12,
        end: 33,
        budget: 12_000,
        costs: &[("Mailing and design", 2_700)],
        standing: "active",
    },
    CampaignPlan {
        code: "WINTER26",
        name: "Winter relocation campaign",
        channel: "tiktok",
        source: "social",
        start: 40,
        end: 100,
        budget: 20_000,
        costs: &[],
        standing: "draft",
    },
    CampaignPlan {
        code: "REFER26",
        name: "Tenant referral rewards",
        channel: "other",
        source: "referral",
        start: 15,
        end: 195,
        budget: 15_000,
        costs: &[],
        standing: "draft",
    },
];

/// What became of a lead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeadFate {
    /// Not touched yet.
    New,
    /// Called.
    Contacted,
    /// Qualified.
    Qualified,
    /// Became this tenant's lease (the tenant's English name).
    Won(&'static str),
    /// Lost for this reason.
    Lost(&'static str),
    /// Lost, then worked again.
    Reopened,
}

/// A lead.
pub struct LeadPlan {
    /// The prospect.
    pub name: &'static str,
    /// The lead source.
    pub source: &'static str,
    /// The campaign code, if any.
    pub campaign: Option<&'static str>,
    /// What they are looking for.
    pub interest: &'static str,
    /// Days ago it came in.
    pub age: i64,
    /// What became of it.
    pub fate: LeadFate,
}

const fn lead(
    name: &'static str,
    source: &'static str,
    campaign: Option<&'static str>,
    interest: &'static str,
    age: i64,
    fate: LeadFate,
) -> LeadPlan {
    LeadPlan {
        name,
        source,
        campaign,
        interest,
        age,
        fate,
    }
}

use LeadFate::{Contacted, Lost, New, Qualified, Reopened, Won};

/// The leads, oldest first.
pub const LEADS: &[LeadPlan] = &[
    lead(
        "Zayed Al Marri",
        "social",
        Some("RAMADAN26"),
        "1BR in Jumeirah Village, up to AED 60,000",
        262,
        Won("Camille Laurent"),
    ),
    lead(
        "Priyanka Shah",
        "social",
        Some("RAMADAN26"),
        "Studio near the metro",
        255,
        Lost("price"),
    ),
    lead(
        "Rashid Iqbal",
        "social",
        Some("RAMADAN26"),
        "2BR for a family of four",
        249,
        Lost("no_response"),
    ),
    lead(
        "Hessa Al Ketbi",
        "walk_in",
        None,
        "Villa or large apartment, Abu Dhabi",
        240,
        Lost("chose_competitor"),
    ),
    lead(
        "Marcus Lindqvist",
        "website",
        None,
        "1BR furnished, 12 months",
        233,
        Lost("not_interested"),
    ),
    lead(
        "Aliya Karimova",
        "referral",
        None,
        "2BR, pets allowed",
        221,
        Qualified,
    ),
    lead(
        "Tariq Mahmood",
        "social",
        Some("RAMADAN26"),
        "Studio, short notice",
        214,
        Lost("duplicate"),
    ),
    lead(
        "Nadine Fakhoury",
        "event",
        None,
        "Office for six people, Business Bay",
        205,
        Contacted,
    ),
    lead(
        "Sergei Volkov",
        "website",
        None,
        "Warehouse bay, Al Quoz, 5,000 sq ft",
        197,
        Won("Meridian Logistics LLC"),
    ),
    lead(
        "Fatima Al Zaabi",
        "referral",
        None,
        "2BR, Sharjah, family",
        188,
        Lost("price"),
    ),
    lead(
        "Joseph Mathew",
        "paid_ads",
        None,
        "1BR, ready this month",
        176,
        Contacted,
    ),
    lead(
        "Leila Nasrallah",
        "partner",
        None,
        "Clinic space, ground floor",
        169,
        Lost("chose_competitor"),
    ),
    lead(
        "Wei Zhang",
        "paid_ads",
        Some("SUMSTUDIO26"),
        "Studio, Tower B",
        131,
        Won("Sunita Rao"),
    ),
    lead(
        "Rosa Delgado",
        "paid_ads",
        Some("SUMSTUDIO26"),
        "Studio, Tower B",
        126,
        Lost("no_response"),
    ),
    lead(
        "Hamad Al Nuaimi",
        "paid_ads",
        Some("SUMSTUDIO26"),
        "1BR, Tower B",
        120,
        Qualified,
    ),
    lead(
        "Bianca Rossi",
        "paid_ads",
        Some("SUMSTUDIO26"),
        "Studio, Tower B",
        118,
        Lost("spam_or_invalid"),
    ),
    lead(
        "Omar Khalil",
        "paid_ads",
        Some("SUMSTUDIO26"),
        "1BR, Jumeirah Village",
        109,
        Contacted,
    ),
    lead(
        "Preeti Nambiar",
        "paid_ads",
        Some("SUMSTUDIO26"),
        "Studio, furnished",
        101,
        Reopened,
    ),
    lead(
        "Khalfan Al Dhaheri",
        "walk_in",
        None,
        "3BR in Sharjah",
        96,
        Won("Ahmed Nabil Khalifa"),
    ),
    lead(
        "Ana Costa",
        "website",
        None,
        "1BR, quiet floor",
        88,
        Qualified,
    ),
    lead(
        "Idris Bello",
        "referral",
        None,
        "2BR near the school",
        79,
        Contacted,
    ),
    lead(
        "Mona Saad",
        "paid_ads",
        Some("SUMSTUDIO26"),
        "Studio, Tower B",
        66,
        Lost("price"),
    ),
    lead(
        "Vikram Chawla",
        "website",
        None,
        "Office, Business Bay, 1,200 sq ft",
        58,
        Qualified,
    ),
    lead(
        "Hind Al Mansoori",
        "social",
        None,
        "2BR, Abu Dhabi, Reem Island",
        49,
        Contacted,
    ),
    lead(
        "Julian Hart",
        "website",
        None,
        "1BR, Tower A, high floor",
        42,
        Lost("other"),
    ),
    lead(
        "Amira Youssef",
        "referral",
        None,
        "2BR, Tower A",
        34,
        Qualified,
    ),
    lead(
        "Dmitri Sokolov",
        "website",
        Some("Q4TOWERS"),
        "1BR, Tower A",
        24,
        Contacted,
    ),
    lead(
        "Shaikha Al Ameri",
        "website",
        Some("Q4TOWERS"),
        "2BR, Tower B",
        22,
        Qualified,
    ),
    lead(
        "Rohan Mehta",
        "website",
        Some("Q4TOWERS"),
        "Studio, Tower B",
        19,
        New,
    ),
    lead(
        "Layan Haddad",
        "website",
        Some("Q4TOWERS"),
        "1BR, Tower A",
        17,
        Contacted,
    ),
    lead(
        "Ethan Brooks",
        "website",
        Some("Q4TOWERS"),
        "2BR, Tower A",
        15,
        Won("Angelica Reyes"),
    ),
    lead(
        "Sana Ullah",
        "website",
        Some("Q4TOWERS"),
        "Studio, Tower B",
        13,
        New,
    ),
    lead(
        "Karim Fouad",
        "partner",
        Some("BBOFFICE"),
        "Office for 10, Business Bay",
        11,
        Contacted,
    ),
    lead(
        "Noor Al Hashimi",
        "partner",
        Some("BBOFFICE"),
        "Shop, ground floor",
        9,
        New,
    ),
    lead(
        "Gregor Hoffmann",
        "partner",
        Some("BBOFFICE"),
        "Office, 1,500 sq ft",
        8,
        Qualified,
    ),
    lead(
        "Amal Said",
        "website",
        Some("Q4TOWERS"),
        "1BR, Tower B",
        6,
        New,
    ),
    lead("Tessa Brown", "social", None, "1BR furnished", 4, New),
    lead("Faris Al Qasimi", "walk_in", None, "2BR, Sharjah", 3, New),
    lead(
        "Yasmin Haddad",
        "website",
        Some("Q4TOWERS"),
        "Studio",
        2,
        New,
    ),
    lead(
        "Rahul Verma",
        "website",
        Some("Q4TOWERS"),
        "1BR, Tower A",
        1,
        New,
    ),
    lead(
        "Hana Al Suwaidi",
        "referral",
        None,
        "2BR, Abu Dhabi",
        0,
        New,
    ),
];

/// An enquiry about a unit, recorded by staff.
pub struct EnquiryPlan {
    /// Who asked.
    pub name: &'static str,
    /// Their phone or e-mail.
    pub contact: &'static str,
    /// The unit asked about: building and number.
    pub unit: Option<(&'static str, &'static str)>,
    /// The source.
    pub source: &'static str,
    /// The campaign code.
    pub campaign: Option<&'static str>,
    /// Days ago.
    pub age: i64,
    /// The note.
    pub note: &'static str,
    /// The tenant it became, if it did.
    pub converted: Option<&'static str>,
}

/// The enquiries.
pub const ENQUIRIES: &[EnquiryPlan] = &[
    EnquiryPlan {
        name: "Ethan Brooks",
        contact: "+971500000401",
        unit: Some(("JVT-A", "402")),
        source: "website",
        campaign: Some("Q4TOWERS"),
        age: 15,
        note: "Viewed on Saturday, wants to move in at the start of the month",
        converted: Some("Angelica Reyes"),
    },
    EnquiryPlan {
        name: "Wei Zhang",
        contact: "wei.zhang@mail.example",
        unit: Some(("SHJ", "102")),
        source: "paid_ads",
        campaign: Some("SUMSTUDIO26"),
        age: 125,
        note: "Asked for a monthly payment plan",
        converted: Some("Sunita Rao"),
    },
    EnquiryPlan {
        name: "Khalfan Al Dhaheri",
        contact: "+971500000402",
        unit: Some(("SHJ", "201")),
        source: "walk_in",
        campaign: None,
        age: 94,
        note: "Came to the building office with his family",
        converted: Some("Ahmed Nabil Khalifa"),
    },
    EnquiryPlan {
        name: "Rohan Mehta",
        contact: "rohan.mehta@mail.example",
        unit: Some(("JVT-B", "602")),
        source: "website",
        campaign: Some("Q4TOWERS"),
        age: 19,
        note: "Wants a studio on a high floor",
        converted: None,
    },
    EnquiryPlan {
        name: "Karim Fouad",
        contact: "+971500000403",
        unit: Some(("BBY", "601")),
        source: "partner",
        campaign: Some("BBOFFICE"),
        age: 11,
        note: "Needs space for ten desks and a meeting room",
        converted: None,
    },
    EnquiryPlan {
        name: "Noor Al Hashimi",
        contact: "+971500000404",
        unit: Some(("BBY", "G01")),
        source: "partner",
        campaign: Some("BBOFFICE"),
        age: 9,
        note: "Interested in a cafe unit",
        converted: None,
    },
    EnquiryPlan {
        name: "Amal Said",
        contact: "amal.said@mail.example",
        unit: Some(("JVT-A", "602")),
        source: "website",
        campaign: Some("Q4TOWERS"),
        age: 6,
        note: "Asked about parking",
        converted: None,
    },
    EnquiryPlan {
        name: "Tessa Brown",
        contact: "+971500000405",
        unit: Some(("JVT-A", "602")),
        source: "social",
        campaign: None,
        age: 4,
        note: "Found it on Instagram",
        converted: None,
    },
    EnquiryPlan {
        name: "Faris Al Qasimi",
        contact: "+971500000406",
        unit: Some(("SHJ", "301")),
        source: "walk_in",
        campaign: None,
        age: 3,
        note: "Wants to see the unit tomorrow",
        converted: None,
    },
    EnquiryPlan {
        name: "Hana Al Suwaidi",
        contact: "hana.alsuwaidi@mail.example",
        unit: None,
        source: "referral",
        campaign: None,
        age: 0,
        note: "Referred by a tenant of Tower A",
        converted: None,
    },
];
