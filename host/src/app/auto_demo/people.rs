//! The people of the demo company: the staff with a login, the customers
//! (private owners, fleets, a workshop, the insurers and the suppliers as
//! parties, English and Arabic names) and their vehicles.

use std::sync::atomic::{AtomicU64, Ordering};

use auto::Emirate;
use auto::vehicles::{self, NewVehicle};
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::PartyKind;
use wl_parties::{
    Channel, CompanyDetails, ContactPoint, ContactType, Language, NewParty, PartyRole,
    PersonDetails,
};

use super::super::demo_data::at;
use super::super::demo_data::day;
use super::super::seed::PASSWORD;
use super::{LOGINS, SAMI, Seed, hooks};

static NONCE: AtomicU64 = AtomicU64::new(1);

/// A person who is a customer: key, English name, Arabic name, mobile.
const PEOPLE: &[(&str, &str, &str, &str)] = &[
    ("mona", "Mona Hassan", "منى حسن", "+971501110001"),
    ("rashed", "Rashed Salem", "راشد سالم", "+971501110002"),
    (
        "yousef",
        "Yousef Al Mansoori",
        "يوسف المنصوري",
        "+971501110003",
    ),
    ("priya", "Priya Nair", "بريا نائير", "+971501110004"),
    ("khalid", "Khalid Ibrahim", "خالد إبراهيم", "+971501110005"),
    (
        "fatima",
        "Fatima Al Zaabi",
        "فاطمة الزعابي",
        "+971501110006",
    ),
    ("ahmed", "Ahmed Raza", "أحمد رضا", "+971501110007"),
    ("layla", "Layla Haddad", "ليلى حداد", "+971501110008"),
    ("omar", "Omar Saleh", "عمر صالح", "+971501110009"),
    ("noor", "Noor Khalid", "نور خالد", "+971501110010"),
    ("hamdan", "Hamdan Saeed", "حمدان سعيد", "+971501110011"),
    ("salim", "Salim Obaid", "سالم عبيد", "+971501110012"),
];

/// A company: key, English name, Arabic name, e-mail, role.
const COMPANIES: &[(&str, &str, &str, &str, &str)] = &[
    (
        "noor_logistics",
        "Al Noor Logistics LLC",
        "النور للخدمات اللوجستية ذ.م.م",
        "fleet@noorlogistics.example",
        "fleet",
    ),
    (
        "desert_rent",
        "Desert Rent a Car LLC",
        "ديزرت لتأجير السيارات ذ.م.م",
        "ops@desertrent.example",
        "fleet",
    ),
    (
        "sharjah_cargo",
        "Sharjah Cargo Express LLC",
        "الشارقة للشحن السريع ذ.م.م",
        "ops@sharjahcargo.example",
        "fleet",
    ),
    (
        "lessor",
        "Emirates Leasing PJSC",
        "الإمارات للتأجير ش.م.ع",
        "leases@emiratesleasing.example",
        "customer",
    ),
    (
        "quick_fit",
        "Quick Tune Workshop",
        "ورشة كويك تيون",
        "orders@quicktune.example",
        "customer",
    ),
    (
        "movers",
        "Gulf Movers Trading",
        "الخليج لنقل البضائع التجارية",
        "accounts@gulfmovers.example",
        "customer",
    ),
    (
        "falcon",
        "Falcon Motor Insurance",
        "الصقر للتأمين على المركبات",
        "claims@falconmotor.example",
        "insurer",
    ),
    (
        "crescent",
        "Crescent Gulf Insurance",
        "الهلال الخليجي للتأمين",
        "motor@crescentgulf.example",
        "insurer",
    ),
    (
        "gulf_spares",
        "Gulf Auto Spares FZE",
        "الخليج لقطع غيار السيارات م.ح.",
        "sales@gulfspares.example",
        "supplier",
    ),
    (
        "brake_parts",
        "Brake Parts FZE",
        "مصنع قطع الفرامل م.ح.",
        "sales@brakeparts.example",
        "supplier",
    ),
    (
        "oil_house",
        "Oil House Trading",
        "بيت الزيوت للتجارة",
        "orders@oilhouse.example",
        "supplier",
    ),
];

