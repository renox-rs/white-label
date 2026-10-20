//! The demo company's people: the staff logins (one per role, scoped to
//! buildings where the role is scoped) and the parties (tenants, owners,
//! vendors) the demo adds to the ones the shared modules seed.
//!
//! Everything is fictional and written out here, so every seed gives the same
//! people. Identifiers use the demo formats of DEMO-DATA.md 1.2: Emirates ID
//! `784-YYYY-0000NNN-C`, passport `X0000NNN`, phone `+971500000NNN`, e-mail
//! on `.example`, TRN `100000000NNNNNN`, trade licence `DEMO-NNNNNN`.

use wl_parties::Language;

use crate::app::roles::{
    ACCOUNTANT, ADMIN, COMPLIANCE, COORDINATOR, FACILITY_MANAGER, FINANCE_MANAGER, HR,
    LEASING_AGENT, MARKETING, MARKETING_MANAGER, OWNER, PROPERTY_MANAGER, RECEIVABLES, TECHNICIAN,
};

/// Where a login works: everywhere, or in some buildings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// Every building.
    Global,
    /// The buildings with these codes.
    Buildings(&'static [&'static str]),
}

/// One staff login.
#[derive(Debug, Clone, Copy)]
pub struct Login {
    /// The name shown on the login page.
    pub name: &'static str,
    /// The sign-in e-mail.
    pub email: &'static str,
    /// The role.
    pub role: &'static str,
    /// What it is for, shown beside the name on the login page.
    pub label: &'static str,
    /// The buildings it works in.
    pub reach: Reach,
}

/// The buildings of the two towers.
pub const TOWERS: &[&str] = &["JVT-A", "JVT-B"];
/// The Business Bay and Al Quoz buildings.
pub const BAY: &[&str] = &["BBY", "AQZ"];
/// The Abu Dhabi and Sharjah buildings.
pub const NORTH: &[&str] = &["ADR", "SHJ"];

/// The staff logins, one per role (the property manager has one per set of
/// buildings), all with the password of `super::seed::PASSWORD`.
pub const LOGINS: &[Login] = &[
    Login {
        name: "Layla Hassan",
        email: "admin@palmgrove.example",
        role: ADMIN,
        label: "Company admin",
        reach: Reach::Global,
    },
    Login {
        name: "Rashid Al Mulla",
        email: "property_manager@palmgrove.example",
        role: PROPERTY_MANAGER,
        label: "Property manager, Tower A and B",
        reach: Reach::Buildings(TOWERS),
    },
    Login {
        name: "Sara Nair",
        email: "property_manager.bay@palmgrove.example",
        role: PROPERTY_MANAGER,
        label: "Property manager, Business Bay and Al Quoz",
        reach: Reach::Buildings(BAY),
    },
    Login {
        name: "Faisal Qadir",
        email: "property_manager.north@palmgrove.example",
        role: PROPERTY_MANAGER,
        label: "Property manager, Abu Dhabi and Sharjah",
        reach: Reach::Buildings(NORTH),
    },
    Login {
        name: "Khaled Mansoor",
        email: "property_manager.all@palmgrove.example",
        role: PROPERTY_MANAGER,
        label: "Property manager, all buildings",
        reach: Reach::Global,
    },
    Login {
        name: "Dana Mercado",
        email: "leasing_agent@palmgrove.example",
        role: LEASING_AGENT,
        label: "Leasing agent",
        reach: Reach::Global,
    },
    Login {
        name: "Vikram Iyer",
        email: "accountant@palmgrove.example",
        role: ACCOUNTANT,
        label: "Accountant",
        reach: Reach::Global,
    },
    Login {
        name: "Hind Al Zaabi",
        email: "receivables@palmgrove.example",
        role: RECEIVABLES,
        label: "Receivables",
        reach: Reach::Global,
    },
    Login {
        name: "Michael Dsouza",
        email: "finance_manager@palmgrove.example",
        role: FINANCE_MANAGER,
        label: "Finance manager",
        reach: Reach::Global,
    },
    Login {
        name: "Hamdan Rashid Al Ketbi",
        email: "owner@palmgrove.example",
        role: OWNER,
        label: "Owner",
        reach: Reach::Global,
    },
    Login {
        name: "Bilal Khan",
        email: "coordinator@palmgrove.example",
        role: COORDINATOR,
        label: "Maintenance coordinator",
        reach: Reach::Global,
    },
    Login {
        name: "Ahmed Yusuf",
        email: "facility_manager@palmgrove.example",
        role: FACILITY_MANAGER,
        label: "Facility manager",
        reach: Reach::Global,
    },
    Login {
        name: "Joel Ramos",
        email: "technician@palmgrove.example",
        role: TECHNICIAN,
        label: "Technician",
        reach: Reach::Global,
    },
    Login {
        name: "Mariam Al Falasi",
        email: "compliance@palmgrove.example",
        role: COMPLIANCE,
        label: "Compliance officer",
        reach: Reach::Global,
    },
    Login {
        name: "Noor Siddiqui",
        email: "marketing@palmgrove.example",
        role: MARKETING,
        label: "Marketing",
        reach: Reach::Global,
    },
    Login {
        name: "Karim Haddad",
        email: "marketing_manager@palmgrove.example",
        role: MARKETING_MANAGER,
        label: "Marketing manager",
        reach: Reach::Global,
    },
    Login {
        name: "Priyanka Das",
        email: "hr@palmgrove.example",
        role: HR,
        label: "HR",
        reach: Reach::Global,
    },
];

