//! The texts the list grids share (#557): what an empty list says creates its
//! first row. They join the app's other texts in [`crate::texts`].

/// Every key of this part, with its English text.
pub(crate) const TEXTS: &[(&str, &str)] = &[
    (
        "auto.jobs.empty_hint",
        "A job card opens when a service advisor checks a vehicle in.",
    ),
    (
        "auto.jobs.my_empty_hint",
        "The job cards a service advisor assigns to you show here until they are done.",
    ),
    (
        "auto.vehicles.empty_hint",
        "Register a vehicle with its plate and owner, or check one in on a job card.",
    ),
    (
        "auto.claims.empty_hint",
        "Register a claim with the insurer's claim number when an insured vehicle comes in.",
    ),
    (
        "auto.fleet.empty_hint",
        "Agree a contract with a fleet customer to service its vehicles at agreed rates.",
    ),
    (
        "auto.supplier.empty_hint",
        "Raise a claim from a warranty ticket that was closed as accepted.",
    ),
    (
        "auto.export.empty_hint",
        "Register a zero-rated invoice of parts that leave the UAE with its supply date.",
    ),
    (
        "auto.insurers.empty_hint",
        "Record an insurer from the parties with its claim email, terms and labour rate.",
    ),
    (
        "auto.labour.empty_hint",
        "Put an operation on the menu with its code, standard hours and price.",
    ),
];
