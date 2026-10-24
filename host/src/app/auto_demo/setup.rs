//! What the workshop and the parts counter start from: the customers (car
//! owners, a contractor, a fleet company), the insurers and the suppliers;
//! the parts with their opening stock in the workshop's store and at the
//! Deira counter; the labour menu; the insurers' agreements; the customers'
//! vehicles; and the workshop's bay with the service types it offers.

use renox::prelude::*;
use wl_inventory::movement::StockMovement;
use wl_inventory::{Item, Location};
use wl_kernel::{PartyId, PartyKind, Subject};
use wl_parties::{
    Channel, CompanyDetails, ContactPoint, ContactType, Language, NewParty, PartyRole,
    PersonDetails,
};
use wl_scheduling::{AvailabilityRule, Resource, ResourceType, ServiceType, resource};

use super::{ADVISOR_EMAIL, INSURANCE_EMAIL, STORE_EMAIL, Seed, WORKSHOP_EMAIL, workshop};

/// A party: key, kind (`person`, `company`), name, Arabic name, e-mail, role.
type PartyRow = (
    &'static str,
    &'static str,
    &'static str,
    Option<&'static str>,
    &'static str,
    Option<&'static str>,
);

const PARTIES: &[PartyRow] = &[
    (
        "mona",
        "person",
        "Mona Hassan",
        Some("منى حسن"),
        "mona.hassan@mail.example",
        None,
    ),
    (
        "rashid",
        "person",
        "Rashid Al Nuaimi",
        Some("راشد النعيمي"),
        "rashid@mail.example",
        None,
    ),
    (
        "priya",
        "person",
        "Priya Raman",
        None,
        "priya.raman@mail.example",
        None,
    ),
    (
        "omar",
        "person",
        "Omar Siddiqui",
        None,
        "omar.siddiqui@mail.example",
        None,
    ),
    (
        "fatima",
        "person",
        "Fatima Al Zaabi",
        Some("فاطمة الزعابي"),
        "fatima@mail.example",
        None,
    ),
    (
        "joseph",
        "person",
        "Joseph Fernandes",
        None,
        "joseph.f@mail.example",
        None,
    ),
    (
        "ahmed",
        "person",
        "Ahmed Karim",
        Some("أحمد كريم"),
        "ahmed.karim@mail.example",
        None,
    ),
    (
        "ravi",
        "person",
        "Ravi Kumar",
        None,
        "ravi.kumar@mail.example",
        None,
    ),
    (
        "sara",
        "person",
        "Sara Al Mansoori",
        Some("سارة المنصوري"),
        "sara.m@mail.example",
        None,
    ),
    (
        "khalid",
        "person",
        "Khalid Obaid",
        Some("خالد عبيد"),
        "khalid.obaid@mail.example",
        None,
    ),
    (
        "gulfbuild",
        "company",
        "Gulf Build Contracting LLC",
        Some("شركة الخليج للمقاولات"),
        "fleet@gulfbuild.example",
        Some("customer"),
    ),
    (
        "efr",
        "company",
        "Emirates Fleet Rentals LLC",
        Some("الإمارات لتأجير الأساطيل"),
        "operations@efrentals.example",
        Some("customer"),
    ),
    (
        "efr_contact",
        "person",
        "Hassan Qureshi",
        None,
        "hassan.qureshi@efrentals.example",
        None,
    ),
    (
        "quickfix",
        "company",
        "Quick Fix Garage LLC",
        Some("كراج الإصلاح السريع"),
        "parts@quickfix.example",
        Some("customer"),
    ),
    (
        "afriparts",
        "company",
        "Afri Motors Export FZE",
        None,
        "buying@afrimotors.example",
        Some("customer"),
    ),
    (
        "shield",
        "company",
        "Gulf Shield Insurance PJSC",
        Some("درع الخليج للتأمين"),
        "motorclaims@gulfshield.example",
        Some("insurer"),
    ),
    (
        "falcon",
        "company",
        "Desert Falcon Insurance Co.",
        Some("صقر الصحراء للتأمين"),
        "claims@desertfalcon.example",
        Some("insurer"),
    ),
    (
        "japarts",
        "company",
        "Orient Japanese Parts Trading LLC",
        None,
        "sales@orientparts.example",
        Some("supplier"),
    ),
    (
        "eurobat",
        "company",
        "Euro Battery Distributors FZCO",
        None,
        "orders@eurobattery.example",
        Some("supplier"),
    ),
];