/// The login with this role and no other for it (the first of its role).
pub fn login_of(role: &str) -> &'static Login {
    LOGINS
        .iter()
        .find(|login| login.role == role)
        .unwrap_or_else(|| panic!("a demo login for {role}"))
}

/// The head of property: a property manager of every building, who decides
/// what is decided for the company (owner payouts, agreements).
pub const HEAD_OF_PROPERTY: &str = "property_manager.all@palmgrove.example";

/// The email of the property manager who works in `building`.
pub fn manager_of(building: &str) -> &'static str {
    LOGINS
        .iter()
        .find(|login| {
            login.role == PROPERTY_MANAGER
                && matches!(login.reach, Reach::Buildings(codes) if codes.contains(&building))
        })
        .map(|login| login.email)
        .unwrap_or_else(|| panic!("a property manager for {building}"))
}

/// Which kind of party a row is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Who {
    /// A person: nationality and birth year.
    Person(&'static str, i32),
    /// A company: the legal form.
    Company(&'static str),
}

/// One party the demo adds. The number is the demo's own (for phone numbers,
/// identifiers and licences), from 300.
#[derive(Debug, Clone, Copy)]
pub struct Party {
    /// The number.
    pub n: u32,
    /// The English name (also the key the rest of the demo finds it by).
    pub en: &'static str,
    /// The Arabic name.
    pub ar: &'static str,
    /// Person or company.
    pub who: Who,
    /// Its role: `tenant`, `owner` or `vendor`.
    pub role: &'static str,
    /// The language it is written to in.
    pub lang: Language,
    /// The e-mail local part.
    pub mail: &'static str,
}

/// Whether a person of this nationality is written to in Arabic.
const fn arabic(nationality: &str) -> bool {
    let b = nationality.as_bytes();
    matches!(
        (b[0], b[1]),
        (b'A', b'E') | (b'E', b'G') | (b'L', b'B') | (b'J', b'O') | (b'S', b'Y') | (b'S', b'A')
    )
}

const fn person(
    n: u32,
    en: &'static str,
    ar: &'static str,
    nationality: &'static str,
    born: i32,
    role: &'static str,
    mail: &'static str,
) -> Party {
    Party {
        n,
        en,
        ar,
        who: Who::Person(nationality, born),
        role,
        lang: if arabic(nationality) {
            Language::Ar
        } else {
            Language::En
        },
        mail,
    }
}

const fn firm(
    n: u32,
    en: &'static str,
    ar: &'static str,
    role: &'static str,
    mail: &'static str,
) -> Party {
    Party {
        n,
        en,
        ar,
        who: Who::Company("llc"),
        role,
        lang: Language::En,
        mail,
    }
}

/// The parties the demo adds, in the order they are created.
pub const PARTIES: &[Party] = &[
    // Tenants, people.
    person(
        301,
        "Omar Faisal Al Shamsi",
        "عمر فيصل الشامسي",
        "AE",
        1986,
        "tenant",
        "omar.alshamsi",
    ),
    person(
        302,
        "Noura Salem Al Mazrouei",
        "نورة سالم المزروعي",
        "AE",
        1991,
        "tenant",
        "noura.almazrouei",
    ),
    person(
        303,
        "Yousef Ibrahim Al Hammadi",
        "يوسف إبراهيم الحمادي",
        "AE",
        1979,
        "tenant",
        "yousef.alhammadi",
    ),
    person(
        304,
        "Aisha Mohammed Al Blooshi",
        "عائشة محمد البلوشي",
        "AE",
        1994,
        "tenant",
        "aisha.alblooshi",
    ),
    person(
        305,
        "Mahmoud Adel Farouk",
        "محمود عادل فاروق",
        "EG",
        1983,
        "tenant",
        "mahmoud.farouk",
    ),
    person(
        306,
        "Rania Khalil Nasser",
        "رانيا خليل ناصر",
        "LB",
        1988,
        "tenant",
        "rania.nasser",
    ),
    person(
        307,
        "Arjun Menon",
        "أرجون مينون",
        "IN",
        1987,
        "tenant",
        "arjun.menon",
    ),
    person(
        308,
        "Sneha Kulkarni",
        "سنيها كولكارني",
        "IN",
        1992,
        "tenant",
        "sneha.kulkarni",
    ),
    person(
        309,
        "Imran Qureshi",
        "عمران قريشي",
        "PK",
        1985,
        "tenant",
        "imran.qureshi",
    ),
    person(
        310,
        "Zainab Ahmed Malik",
        "زينب أحمد مالك",
        "PK",
        1990,
        "tenant",
        "zainab.malik",
    ),
    person(
        311,
        "Jose Miguel Santos",
        "خوسيه ميغيل سانتوس",
        "PH",
        1984,
        "tenant",
        "jose.santos",
    ),
    person(
        312,
        "Angelica Reyes",
        "أنجيليكا رييس",
        "PH",
        1995,
        "tenant",
        "angelica.reyes",
    ),
    person(
        313,
        "Daniel Whitaker",
        "دانيال ويتاكر",
        "GB",
        1981,
        "tenant",
        "daniel.whitaker",
    ),
    person(
        314,
        "Elena Petrova",
        "إيلينا بيتروفا",
        "RU",
        1989,
        "tenant",
        "elena.petrova",
    ),
    person(
        315,
        "Thomas Okafor",
        "توماس أوكافور",
        "NG",
        1982,
        "tenant",
        "thomas.okafor",
    ),
    person(
        316,
        "Fatuma Ali Hassan",
        "فاتوما علي حسن",
        "KE",
        1993,
        "tenant",
        "fatuma.hassan",
    ),
    person(
        317,
        "Lukas Meier",
        "لوكاس ماير",
        "DE",
        1980,
        "tenant",
        "lukas.meier",
    ),
    person(
        318,
        "Camille Laurent",
        "كميل لوران",
        "FR",
        1990,
        "tenant",
        "camille.laurent",
    ),
    person(
        319,
        "Hassan Jaber Al Rumaithi",
        "حسن جابر الرميثي",
        "AE",
        1977,
        "tenant",
        "hassan.alrumaithi",
    ),
    person(
        320,
        "Salma Tarek Mansour",
        "سلمى طارق منصور",
        "EG",
        1996,
        "tenant",
        "salma.mansour",
    ),
    person(
        321,
        "Nikhil Deshpande",
        "نيخيل ديشباندي",
        "IN",
        1986,
        "tenant",
        "nikhil.deshpande",
    ),
    person(
        322,
        "Marwa Saleh Al Ameri",
        "مروى صالح العامري",
        "AE",
        1998,
        "tenant",
        "marwa.alameri",
    ),
    person(
        323,
        "Ibrahim Saleh Al Kaabi",
        "إبراهيم صالح الكعبي",
        "AE",
        1975,
        "tenant",
        "ibrahim.alkaabi",
    ),
    person(
        324,
        "Sunita Rao",
        "سونيتا راو",
        "IN",
        1988,
        "tenant",
        "sunita.rao",
    ),
    person(
        325,
        "Ahmed Nabil Khalifa",
        "أحمد نبيل خليفة",
        "EG",
        1984,
        "tenant",
        "ahmed.khalifa",
    ),
    person(
        326,
        "Grace Mwangi",
        "غريس موانجي",
        "KE",
        1991,
        "tenant",
        "grace.mwangi",
    ),
    person(
        327,
        "Pavel Novak",
        "بافيل نوفاك",
        "CZ",
        1983,
        "tenant",
        "pavel.novak",
    ),
    person(
        328,
        "Dina Qasim Al Zahrani",
        "دينا قاسم الزهراني",
        "SA",
        1987,
        "tenant",
        "dina.alzahrani",
    ),
    // Tenants, companies.
    firm(
        331,
        "Desert Rose Cafe LLC",
        "مقهى وردة الصحراء ذ.م.م",
        "tenant",
        "accounts@desertrose",
    ),
    firm(
        332,
        "Falcon Eye Security Services LLC",
        "عين الصقر للخدمات الأمنية ذ.م.م",
        "tenant",
        "finance@falconeye",
    ),
    firm(
        333,
        "Skyline Dental Clinic LLC",
        "عيادة الأفق لطب الأسنان ذ.م.م",
        "tenant",
        "admin@skylinedental",
    ),
    firm(
        334,
        "Northgate Trading FZCO",
        "نورثجيت للتجارة م.د.م.س",
        "tenant",
        "ap@northgate",
    ),
    firm(
        335,
        "Pearl Tech Solutions LLC",
        "لؤلؤة تك للحلول ذ.م.م",
        "tenant",
        "billing@pearltech",
    ),
    firm(
        336,
        "Meridian Logistics LLC",
        "ميريديان للخدمات اللوجستية ذ.م.م",
        "tenant",
        "finance@meridian",
    ),
    // Owners.
    person(
        341,
        "Mariam Obaid Al Suwaidi",
        "مريم عبيد السويدي",
        "AE",
        1972,
        "owner",
        "mariam.alsuwaidi",
    ),
    person(
        342,
        "Rashed Saif Al Dhaheri",
        "راشد سيف الظاهري",
        "AE",
        1965,
        "owner",
        "rashed.aldhaheri",
    ),
    person(
        343,
        "Anwar Hossain",
        "أنور حسين",
        "BD",
        1970,
        "owner",
        "anwar.hossain",
    ),
    firm(
        344,
        "Greenfield Property Investments LLC",
        "غرينفيلد للاستثمار العقاري ذ.م.م",
        "owner",
        "office@greenfield",
    ),
    // Vendors.
    firm(
        351,
        "Skyhigh Lifts and Escalators LLC",
        "سكاي هاي للمصاعد والسلالم المتحركة ذ.م.م",
        "vendor",
        "service@skyhighlifts",
    ),
    firm(
        352,
        "Safeguard Fire Systems LLC",
        "سيف جارد لأنظمة الإطفاء ذ.م.م",
        "vendor",
        "ops@safeguardfire",
    ),
    firm(
        353,
        "BrightClean Facility Services LLC",
        "برايت كلين لخدمات المرافق ذ.م.م",
        "vendor",
        "jobs@brightclean",
    ),
    firm(
        354,
        "Pestguard Gulf LLC",
        "بست جارد الخليج ذ.م.م",
        "vendor",
        "hello@pestguard",
    ),
    firm(
        355,
        "Spark Electrical Contracting LLC",
        "سبارك للمقاولات الكهربائية ذ.م.م",
        "vendor",
        "jobs@sparkelectrical",
    ),
];
