//! The demo company's people: the staff logins (one per role) and the parties
//! (clients, suppliers and subcontractors) the demo adds to the ones the
//! shared modules seed.
//!
//! Everything is fictional and written out here, so every seed gives the same
//! people. Identifiers use the demo formats of DEMO-DATA.md 1.2: Emirates ID
//! `784-YYYY-0000NNN-C`, passport `X0000NNN`, phone `+971500000NNN`, e-mail on
//! `.example`, TRN `100000000NNNNNN`, trade licence `DEMO-NNNNNN`.

use crate::app::roles::{
    ACCOUNTANT, ADMIN, COMPLIANCE, HR, HSE_OFFICER, PROCUREMENT, PROJECT_MANAGER,
    QUANTITY_SURVEYOR, RECEIVABLES, STOREKEEPER, SUPERVISOR,
};

/// One staff login.
#[derive(Debug, Clone, Copy)]
pub struct Login {
    /// The name shown on the login page.
    pub name: &'static str,
    /// The Arabic name.
    pub ar: &'static str,
    /// The sign-in e-mail.
    pub email: &'static str,
    /// The role.
    pub role: &'static str,
    /// What it is for, shown beside the name on the login page.
    pub label: &'static str,
}

/// The staff logins, one per role, all with the password of
/// `crate::app::seed::PASSWORD`. The MD is the company admin.
pub const LOGINS: &[Login] = &[
    Login {
        name: "Faisal Al Mansoori",
        ar: "فيصل المنصوري",
        email: "admin@alwaha.example",
        role: ADMIN,
        label: "Managing director (company admin)",
    },
    Login {
        name: "Omar Haddad",
        ar: "عمر حداد",
        email: "project_manager@alwaha.example",
        role: PROJECT_MANAGER,
        label: "Project manager",
    },
    Login {
        name: "Anitha Krishnan",
        ar: "أنيثا كريشنان",
        email: "quantity_surveyor@alwaha.example",
        role: QUANTITY_SURVEYOR,
        label: "Quantity surveyor",
    },
    Login {
        name: "Ramon Dela Cruz",
        ar: "رامون دي لا كروز",
        email: "supervisor@alwaha.example",
        role: SUPERVISOR,
        label: "Site supervisor",
    },
    Login {
        name: "Tariq Mahmood",
        ar: "طارق محمود",
        email: "storekeeper@alwaha.example",
        role: STOREKEEPER,
        label: "Storekeeper",
    },
    Login {
        name: "Hessa Al Neyadi",
        ar: "حصة النيادي",
        email: "procurement@alwaha.example",
        role: PROCUREMENT,
        label: "Procurement officer",
    },
    Login {
        name: "Joseph Thomas",
        ar: "جوزيف توماس",
        email: "accountant@alwaha.example",
        role: ACCOUNTANT,
        label: "Accountant",
    },
    Login {
        name: "Mona Fakhoury",
        ar: "منى فاخوري",
        email: "receivables@alwaha.example",
        role: RECEIVABLES,
        label: "Receivables",
    },
    Login {
        name: "Divya Menon",
        ar: "ديفيا مينون",
        email: "hr@alwaha.example",
        role: HR,
        label: "HR and PRO",
    },
    Login {
        name: "Gareth Whitfield",
        ar: "غاريث ويتفيلد",
        email: "hse@alwaha.example",
        role: HSE_OFFICER,
        label: "HSE officer",
    },
    Login {
        name: "Salem Al Dhaheri",
        ar: "سالم الظاهري",
        email: "compliance@alwaha.example",
        role: COMPLIANCE,
        label: "Compliance officer",
    },
];

/// The login with this role.
///
/// # Panics
///
/// When no demo login has the role.
pub fn login_of(role: &str) -> &'static Login {
    LOGINS
        .iter()
        .find(|login| login.role == role)
        .unwrap_or_else(|| panic!("a demo login for {role}"))
}

