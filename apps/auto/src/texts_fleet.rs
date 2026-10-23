//! The texts of the fleet service contracts (`AUT.FLT.01`), in English. They
//! join the app's other texts in [`crate::texts`].

/// Every key of this part, with its English text.
pub(crate) const TEXTS: &[(&str, &str)] = &[
    ("auto.fleet.title", "Fleet contracts"),
    (
        "auto.fleet.subtitle",
        "Service agreements with fleets: the vehicles under contract, the rates, the service plan and the consolidated monthly invoice.",
    ),
    ("auto.fleet.new", "Agree a contract"),
    (
        "auto.fleet.new_description",
        "A contract names the fleet, the term, the labour rate and what is included. Put the vehicles under it, set the service plan and attach the signed contract before activating it.",
    ),
    ("auto.fleet.created", "Contract agreed."),
    ("auto.fleet.saved", "Contract saved."),
    ("auto.fleet.edit", "Change the terms"),
    (
        "auto.fleet.edit_description",
        "The fleet, the term and the labour rate change while the contract is a draft; after, a new rate is a renewal.",
    ),
    ("auto.fleet.number", "Contract"),
    ("auto.fleet.name", "Name"),
    ("auto.fleet.fleet", "Fleet"),
    ("auto.fleet.fleet_placeholder", "Pick the fleet"),
    ("auto.fleet.contact", "Fleet contact"),
    ("auto.fleet.contact_placeholder", "Pick the contact"),
    ("auto.fleet.start", "First day"),
    ("auto.fleet.end", "Last day of service"),
    (
        "auto.fleet.end_hint",
        "The contract is valid through this day; it ends on the first daily run after it.",
    ),
    ("auto.fleet.term", "Term"),
    ("auto.fleet.term_text", ":from to :to"),
    ("auto.fleet.labour_rate", "Labour rate per hour"),
    (
        "auto.fleet.money_hint",
        "In dirhams, for example 120 or 120.50.",
    ),
    ("auto.fleet.price_tier", "Part price tier"),
    ("auto.fleet.inclusions", "Inclusions"),
    ("auto.fleet.notice", "Notice period (days)"),
    ("auto.fleet.fee", "Early termination fee"),
    ("auto.fleet.renewal_window", "Renewal window (days)"),
    ("auto.fleet.signed", "Signed contract"),
    ("auto.fleet.unsigned", "Not attached"),
    ("auto.fleet.invoiced_through", "Invoiced through"),
    ("auto.fleet.details", "Contract"),
    ("auto.fleet.search", "Search by number or name"),
    ("auto.fleet.show", "Show"),
    ("auto.fleet.filter", "Show contracts"),
    ("auto.fleet.tab.all", "All"),
    ("auto.fleet.tab.active", "In force"),
    ("auto.fleet.tab.draft", "Drafts"),
    ("auto.fleet.tab.ended", "Ended"),
    ("auto.fleet.empty", "No contract yet."),
    ("auto.fleet.status.draft", "Draft"),
    ("auto.fleet.status.active", "Active"),
    ("auto.fleet.status.suspended", "Suspended"),
    ("auto.fleet.status.renewal_due", "Renewal due"),
    ("auto.fleet.status.ended", "Ended"),
    ("auto.fleet.document", "Attach the signed contract"),
    (
        "auto.fleet.document_description",
        "The reference of the signed contract; the contract cannot be activated without it.",
    ),
    (
        "auto.fleet.document_hint",
        "The name or reference of the signed document.",
    ),
    ("auto.fleet.document_saved", "Signed contract attached."),
    ("auto.fleet.activate", "Activate"),
    (
        "auto.fleet.activate_message",
        "The contract comes into force and the service plan starts: services that fall due are booked.",
    ),
    ("auto.fleet.activated", "Contract activated."),
    ("auto.fleet.suspend", "Suspend"),
    (
        "auto.fleet.suspend_description",
        "No new bookings are requested while the contract is suspended. The bookings already made are kept and listed for the advisor; open job cards finish.",
    ),
    ("auto.fleet.reason", "Reason"),
    (
        "auto.fleet.reason_hint",
        "For example unpaid invoices, or the fleet put the contract on hold.",
    ),
    ("auto.fleet.suspended", "Contract suspended."),
    ("auto.fleet.suspended_because", "Suspended because:"),
    ("auto.fleet.resume", "Resume"),
    (
        "auto.fleet.resume_message",
        "The contract is in force again and new bookings are requested.",
    ),
    ("auto.fleet.resumed", "Contract resumed."),
    ("auto.fleet.renew", "Renew"),
    (
        "auto.fleet.renew_description",
        "Give the new last day of service, and the new labour rate when it changes.",
    ),
    ("auto.fleet.new_end", "New last day of service"),
    (
        "auto.fleet.rate_blank_hint",
        "Leave blank to keep the rate of the contract.",
    ),
    ("auto.fleet.renewed", "Contract renewed."),
    ("auto.fleet.termination_scheduled", "Termination scheduled:"),
    (
        "auto.fleet.termination_text",
        "notice :notice, last day of service :effective",
    ),
    // The vehicles.
    ("auto.fleet.vehicles", "Vehicles under contract"),
    ("auto.fleet.vehicle", "Vehicle"),
    ("auto.fleet.vehicle_placeholder", "Pick the vehicle"),
    ("auto.fleet.vehicle_add", "Put a vehicle under contract"),
    (
        "auto.fleet.vehicle_add_description",
        "The vehicle is serviced under the contract and its job cards are billed in the monthly invoice.",
    ),
    ("auto.fleet.vehicle_added", "Vehicle put under contract."),
    ("auto.fleet.vehicle_remove", "Take a vehicle off"),
    (
        "auto.fleet.vehicle_remove_description",
        "The vehicle leaves the contract and its service plans stop. It cannot leave while a job card is open under the contract.",
    ),
    (
        "auto.fleet.vehicle_removed",
        "Vehicle taken off the contract.",
    ),
    (
        "auto.fleet.vehicles_empty",
        "No vehicle is under this contract.",
    ),
    // The service plan.
    ("auto.fleet.plans", "Service plan"),
    ("auto.fleet.plan_add", "Add a service"),
    (
        "auto.fleet.plan_add_description",
        "A service falls due by mileage, by months, or both. A week before it is due it is booked for the first free slot.",
    ),
    ("auto.fleet.service", "Service"),
    (
        "auto.fleet.service_hint",
        "The code of the service type that is booked, for example periodic_service.",
    ),
    ("auto.fleet.interval", "Every"),
    ("auto.fleet.interval_km", "Every (km)"),
    ("auto.fleet.interval_months", "Every (months)"),
    ("auto.fleet.every_km", "every :km km"),
    ("auto.fleet.every_months", "every :months months"),
    ("auto.fleet.last_date", "Last service on"),
    ("auto.fleet.last_km", "Odometer at the last service"),
    ("auto.fleet.next_due", "Next due"),
    ("auto.fleet.booking", "Booking"),
    ("auto.fleet.plan_active", "In use"),
    ("auto.fleet.plan_stopped", "Stopped"),
    ("auto.fleet.plan_added", "Service added to the plan."),
    ("auto.fleet.plan_removed", "Service taken off the plan."),
    ("auto.fleet.plans_empty", "No service is planned."),
    ("auto.fleet.bookings", "Bookings made"),
    ("auto.fleet.bookings_empty", "No booking is waiting."),
    (
        "auto.fleet.booking_note",
        "Service of fleet contract :contract",
    ),
    // The job cards and the invoices.
    ("auto.fleet.jobs", "Job cards"),
    (
        "auto.fleet.jobs_empty",
        "No job card under this contract yet.",
    ),
    ("auto.fleet.po", "PO reference"),
    ("auto.fleet.po_set", "PO reference of"),
    (
        "auto.fleet.po_description",
        "The fleet's purchase order number, printed on the invoice line.",
    ),
    ("auto.fleet.po_saved", "PO reference saved."),
    ("auto.fleet.invoice_ref", "Invoice"),
    ("auto.fleet.invoices", "Consolidated invoices"),
    (
        "auto.fleet.invoices_empty",
        "No period has been invoiced yet.",
    ),
    ("auto.fleet.invoice", "Invoice a period"),
    (
        "auto.fleet.invoice_description",
        "One invoice lists the job cards delivered up to the last day, a line for each, with the vehicle and the PO reference. It follows the period invoiced last.",
    ),
    ("auto.fleet.invoiced", "Period invoiced."),
    ("auto.fleet.from", "First day"),
    ("auto.fleet.to", "Last day"),
    ("auto.fleet.period", "Period"),
    ("auto.fleet.invoice_kind", "Kind"),
    ("auto.fleet.invoice_kind.monthly", "Monthly"),
    ("auto.fleet.invoice_kind.final", "Final"),
    ("auto.fleet.net", "Before VAT"),
    // What is refused.
    ("auto.fleet.problem.title", "Give the contract a name."),
    (
        "auto.fleet.problem.term",
        "The last day of service must be after the first day.",
    ),
    (
        "auto.fleet.problem.start",
        "Give the first day of the term.",
    ),
    ("auto.fleet.problem.end", "Give the last day of service."),
    (
        "auto.fleet.problem.negative",
        "A rate, a fee or a number of days cannot be less than zero.",
    ),
    (
        "auto.fleet.problem.party",
        "Pick the fleet and its contact from the parties.",
    ),
    ("auto.fleet.problem.ended", "This contract has ended."),
    (
        "auto.fleet.problem.locked",
        "The fleet, the term and the labour rate of a contract in force change by renewing it.",
    ),
    (
        "auto.fleet.problem.document",
        "Give the reference of the signed contract.",
    ),
    (
        "auto.fleet.problem.vehicle",
        "Pick a vehicle of the register that is serviced here.",
    ),
    (
        "auto.fleet.problem.other_contract",
        "That vehicle is under another fleet contract.",
    ),
    (
        "auto.fleet.problem.not_on_contract",
        "That vehicle is not under this contract.",
    ),
    (
        "auto.fleet.problem.job_open",
        "A job card of that vehicle is open under the contract; it finishes first.",
    ),
    (
        "auto.fleet.problem.vehicles",
        "Put at least one vehicle under the contract first.",
    ),
    ("auto.fleet.problem.rates", "Agree a labour rate first."),
    (
        "auto.fleet.problem.schedule",
        "Set the service plan of at least one vehicle first.",
    ),
    (
        "auto.fleet.problem.signed",
        "Attach the signed contract first.",
    ),
    (
        "auto.fleet.problem.not_in_window",
        "The contract is not in its renewal window yet.",
    ),
    (
        "auto.fleet.problem.new_end",
        "Give a last day of service after the current one.",
    ),
    (
        "auto.fleet.problem.not_over",
        "The last day of service has not passed.",
    ),
    (
        "auto.fleet.problem.termination_dates",
        "Give the notice date and the effective date.",
    ),
    (
        "auto.fleet.problem.effective_before_notice",
        "The effective date cannot be before the notice date.",
    ),
    (
        "auto.fleet.problem.effective_invoiced",
        "The effective date is inside a period that is invoiced already.",
    ),
    (
        "auto.fleet.problem.service_code",
        "Give the code of the service type.",
    ),
    (
        "auto.fleet.problem.interval",
        "Give a mileage, a number of months or both, each above zero.",
    ),
    (
        "auto.fleet.problem.last_service",
        "The last service cannot be in the future.",
    ),
    (
        "auto.fleet.problem.job_billed",
        "That job card is billed already.",
    ),
    (
        "auto.fleet.problem.not_in_force",
        "The contract is still a draft.",
    ),
    (
        "auto.fleet.problem.period",
        "Give a period that is over: the first day, and a last day before today.",
    ),
    (
        "auto.fleet.problem.period_billed",
        "That period overlaps one that is invoiced already.",
    ),
    (
        "auto.fleet.problem.period_gap",
        "The period must follow the one invoiced last.",
    ),
    (
        "auto.fleet.problem.invoicing_off",
        "Invoicing is not available.",
    ),
    // Suspending, ending and terminating (BL-AUT-13, BL-AUT-12).
    ("auto.fleet.terminate", "Terminate early"),
    (
        "auto.fleet.terminate_description",
        "Ask to end the contract before its last day. The owner decides under Needs your action; nothing changes until then. The last day of service is still served.",
    ),
    ("auto.fleet.terminate_submit", "Ask the owner"),
    ("auto.fleet.reason_placeholder", "Pick the reason"),
    ("auto.fleet.reason_note", "More about the reason"),
    ("auto.fleet.notice_date", "Notice given on"),
    (
        "auto.fleet.notice_date_hint",
        "The day the fleet or the workshop gave notice.",
    ),
    ("auto.fleet.effective_date", "Last day of service"),
    (
        "auto.fleet.effective_date_hint",
        "Still served at the contract rates. The contract ends on the first daily run after it.",
    ),
    ("auto.fleet.waive_fee", "Waive the early termination fee"),
    (
        "auto.fleet.waive_fee_hint",
        "Only the owner's approval waives the fee.",
    ),
    (
        "auto.fleet.termination_waits",
        "Termination requested. It waits for the owner's approval.",
    ),
    ("auto.fleet.terminated", "Contract terminated."),
    (
        "auto.fleet.termination_pending",
        "A termination waits for the owner's approval. The contract is unchanged until it is approved.",
    ),
    ("auto.fleet.termination.fleet_sold", "Fleet sold"),
    ("auto.fleet.termination.non_payment", "Non-payment"),
    ("auto.fleet.termination.breach", "Breach"),
    ("auto.fleet.termination.agreement", "By agreement"),
    (
        "auto.fleet.bookings_suspended",
        "The contract is suspended: no new booking is made for its vehicles. Confirm or cancel the bookings already made.",
    ),
    ("auto.fleet.booking_confirm", "Confirm the booking of"),
    ("auto.fleet.booking_confirm_message", "The booking is kept."),
    ("auto.fleet.booking_confirmed_done", "Booking confirmed."),
    ("auto.fleet.booking_cancel", "Cancel the booking of"),
    (
        "auto.fleet.booking_cancel_message",
        "The booking is cancelled and the service is asked for again once the contract is resumed.",
    ),
    ("auto.fleet.booking_cancelled", "Booking cancelled."),
    ("auto.fleet.booking_confirmed", "Confirmed"),
    ("auto.fleet.booking_unconfirmed", "To confirm or cancel"),
    (
        "auto.fleet.fee_line",
        "Early termination fee, fleet contract :contract",
    ),
    (
        "auto.fleet.notice_ended_title",
        "Fleet contract :contract has ended",
    ),
    (
        "auto.fleet.notice_ended_body",
        "Your service contract :contract (:title) ended after its last day of service, :date. Services are no longer booked under it, and the last invoice for the period up to that day is on its way. Job cards still open finish at the contract rates.",
    ),
    (
        "auto.fleet.notice_terminated_title",
        "Fleet contract :contract is terminated",
    ),
    (
        "auto.fleet.notice_terminated_body",
        "Your service contract :contract (:title) is terminated with the last day of service :date. Services are no longer booked under it, and the final invoice for the period up to that day is on its way. Job cards still open finish at the contract rates.",
    ),
    (
        "auto.fleet.notice_task_title",
        "Tell the fleet contact that :contract ended",
    ),
    (
        "auto.fleet.notice_task_body",
        "The message to the fleet contact of :contract could not be sent. Call or write to them: the contract ended and the last invoice is drafted.",
    ),
    (
        "auto.fleet.problem.already_scheduled",
        "A termination of this contract is scheduled already.",
    ),
    (
        "auto.fleet.problem.approval",
        "The termination needs the owner's approval, and no approval policy is in force.",
    ),
    (
        "auto.fleet.problem.suspended_booking",
        "Fleet contract :contract is suspended: no new booking is made for its vehicles.",
    ),
    (
        "auto.fleet.problem.terminate_status",
        "Only a contract in force can be terminated.",
    ),
    (
        "auto.fleet.problem.reason",
        "Pick the reason: fleet sold, non-payment, breach or agreement.",
    ),
    (
        "auto.fleet.problem.no_booking",
        "That service has no booking waiting.",
    ),
    (
        "auto.fleet.problem.booking_kept",
        "That booking cannot be cancelled any more: it has started or was cancelled.",
    ),
];