/// A vehicle: key, emirate, plate code, number, make, model, year, colour,
/// owner key, odometer, VIN.
pub(crate) struct Car {
    pub key: &'static str,
    pub emirate: Emirate,
    pub code: &'static str,
    pub number: &'static str,
    pub make: &'static str,
    pub model: &'static str,
    pub year: i64,
    pub colour: &'static str,
    pub owner: &'static str,
    pub km: i64,
}

#[allow(clippy::too_many_arguments)]
const fn car(
    key: &'static str,
    emirate: Emirate,
    code: &'static str,
    number: &'static str,
    make: &'static str,
    model: &'static str,
    year: i64,
    colour: &'static str,
    owner: &'static str,
    km: i64,
) -> Car {
    Car {
        key,
        emirate,
        code,
        number,
        make,
        model,
        year,
        colour,
        owner,
        km,
    }
}

pub(crate) const CARS: &[Car] = &[
    car(
        "hilux",
        Emirate::Dubai,
        "A",
        "12345",
        "Toyota",
        "Hilux",
        2021,
        "White",
        "mona",
        52_000,
    ),
    car(
        "patrol",
        Emirate::Dubai,
        "C",
        "77701",
        "Nissan",
        "Patrol",
        2019,
        "Black",
        "rashed",
        88_000,
    ),
    car(
        "accord",
        Emirate::Sharjah,
        "2",
        "45120",
        "Honda",
        "Accord",
        2020,
        "Silver",
        "yousef",
        61_000,
    ),
    car(
        "tucson",
        Emirate::Dubai,
        "K",
        "90311",
        "Hyundai",
        "Tucson",
        2022,
        "Grey",
        "priya",
        30_400,
    ),
    car(
        "sportage",
        Emirate::AbuDhabi,
        "12",
        "55872",
        "Kia",
        "Sportage",
        2021,
        "Blue",
        "khalid",
        47_900,
    ),
    car(
        "l200",
        Emirate::Sharjah,
        "1",
        "30188",
        "Mitsubishi",
        "L200",
        2018,
        "White",
        "fatima",
        120_300,
    ),
    car(
        "corolla",
        Emirate::Dubai,
        "N",
        "66023",
        "Toyota",
        "Corolla",
        2023,
        "Red",
        "ahmed",
        12_800,
    ),
    car(
        "lexus",
        Emirate::Dubai,
        "B",
        "40417",
        "Lexus",
        "ES 350",
        2020,
        "Pearl",
        "layla",
        58_200,
    ),
    car(
        "sunny",
        Emirate::Ajman,
        "A",
        "21904",
        "Nissan",
        "Sunny",
        2017,
        "Beige",
        "omar",
        143_000,
    ),
    car(
        "camry",
        Emirate::Dubai,
        "D",
        "18830",
        "Toyota",
        "Camry",
        2022,
        "Black",
        "noor",
        22_500,
    ),
    car(
        "pajero",
        Emirate::Dubai,
        "E",
        "70212",
        "Mitsubishi",
        "Pajero",
        2016,
        "Gold",
        "salim",
        171_000,
    ),
    car(
        "civic",
        Emirate::Dubai,
        "F",
        "33098",
        "Honda",
        "Civic",
        2021,
        "White",
        "hamdan",
        40_100,
    ),
    car(
        "urvan1",
        Emirate::Dubai,
        "B",
        "55501",
        "Nissan",
        "Urvan",
        2022,
        "White",
        "noor_logistics",
        59_800,
    ),
    car(
        "urvan2",
        Emirate::Dubai,
        "B",
        "55502",
        "Nissan",
        "Urvan",
        2022,
        "White",
        "noor_logistics",
        41_300,
    ),
    car(
        "hiace",
        Emirate::Dubai,
        "B",
        "55503",
        "Toyota",
        "Hiace",
        2021,
        "White",
        "noor_logistics",
        77_000,
    ),
    car(
        "yaris1",
        Emirate::Dubai,
        "G",
        "88101",
        "Toyota",
        "Yaris",
        2023,
        "Orange",
        "desert_rent",
        36_000,
    ),
    car(
        "yaris2",
        Emirate::Dubai,
        "G",
        "88102",
        "Toyota",
        "Yaris",
        2023,
        "Orange",
        "desert_rent",
        33_500,
    ),
    car(
        "canter",
        Emirate::Sharjah,
        "3",
        "14420",
        "Mitsubishi",
        "Canter",
        2019,
        "White",
        "sharjah_cargo",
        98_000,
    ),
    car(
        "van_lease",
        Emirate::Dubai,
        "B",
        "55509",
        "Nissan",
        "Urvan",
        2020,
        "White",
        "lessor",
        60_000,
    ),
    car(
        "cl1",
        Emirate::Dubai,
        "P",
        "20101",
        "Nissan",
        "Altima",
        2021,
        "Grey",
        "mona",
        34000,
    ),
    car(
        "cl2",
        Emirate::Dubai,
        "P",
        "20102",
        "Toyota",
        "RAV4",
        2022,
        "White",
        "priya",
        18000,
    ),
    car(
        "cl3",
        Emirate::Sharjah,
        "4",
        "20103",
        "Kia",
        "Cerato",
        2020,
        "Red",
        "yousef",
        52000,
    ),
    car(
        "cl4",
        Emirate::Dubai,
        "P",
        "20104",
        "Hyundai",
        "Santa Fe",
        2021,
        "Black",
        "khalid",
        41000,
    ),
    car(
        "cl5",
        Emirate::Dubai,
        "P",
        "20105",
        "Ford",
        "Explorer",
        2019,
        "Blue",
        "fatima",
        77000,
    ),
    car(
        "cl6",
        Emirate::Dubai,
        "P",
        "20106",
        "Mazda",
        "CX-5",
        2022,
        "Grey",
        "ahmed",
        15500,
    ),
    car(
        "cl7",
        Emirate::Ajman,
        "B",
        "20107",
        "Toyota",
        "Prado",
        2018,
        "White",
        "layla",
        120000,
    ),
    car(
        "cl8",
        Emirate::Dubai,
        "P",
        "20108",
        "Honda",
        "CR-V",
        2020,
        "Silver",
        "omar",
        63000,
    ),
    car(
        "cl9",
        Emirate::Dubai,
        "P",
        "20109",
        "Nissan",
        "X-Trail",
        2021,
        "Green",
        "noor",
        39000,
    ),
    car(
        "cl10",
        Emirate::Sharjah,
        "5",
        "20110",
        "Chevrolet",
        "Tahoe",
        2017,
        "Black",
        "hamdan",
        138000,
    ),
    car(
        "cl11",
        Emirate::Dubai,
        "P",
        "20111",
        "Suzuki",
        "Swift",
        2023,
        "Blue",
        "salim",
        9800,
    ),
    car(
        "accident2",
        Emirate::Dubai,
        "H",
        "10022",
        "Nissan",
        "Patrol",
        2020,
        "Silver",
        "ahmed",
        66_000,
    ),
    car(
        "accident1",
        Emirate::Dubai,
        "H",
        "10021",
        "Toyota",
        "Land Cruiser",
        2022,
        "Black",
        "mona",
        28_000,
    ),
];

