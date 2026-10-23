# auto

The vertical app for auto parts suppliers and fleet and workshop services
([spec](../../docs/verticals/08-auto.md)): the vehicle register, workshop job
cards with their estimates, intake condition and customer consent, parts
issued from stock, and the invoice on delivery. The app reaches the other
modules only through the contracts of `wl-kernel` (the scheduling events, the
stock service, the invoicer, the party directory), never through their crates.

The host mounts it when `client.toml` names `app = "auto"`:

```
use renox::prelude::*;

# let _ =
App::new().module(auto::Auto)
# ;
```

What is here so far:

| Part | Spec | Module |
|---|---|---|
| Permissions (`vehicles.*`, `job_cards.*`, `labour_menu.*`, and those of the insurance, fleet and supplier-claim screens) and the role names the rules use (owner, branch manager, service advisor, workshop manager, technician, counter sales, storekeeper, purchaser, accountant, insurance coordinator, fleet account manager) | §3.1, §3.3 | `permissions` |
| The starter tables the host installs: the document kinds (police accident report, claim documents, customer trade licence, fleet contract), the workshop service types and the ticket categories (warranty, vehicle incident, complaint) | §4.4, §7.1 *Booking*, §9 | `seed` |
| The cancellation reason codes 08 adds to wl-scheduling (`estimate_declined`, `duplicate_check_in`, `job_stopped`) | §7.1 *Booking* | `reasons` |
| The vehicle register: plate (emirate, code, number), VIN, make, model, year, engine, odometer and owner or fleet contract; registered once per plate and VIN; the odometer only goes up | §6 *Vehicle*, `AUT.VEH.01` | `vehicle`, `vehicles` |
| The vehicle pages: a list searched by plate, VIN, make or model and narrowed to fleet or private vehicles, the vehicle with its details and the action that registers or changes one (owner from the party picker) | §3.1, `AUT.VEH.01` | `pages::vehicles` |
| The job card (`JOB-00007`): the vehicle, customer, workshop and booking, the complaint, odometer, fuel level and intake condition acknowledged by the customer, the technician, the estimate rounds, the QC checklist and the delivery; its machine as the §7.1 table: `start_inspection`, `send_estimate`, `approve` (user and system), `decline` and `decline_added` (user and system), `wait_parts`, `parts_arrived`, `finish_work`, `pass_qc`, `fail_qc`, `deliver`, `cancel` (user and system), `void`; the guard slots the negative flows fill (`police_report`, `nothing_recorded`, `parts_requested`, `stock_issued`, `qc_findings_closed`, `substitute_closed`, `payment_cleared`, `work_done_recorded`, `acknowledged`, `approved_if_below_work_done`, `no_claim_linked`) pass for now | §6 *Job card*, §7.1 *Job card*, `AUT.JOB.01` | `job_card`, `job_card_machine`, `job_cards` |
| The estimate: labour lines (hourly, or a flat-rate operation of the labour menu) and part lines (new, used or renovated; special order), totals with VAT line by line, the work done per line and its value; a round sent to the customer with the link, the promised day and the warranty (15 days at least), answered on the link, recorded by the advisor, or declined (added work declines alone) | §4.4, §9, `AUT.JOB.02`, `AUT.JOB.03`, `FIN.QUO.01` | `labour_line`, `part_line`, `estimate`, `approval`, `approval_rounds`, `labour_operation`, `labour_menu` |
| Parts issued to a job card from the workshop's store through the kernel stock service (`consume`; a short part is a pending issue), fitted, and what is not fitted returned to stock (`return_to_stock`); the job goes on when the last part is issued | §4.4 step 5, `STK.MOV.01` | `parts` |
| The invoice on delivery through the kernel invoicer (each part with its condition), and the booking completed as the system; the check-in of a workshop booking opens the job card and links the booking (`BookingActions`) | §4.4 steps 2 and 7, §7.1 *Booking* | `billing`, `bookings` |
| The job card pages: the list (status tabs, search), the check-in form, the job card with its actions by state and role, the estimate sheet, the technician's own list (no contact details), the page the customer answers on without signing in, and the labour menu | §3.1, `AUT.ACL.08`, `AUT.JOB.01` | `pages::job_cards`, `pages::labour` |