/// A part: code, name, Arabic name, average cost (fils), reorder point,
/// units in the workshop's store, units at the Deira counter.
type PartRow = (&'static str, &'static str, &'static str, i64, i64, i64, i64);

const PARTS: &[PartRow] = &[
    (
        "OIL-5W30-4L",
        "Engine oil 5W-30, 4 L",
        "زيت محرك 5W-30 4 لتر",
        6250,
        20,
        48,
        120,
    ),
    (
        "FLT-OIL-TY",
        "Oil filter, Toyota",
        "فلتر زيت تويوتا",
        1475,
        15,
        36,
        80,
    ),
    (
        "FLT-AIR-TY",
        "Air filter, Toyota",
        "فلتر هواء تويوتا",
        2840,
        10,
        18,
        40,
    ),
    (
        "BRK-PAD-TY",
        "Front brake pad set, Toyota",
        "فحمات فرامل أمامية تويوتا",
        9600,
        8,
        12,
        30,
    ),
    (
        "BRK-PAD-NS",
        "Front brake pad set, Nissan",
        "فحمات فرامل أمامية نيسان",
        8850,
        8,
        3,
        6,
    ),
    (
        "BRK-DSC-TY",
        "Front brake disc, Toyota",
        "قرص فرامل أمامي تويوتا",
        14200,
        4,
        6,
        10,
    ),
    (
        "SPK-PLG-IR",
        "Iridium spark plug",
        "شمعة احتراق إيريديوم",
        3125,
        24,
        40,
        96,
    ),
    (
        "BAT-70AH",
        "Battery 70 Ah",
        "بطارية 70 أمبير",
        26800,
        6,
        4,
        22,
    ),
    (
        "WPR-BLD-24",
        "Wiper blade 24 in",
        "مساحة زجاج 24 بوصة",
        1790,
        20,
        30,
        60,
    ),
    (
        "BMP-FR-FT",
        "Front bumper, Fortuner",
        "صدام أمامي فورتشنر",
        115000,
        1,
        1,
        0,
    ),
    (
        "HDL-LH-FT",
        "Headlamp left, Fortuner",
        "مصباح أمامي أيسر فورتشنر",
        84500,
        1,
        1,
        0,
    ),
    (
        "ALT-PJ-90A",
        "Alternator 90 A, Pajero",
        "دينامو 90 أمبير باجيرو",
        132000,
        1,
        0,
        0,
    ),
    (
        "CLT-KIT-NS",
        "Clutch kit, Nissan Sunny",
        "طقم كلتش نيسان صني",
        61000,
        2,
        2,
        3,
    ),
    (
        "SHK-RR-PT",
        "Rear shock absorber, Patrol",
        "ممتص صدمات خلفي باترول",
        39500,
        2,
        4,
        2,
    ),
];

/// A labour operation: code, name, Arabic name, `hourly` or `flat_rate`,
/// hours, rate (AED).
const LABOUR: &[(&str, &str, &str, &str, &str, &str)] = &[
    (
        "SVC-MINOR",
        "Minor service (oil and filter)",
        "صيانة صغرى",
        "flat_rate",
        "1",
        "180",
    ),
    (
        "SVC-MAJOR",
        "Major service (60,000 km)",
        "صيانة كبرى",
        "flat_rate",
        "3",
        "650",
    ),
    (
        "BRK-FRONT",
        "Front brake pads replacement",
        "تغيير فحمات الفرامل الأمامية",
        "flat_rate",
        "1.5",
        "220",
    ),
    (
        "DIAG",
        "Electrical diagnosis",
        "تشخيص كهربائي",
        "hourly",
        "",
        "240",
    ),
    (
        "BODY-PAINT",
        "Body and paint",
        "سمكرة ودهان",
        "hourly",
        "",
        "260",
    ),
    (
        "CLUTCH",
        "Clutch replacement",
        "تغيير الكلتش",
        "flat_rate",
        "4",
        "780",
    ),
];

/// A vehicle: key, owner key, emirate, plate code, plate number, make,
/// model, year, colour, odometer.
type VehicleRow = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
);