impl Seed {
    /// Makes the staff users, signs them in and keeps them as the services
    /// take them.
    pub(crate) async fn logins(&mut self) -> Result {
        for login in LOGINS {
            let user = match User::find_by_email(self.db(), login.email).await? {
                Some(user) => user,
                None => {
                    let mut user =
                        User::register(self.db(), login.name, login.email, PASSWORD).await?;
                    user.email_verified_at = Some(renox::db::now());
                    user.save(self.db()).await?;
                    user.assign_role(self.db(), login.role).await?;
                    user
                }
            };
            self.driver
                .sign_in(&self.state.db, login.email, &user)
                .await?;
            let key = format!(
                "{}-{}-{}",
                login.email,
                std::process::id(),
                NONCE.fetch_add(1, Ordering::Relaxed)
            );
            self.driver
                .post(login.email, "/demo/auto/whoami", &[("key", key.as_str())])
                .await?;
            let auth = hooks::take(&key)
                .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no user for {}", login.email))?;
            self.staff.insert(login.email, auth);
            self.users.push((login.email, user));
        }
        Ok(())
    }

    async fn make_party(&self, new: NewParty) -> Result<PartyId> {
        Ok(wl_parties::create(&self.state, new).await?.party_id())
    }

    /// The customers, fleets, insurers and suppliers, then the vehicles.
    pub(crate) async fn people(&mut self) -> Result {
        let advisor = self.who(SAMI).clone();
        at!(self, day(-200), {
            for (key, en, ar, mobile) in PEOPLE {
                let id = self
                    .make_party(NewParty {
                        kind: PartyKind::Person,
                        legal_name_en: (*en).to_owned(),
                        legal_name_ar: Some((*ar).to_owned()),
                        person: Some(PersonDetails::default()),
                        default_language: Language::En,
                        preferred_channel: Channel::Sms,
                        roles: vec![PartyRole {
                            role_key: "customer".to_owned(),
                            ..PartyRole::default()
                        }],
                        contacts: vec![
                            ContactPoint {
                                contact_type: ContactType::Mobile,
                                value: (*mobile).to_owned(),
                                is_primary: true,
                                verified_at: Some(renox::db::now()),
                                ..ContactPoint::default()
                            },
                            ContactPoint {
                                contact_type: ContactType::Email,
                                value: format!("{key}@customers.alnoorauto.example"),
                                ..ContactPoint::default()
                            },
                        ],
                        source: "seed".to_owned(),
                        ..NewParty::default()
                    })
                    .await?;
                self.parties.insert(key, id);
            }
            for (key, en, ar, email, role) in COMPANIES {
                let id = self
                    .make_party(NewParty {
                        kind: PartyKind::Company,
                        legal_name_en: (*en).to_owned(),
                        legal_name_ar: Some((*ar).to_owned()),
                        company: Some(CompanyDetails {
                            legal_form: Some("llc".to_owned()),
                            licence_authority: Some("Dubai Economy".to_owned()),
                            trade_licence_no: Some(format!(
                                "DEMO-8{:05}",
                                self.parties.len() * 137
                            )),
                            ..CompanyDetails::default()
                        }),
                        roles: vec![PartyRole {
                            role_key: (*role).to_owned(),
                            ..PartyRole::default()
                        }],
                        contacts: vec![ContactPoint {
                            contact_type: ContactType::Email,
                            value: (*email).to_owned(),
                            is_primary: true,
                            ..ContactPoint::default()
                        }],
                        source: "seed".to_owned(),
                        ..NewParty::default()
                    })
                    .await?;
                self.parties.insert(key, id);
            }
            Ok::<(), renox::Error>(())
        })?;
        let mut made = Vec::new();
        at!(self, day(-190), {
            for car in CARS {
                let owner = self.party(car.owner);
                let vehicle = vehicles::create(
                    &self.state,
                    &advisor,
                    NewVehicle::new(
                        car.emirate,
                        car.code,
                        car.number,
                        car.make,
                        car.model,
                        owner.0,
                    )
                    .vin(&format!("JT{:015}", 4_000_000 + made.len() * 7_919))
                    .year(car.year)
                    .colour(car.colour)
                    .odometer(car.km),
                )
                .await?;
                made.push((car.key, vehicle));
            }
            Ok::<(), renox::Error>(())
        })?;
        self.vehicles.extend(made);
        Ok(())
    }
}
