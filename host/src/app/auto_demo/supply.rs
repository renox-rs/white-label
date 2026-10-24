//! The supplier warranty claims: a part that failed under warranty, the
//! customer's warranty ticket closed as accepted, the purchaser's claim to the
//! supplier, and what the supplier answered: nothing yet (raised), accepted
//! (the credit follows), credited (recovered), or refused (the write-off waits
//! for the branch manager).

use auto::SupplierAnswer;
use auto::supplier_claims::{self, NewSupplierClaim};
use renox::prelude::*;
use wl_kernel::tickets::{OpenTicket, task_creator};

use super::super::demo_data::{at, day};
use super::{PIA, Seed};

/// A claim of the story: the job card it is about, the part, the supplier,
/// the stock code and brand, the cost, the day the ticket opened and the day
/// the claim was raised.
struct Story {
    card: &'static str,
    part: &'static str,
    code: &'static str,
    brand: &'static str,
    supplier: &'static str,
    cost: i64,
    ticket: i64,
    raised: i64,
}

const STORIES: &[Story] = &[
    Story {
        card: "delivered_pajero",
        part: "Alternator",
        code: "ALT-9",
        brand: "Bosch",
        supplier: "gulf_spares",
        cost: 45_000,
        ticket: -40,
        raised: -38,
    },
    Story {
        card: "delivered_pajero_battery",
        part: "Battery 70Ah",
        code: "BAT-70",
        brand: "Varta",
        supplier: "gulf_spares",
        cost: 31_000,
        ticket: -25,
        raised: -24,
    },
    Story {
        card: "delivered_civic",
        part: "Spark plug set (4)",
        code: "SPARK-4",
        brand: "NGK",
        supplier: "gulf_spares",
        cost: 6_000,
        ticket: -9,
        raised: -8,
    },
    Story {
        card: "delivered_hilux",
        part: "Oil filter",
        code: "OIL-F",
        brand: "Mann",
        supplier: "oil_house",
        cost: 3_500,
        ticket: -30,
        raised: -29,
    },
];

impl Seed {
    /// The four claims.
    pub(crate) async fn supplier_claims(&mut self) -> Result {
        let pia = self.who(PIA).clone();
        let mut claims = Vec::new();
        for story in STORIES {
            let card = auto::JobCard::find_or_404(self.db(), self.cards[story.card]).await?;
            let supplier = self.party(story.supplier);
            let ticket = at!(self, day(story.ticket), {
                let creator = task_creator(&self.state);
                let ticket = creator
                    .open(
                        &self.state,
                        OpenTicket::new(
                            card.subject(),
                            "warranty",
                            &format!("{} failed after the repair", story.part),
                        ),
                    )
                    .await?
                    .ok_or_else(|| renox::anyhow::anyhow!("demo seed: ticketing is off"))?;
                creator
                    .complete(
                        &self.state,
                        ticket.id,
                        &format!(
                            "Accepted: replace the {} under warranty",
                            story.part.to_lowercase()
                        ),
                    )
                    .await?;
                Ok::<_, renox::Error>(ticket)
            })?;
            let claim = at!(self, day(story.raised), {
                supplier_claims::raise(
                    &self.state,
                    &pia,
                    NewSupplierClaim::new(
                        ticket.id.0,
                        card.subject(),
                        supplier.0,
                        story.part,
                        story.cost,
                        day(-148),
                        12,
                        day(story.ticket),
                    )
                    .item(story.code, story.brand),
                )
                .await
            })?;
            claims.push(claim);
        }
        // Recovered: the supplier accepted and credited.
        at!(self, day(-30), {
            supplier_claims::record_answer(
                &self.state,
                &pia,
                claims[0].id,
                SupplierAnswer::Accepted,
                Some("RMA-5520"),
                None,
            )
            .await?;
            supplier_claims::recover(&self.state, &pia, claims[0].id, Some("CN-GAS-2201"), false)
                .await
        })?;
        // Accepted, the credit note still to come.
        at!(self, day(-20), {
            supplier_claims::record_answer(
                &self.state,
                &pia,
                claims[1].id,
                SupplierAnswer::Accepted,
                Some("RMA-5581"),
                Some("Replacement battery in transit"),
            )
            .await
        })?;
        // Refused: the write-off waits for the branch manager.
        at!(self, day(-15), {
            supplier_claims::record_answer(
                &self.state,
                &pia,
                claims[3].id,
                SupplierAnswer::Rejected,
                Some("RMA-5602"),
                Some("Fitted with a worn gasket, outside the warranty"),
            )
            .await?;
            supplier_claims::write_off(
                &self.state,
                &pia,
                claims[3].id,
                "Supplier refused: fitted with a worn gasket",
            )
            .await
        })?;
        Ok(())
    }
}
