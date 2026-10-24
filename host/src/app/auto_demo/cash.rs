//! Cheques collected in the field and confirmed in the cash handover: the
//! counter ticks each cheque present or missing; a missing one makes the
//! handover short (it is never cash), and is returned as lost with a request
//! for a replacement.

use renox::prelude::*;
use wl_kernel::cheques::{RegisterCheque, cheques as cheque_contract};
use wl_payments::{CashHandover, Cheque};

use super::super::demo_data::{at, day};
use super::{AISHA, HUSSEIN, Seed};

impl Seed {
    async fn field_cheque(&self, party: &str, number: &str, amount: i64) -> Result<Cheque> {
        let summary = cheque_contract(&self.state)
            .register(
                &self.state,
                RegisterCheque::new(
                    self.party(party),
                    self.entity,
                    "ENBD",
                    number,
                    wl_kernel::documents::today(),
                    amount,
                )
                .collected_by(self.user(HUSSEIN).id, true),
                None,
            )
            .await?;
        Cheque::find_or_404(self.db(), summary.id.0).await
    }

    pub(crate) async fn cash(&mut self) -> Result {
        // Two cheques collected on one day: one is ticked present and
        // deposited, the other is missing at the count.
        let (first, second) = at!(self, day(-12), {
            let first = self.field_cheque("quick_fit", "300101", 1_250_000).await?;
            let second = self.field_cheque("movers", "300102", 840_000).await?;
            Ok::<_, renox::Error>((first, second))
        })?;
        let handover = CashHandover::query()
            .where_(CashHandover::HOLDER_USER_ID.eq(self.user(HUSSEIN).id))
            .order_by(CashHandover::ID)
            .first(self.db())
            .await?
            .ok_or_else(|| {
                renox::anyhow::anyhow!("demo seed: no handover for the field cheques")
            })?;
        let shown = format!("{},{}", first.id, second.id);
        let present = first.id.to_string();
        at!(self, day(-11), {
            self.driver
                .post(
                    AISHA,
                    &format!("/cash/{}/count", handover.id),
                    &[
                        ("counted", "0"),
                        ("holder_present", "yes"),
                        ("shown", shown.as_str()),
                        ("present", present.as_str()),
                    ],
                )
                .await?;
            self.driver
                .post(AISHA, &format!("/cheques/{}/deposit", first.id), &[])
                .await
        })?;
        at!(self, day(-10), {
            self.driver
                .post(
                    AISHA,
                    &format!("/cash/{}/cheques/{}/lost", handover.id, second.id),
                    &[("x", "")],
                )
                .await?;
            self.driver
                .post(AISHA, &format!("/cash/{}/recover", handover.id), &[])
                .await
        })?;
        // A cheque collected yesterday waits for the next count.
        at!(self, day(-1), {
            self.field_cheque("quick_fit", "300103", 760_000).await
        })?;
        Ok(())
    }
}