const VEHICLES: &[VehicleRow] = &[
    (
        "lc",
        "mona",
        "dubai",
        "K",
        "48213",
        "Toyota",
        "Land Cruiser",
        "2021",
        "White",
        "61850",
    ),
    (
        "patrol",
        "rashid",
        "abu_dhabi",
        "5",
        "70129",
        "Nissan",
        "Patrol",
        "2019",
        "Silver",
        "118400",
    ),
    (
        "hilux",
        "gulfbuild",
        "dubai",
        "R",
        "33017",
        "Toyota",
        "Hilux",
        "2022",
        "White",
        "96200",
    ),
    (
        "accord", "priya", "dubai", "F", "21944", "Honda", "Accord", "2020", "Grey", "74310",
    ),
    (
        "pajero",
        "omar",
        "sharjah",
        "3",
        "55802",
        "Mitsubishi",
        "Pajero",
        "2017",
        "Black",
        "152600",
    ),
    (
        "camry", "fatima", "dubai", "L", "60451", "Toyota", "Camry", "2023", "Pearl", "28900",
    ),
    (
        "corolla", "joseph", "dubai", "B", "19376", "Toyota", "Corolla", "2018", "Blue", "131250",
    ),
    (
        "altima", "ahmed", "ajman", "A", "42718", "Nissan", "Altima", "2021", "Red", "57400",
    ),
    (
        "sunny", "ravi", "sharjah", "2", "88123", "Nissan", "Sunny", "2016", "White", "189700",
    ),
    (
        "fortuner", "sara", "dubai", "M", "71590", "Toyota", "Fortuner", "2022", "Bronze", "43800",
    ),
    (
        "tiida", "khalid", "dubai", "D", "30982", "Nissan", "Tiida", "2019", "Silver", "88050",
    ),
    (
        "van1", "efr", "dubai", "Q", "11406", "Toyota", "Hiace", "2023", "White", "38600",
    ),
    (
        "van2", "efr", "dubai", "Q", "11407", "Toyota", "Hiace", "2023", "White", "41120",
    ),
];

impl Seed {
    async fn party(&self, row: &PartyRow, phone: usize) -> Result<PartyId> {
        let (_, kind, name, name_ar, email, role) = *row;
        let contact = |contact_type, value: String| ContactPoint {
            contact_type,
            value,
            is_primary: true,
            ..Default::default()
        };
        let company = kind == "company";
        Ok(wl_parties::create(
            &self.state,
            NewParty {
                kind: if company {
                    PartyKind::Company
                } else {
                    PartyKind::Person
                },
                legal_name_en: name.to_owned(),
                legal_name_ar: name_ar.map(str::to_owned),
                company: company.then(CompanyDetails::default),
                person: (!company).then(PersonDetails::default),
                roles: role
                    .map(|key| {
                        vec![PartyRole {
                            role_key: key.to_owned(),
                            ..PartyRole::default()
                        }]
                    })
                    .unwrap_or_default(),
                default_language: Language::En,
                preferred_channel: Channel::Email,
                contacts: vec![
                    contact(ContactType::Email, email.to_owned()),
                    contact(ContactType::Mobile, format!("+97150000{phone:04}")),
                ],
                source: "seed".to_owned(),
                ..Default::default()
            },
        )
        .await?
        .party_id())
    }