/// The e-mail of the login with this role.
pub fn email_of(role: &str) -> &'static str {
    login_of(role).email
}

/// What a party is to the company.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A client or main contractor.
    Client,
    /// A material supplier.
    Supplier,
    /// A subcontractor (a supplier too: it also invoices the company).
    Subcontractor,
}

impl Kind {
    /// The party roles it holds.
    pub fn roles(self) -> &'static [&'static str] {
        match self {
            Kind::Client => &["client"],
            Kind::Supplier => &["supplier"],
            Kind::Subcontractor => &["supplier", "subcontractor"],
        }
    }
}

/// One company the demo adds. The number is the demo's own (for phone
/// numbers, identifiers and licences), from 300.
#[derive(Debug, Clone, Copy)]
pub struct Firm {
    /// The number.
    pub n: u32,
    /// The English name (also the key the rest of the demo finds it by).
    pub en: &'static str,
    /// The Arabic name.
    pub ar: &'static str,
    /// What it is to the company.
    pub kind: Kind,
    /// The e-mail local part and domain.
    pub mail: &'static str,
}

/// The clients, in the order they are created.
pub const GULF_HORIZON: &str = "Gulf Horizon Developments LLC";
/// The retail client of the mall and the restaurant.
pub const FESTIVAL_PLAZA: &str = "Festival Plaza Retail Holdings LLC";
/// The healthcare client, blocked by compliance.
pub const AL_NOOR: &str = "Al Noor Healthcare Group LLC";
/// The board supplier.
pub const GYPSUM: &str = "Emirates Gypsum & Building Materials Trading LLC";
/// The electrical materials supplier.
pub const ELECTRICAL: &str = "Desert Star Electrical Materials LLC";
/// The Dubai subcontractor with a valid Dubai Municipality registration.
pub const MEP: &str = "Marina MEP Contracting LLC";
/// The Dubai subcontractor whose registration expired.
pub const GLASS: &str = "Al Qusais Aluminium & Glass LLC";
/// The paints supplier.
pub const PAINTS: &str = "Jebel Ali Paints & Coatings FZE";

/// The companies the demo adds.
pub const FIRMS: &[Firm] = &[
    Firm {
        n: 301,
        en: GULF_HORIZON,
        ar: "غلف هورايزن للتطوير ذ.م.م",
        kind: Kind::Client,
        mail: "projects@gulfhorizon",
    },
    Firm {
        n: 302,
        en: FESTIVAL_PLAZA,
        ar: "فستيفال بلازا للتجزئة القابضة ذ.م.م",
        kind: Kind::Client,
        mail: "fitout@festivalplaza",
    },
    Firm {
        n: 303,
        en: AL_NOOR,
        ar: "مجموعة النور للرعاية الصحية ذ.م.م",
        kind: Kind::Client,
        mail: "facilities@alnoorhealth",
    },
    Firm {
        n: 311,
        en: GYPSUM,
        ar: "الإمارات للجبس واللوحات للتجارة ذ.م.م",
        kind: Kind::Supplier,
        mail: "sales@emiratesgypsum",
    },
    Firm {
        n: 312,
        en: ELECTRICAL,
        ar: "ديزرت ستار للمواد الكهربائية ذ.م.م",
        kind: Kind::Supplier,
        mail: "orders@desertstar",
    },
    Firm {
        n: 313,
        en: MEP,
        ar: "مارينا للمقاولات الكهروميكانيكية ذ.م.م",
        kind: Kind::Subcontractor,
        mail: "accounts@marinamep",
    },
    Firm {
        n: 314,
        en: GLASS,
        ar: "القصيص للألمنيوم والزجاج ذ.م.م",
        kind: Kind::Subcontractor,
        mail: "office@qusaisglass",
    },
    Firm {
        n: 315,
        en: PAINTS,
        ar: "جبل علي للدهانات والطلاء م.ح.",
        kind: Kind::Supplier,
        mail: "sales@jebelalipaints",
    },
];
