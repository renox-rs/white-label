//! The texts of the vehicle register (`AUT.VEH.01`), in English. They join the
//! app's other texts in [`crate::texts`].

/// Every key of this part, with its English text.
pub(crate) const TEXTS: &[(&str, &str)] = &[
    ("auto.vehicles.kind", "Kind"),
    (
        "auto.vehicles.fleet_contract_placeholder",
        "Not under a fleet contract",
    ),
    ("auto.vehicles.title", "Vehicles"),
    (
        "auto.vehicles.subtitle",
        "The vehicles of the workshop's customers and fleets, with their plate, VIN, odometer and owner.",
    ),
    ("auto.vehicles.new", "Register a vehicle"),
    (
        "auto.vehicles.new_description",
        "A vehicle has a UAE plate, a make and a model, and an owner. A fleet vehicle also names its fleet contract.",
    ),
    ("auto.vehicles.registered", "Vehicle registered."),
    ("auto.vehicles.saved", "Vehicle saved."),
    ("auto.vehicles.edit", "Change the details"),
    (
        "auto.vehicles.edit_description",
        "The odometer can only go up; the plate changes when the vehicle is re-registered.",
    ),
    ("auto.vehicles.plate", "Plate"),
    ("auto.vehicles.emirate", "Emirate"),
    ("auto.vehicles.plate_code", "Plate code"),
    (
        "auto.vehicles.plate_code_hint",
        "A letter or a number, for example A or 22.",
    ),
    ("auto.vehicles.plate_number", "Plate number"),
    ("auto.vehicles.vin", "VIN"),
    (
        "auto.vehicles.vin_hint",
        "17 characters; leave blank when it is not known yet.",
    ),
    ("auto.vehicles.make", "Make"),
    ("auto.vehicles.model", "Model"),
    ("auto.vehicles.year", "Year"),
    ("auto.vehicles.engine", "Engine"),
    ("auto.vehicles.colour", "Colour"),
    ("auto.vehicles.owner", "Owner"),
    (
        "auto.vehicles.owner_placeholder",
        "Pick the owner or fleet company, or give the party code, for example P-00023.",
    ),
    ("auto.vehicles.fleet_contract", "Fleet contract"),
    (
        "auto.vehicles.fleet_contract_hint",
        "The fleet contract, when the vehicle is serviced under one.",
    ),
    ("auto.vehicles.odometer", "Odometer (km)"),
    ("auto.vehicles.odometer_at", "Reading taken"),
    ("auto.vehicles.next_service", "Next service"),
    ("auto.vehicles.next_service_none", "Not recorded"),
    ("auto.vehicles.next_service_at", ":date or at :km km"),
    ("auto.vehicles.details", "Vehicle"),
    ("auto.vehicles.history", "Job cards"),
    ("auto.vehicles.history_empty", "No job card yet."),
    ("auto.vehicles.empty", "No vehicle registered yet."),
    ("auto.vehicles.search", "Plate, VIN, make or model"),
    ("auto.vehicles.show", "Show"),
    ("auto.vehicles.filter", "Which vehicles"),
    ("auto.vehicles.filter.all", "All"),
    ("auto.vehicles.filter.fleet", "Fleet"),
    ("auto.vehicles.filter.private", "Private"),
    ("auto.vehicles.kind.fleet", "Fleet"),
    ("auto.vehicles.kind.private", "Private"),
    ("auto.vehicles.inactive", "Not serviced here any more"),
    (
        "auto.vehicles.problem.plate_code",
        "The plate code is a letter or a number of up to three characters, for example A or 22.",
    ),
    (
        "auto.vehicles.problem.plate_number",
        "The plate number is one to six digits, for example 12345.",
    ),
    (
        "auto.vehicles.problem.emirate",
        "Pick the emirate of the plate.",
    ),
    (
        "auto.vehicles.problem.vin",
        "A VIN has 17 capital letters and digits, without I, O and Q.",
    ),
    (
        "auto.vehicles.problem.year",
        "Give the model year as four digits, between 1950 and next year.",
    ),
    (
        "auto.vehicles.problem.make",
        "Give the make, for example Toyota.",
    ),
    (
        "auto.vehicles.problem.model",
        "Give the model, for example Hilux.",
    ),
    (
        "auto.vehicles.problem.owner",
        "Pick the owner from the list, or give the party code, for example P-00023.",
    ),
    (
        "auto.vehicles.problem.plate_taken",
        ":plate is registered already.",
    ),
    (
        "auto.vehicles.problem.vin_taken",
        "The VIN :vin is registered already on :plate.",
    ),
    (
        "auto.vehicles.problem.odometer",
        "The odometer cannot read below the last reading of :km km.",
    ),
    (
        "auto.vehicles.problem.odometer_negative",
        "The odometer reading cannot be negative.",
    ),
];
