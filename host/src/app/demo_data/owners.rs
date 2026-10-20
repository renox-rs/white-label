//! The management agreements of the demo company: which owner has which units
//! with the company, on what fee, and where each agreement stands.

/// Where an agreement stands at the end of the seed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// Being prepared.
    Draft,
    /// In force; its monthly statements run.
    Active,
    /// Notice given on this day (days from today), units handed over on the
    /// other.
    Ending(i64, i64),
    /// Notice and handover (days from today) both in the past; ended.
    Ended(i64, i64),
}

/// One agreement.
#[derive(Debug, Clone, Copy)]
pub struct AgreementPlan {
    /// The owner's English name.
    pub owner: &'static str,
    /// The fee in percent of the rent collected.
    pub fee: &'static str,
    /// The fixed fee per statement in dirhams.
    pub fixed: &'static str,
    /// The first day, in days from today.
    pub start: i64,
    /// The units: building code and numbers.
    pub units: &'static [(&'static str, &'static [&'static str])],
    /// Where it stands.
    pub standing: Standing,
}

/// The agreements, in the order they are made.
pub const AGREEMENTS: &[AgreementPlan] = &[
    AgreementPlan {
        owner: "Oliver Brennan",
        fee: "5.00",
        fixed: "150",
        start: -760,
        units: &[("JVT-A", &["101", "102", "201", "202", "301", "302"])],
        standing: Standing::Active,
    },
    AgreementPlan {
        owner: "Hamdan Rashid Al Ketbi",
        fee: "4.50",
        fixed: "",
        start: -700,
        units: &[(
            "JVT-A",
            &["401", "402", "501", "502", "601", "602", "701", "702"],
        )],
        standing: Standing::Active,
    },
    AgreementPlan {
        owner: "Mariam Obaid Al Suwaidi",
        fee: "5.00",
        fixed: "200",
        start: -740,
        units: &[(
            "JVT-B",
            &[
                "101", "102", "201", "202", "301", "302", "401", "402", "501", "502", "601", "602",
            ],
        )],
        standing: Standing::Active,
    },
    AgreementPlan {
        owner: "Sandline Holdings LLC",
        fee: "6.00",
        fixed: "500",
        start: -730,
        units: &[(
            "BBY",
            &[
                "G01", "G02", "301", "302", "401", "402", "501", "801", "802", "602",
            ],
        )],
        standing: Standing::Active,
    },
    AgreementPlan {
        owner: "Greenfield Property Investments LLC",
        fee: "5.50",
        fixed: "",
        start: -650,
        units: &[("AQZ", &["W1", "W2", "W3", "W4", "W5"])],
        standing: Standing::Active,
    },
    AgreementPlan {
        owner: "Rashed Saif Al Dhaheri",
        fee: "5.00",
        fixed: "",
        start: -600,
        units: &[("ADR", &["101", "102", "201", "401", "402"])],
        standing: Standing::Active,
    },
    AgreementPlan {
        owner: "Anwar Hossain",
        fee: "7.00",
        fixed: "100",
        start: -580,
        units: &[("SHJ", &["101", "102", "201", "202", "302", "401", "402"])],
        standing: Standing::Active,
    },
    // Being prepared.
    AgreementPlan {
        owner: "Oliver Brennan",
        fee: "5.00",
        fixed: "",
        start: 10,
        units: &[("ADR", &["302"])],
        standing: Standing::Draft,
    },
    AgreementPlan {
        owner: "Greenfield Property Investments LLC",
        fee: "5.50",
        fixed: "",
        start: 20,
        units: &[("AQZ", &["W8"])],
        standing: Standing::Draft,
    },
    // Notice given, the handover to come.
    AgreementPlan {
        owner: "Sandline Holdings LLC",
        fee: "6.00",
        fixed: "",
        start: -420,
        units: &[("BBY", &["502", "601"])],
        standing: Standing::Ending(-12, 35),
    },
    AgreementPlan {
        owner: "Mariam Obaid Al Suwaidi",
        fee: "5.00",
        fixed: "",
        start: -380,
        units: &[("ADR", &["202"])],
        standing: Standing::Ending(-5, 60),
    },
    // Handed over.
    AgreementPlan {
        owner: "Hamdan Rashid Al Ketbi",
        fee: "4.50",
        fixed: "",
        start: -520,
        units: &[("AQZ", &["W6"])],
        standing: Standing::Ended(-170, -100),
    },
    AgreementPlan {
        owner: "Rashed Saif Al Dhaheri",
        fee: "5.00",
        fixed: "",
        start: -560,
        units: &[("SHJ", &["301"])],
        standing: Standing::Ended(-180, -90),
    },
];