    /// The parts, a store at the workshop and one at the Deira counter, and
    /// the opening stock of each.
    async fn stock(&mut self) -> Result {
        let mut store = Location::new("WS-ALQUOZ", "Al Quoz workshop store").in_scope(&workshop());
        store.save(self.db()).await?;
        let mut counter = Location::new("CTR-DEIRA", "Deira parts counter");
        counter.save(self.db()).await?;
        let by = self.user(STORE_EMAIL).await?.id;
        let mut receipt = 3_100;
        for (code, name, name_ar, cost, reorder, at_store, at_counter) in PARTS {
            let mut item = Item::new(code, name, "pcs");
            item.name_ar = Some((*name_ar).to_owned());
            item.average_cost = *cost;
            item.reorder_point_milli = reorder * 1_000;
            item.reorder_max_milli = reorder * 3_000;
            item.save(self.db()).await?;
            for (location, units) in [(store.id, *at_store), (counter.id, *at_counter)] {
                if units == 0 {
                    continue;
                }
                receipt += 1;
                let quantity = units * 1_000;
                StockMovement::receive(
                    item.id,
                    None,
                    location,
                    quantity,
                    *cost,
                    quantity * cost / 1_000,
                    &Subject::new("inventory.goods_receipt", receipt),
                    Some(by),
                )
                .save(self.db())
                .await?;
            }
        }
        Ok(())
    }

    /// The bay of the workshop, open every day, offering every service type
    /// of the workshop.
    async fn bay(&mut self) -> Result {
        let mut bay = Resource::new("Bay 1", ResourceType::Bay).at(&workshop());
        bay.save(self.db()).await?;
        for weekday in 1..=7 {
            AvailabilityRule::new(bay.id, weekday, 0, 1440)
                .save(self.db())
                .await?;
        }
        for service in ServiceType::query().get(self.db()).await? {
            resource::offer(self.db(), service.id, bay.id).await?;
        }
        self.bay = bay.id;
        Ok(())
    }

    /// Everything the workshop and the counter start from.
    pub(crate) async fn setup(&mut self) -> Result {
        for (n, row) in PARTIES.iter().enumerate() {
            let party = self.party(row, 200 + n).await?;
            self.parties.insert(row.0, party);
        }
        self.stock().await?;
        self.bay().await?;
        for (code, name, name_ar, pricing, hours, rate) in LABOUR {
            self.driver
                .post(
                    WORKSHOP_EMAIL,
                    "/auto/labour-menu",
                    &[
                        ("code", code),
                        ("name_en", name),
                        ("name_ar", name_ar),
                        ("pricing", pricing),
                        ("hours", hours),
                        ("rate", rate),
                    ],
                )
                .await?;
        }
        for (key, email, surveyors, terms, labour) in [
            (
                "shield",
                "motorclaims@gulfshield.example",
                "Anil Menon, Yasmin Saleh",
                "45",
                "240",
            ),
            (
                "falcon",
                "claims@desertfalcon.example",
                "Rami Khoury",
                "60",
                "225",
            ),
        ] {
            self.driver
                .post(
                    INSURANCE_EMAIL,
                    "/auto/insurers",
                    &[
                        ("party", &self.parties[key].0.to_string()),
                        ("email", email),
                        ("surveyors", surveyors),
                        ("terms", terms),
                        ("labour_rate", labour),
                        ("storage_rate", "75"),
                    ],
                )
                .await?;
        }
        for (key, owner, emirate, code, number, make, model, year, colour, km) in VEHICLES {
            let made = self
                .driver
                .post(
                    ADVISOR_EMAIL,
                    "/auto/vehicles",
                    &[
                        ("emirate", emirate),
                        ("plate_code", code),
                        ("plate_number", number),
                        ("vin", &vin(make, number)),
                        ("make", make),
                        ("model_name", model),
                        ("model_year", year),
                        ("colour", colour),
                        ("owner", &self.parties[owner].0.to_string()),
                        ("odometer", km),
                    ],
                )
                .await?;
            let id = made
                .redirected_id()
                .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the vehicle {key}"))?;
            self.vehicles.insert(key, id);
        }
        Ok(())
    }
}

/// A demo VIN: the maker's prefix, `DEM0` (a zero: a VIN has no letter O)
/// and the plate number, 17 characters.
fn vin(make: &str, number: &str) -> String {
    let prefix = match make {
        "Toyota" => "JTM",
        "Nissan" => "JN1",
        "Honda" => "JHM",
        _ => "JMB",
    };
    format!(
        "{prefix}DEM0{:010}",
        number.parse::<i64>().unwrap_or_default()
    )
}
