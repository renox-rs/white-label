//! The fleet contracts: Emirates Fleet Rentals' two Hiace vans under an
//! active contract (signed copy on file, a service plan per van, its own
//! labour rate), and Gulf Build Contracting's contract for its pickup still
//! in draft, waiting for the signed copy.

use renox::prelude::*;

use super::{FLEET_EMAIL, Seed, day};

impl Seed {
    /// A contract of the fleet `fleet` (a party key) for `title`, in draft;
    /// its id.
    async fn contract(
        &self,
        fleet: &str,
        contact: Option<&str>,
        title: &str,
        rate: &str,
    ) -> Result<i64> {
        let mut form = vec![
            ("fleet", self.parties[fleet].0.to_string()),
            ("title", title.to_owned()),
            ("start_date", day(-90).to_string()),
            ("end_date", day(275).to_string()),
            ("labour_rate", rate.to_owned()),
            (
                "inclusions",
                "Periodic service, brakes, tyres rotation; body work quoted apart".to_owned(),
            ),
            ("notice_days", "30".to_owned()),
            ("fee", "1,500".to_owned()),
            ("renewal_window", "60".to_owned()),
        ];
        if let Some(contact) = contact {
            form.push(("contact", self.parties[contact].0.to_string()));
        }
        let pairs: Vec<(&str, &str)> = form.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let made = self.driver.post(FLEET_EMAIL, "/auto/fleet", &pairs).await?;
        made.redirected_id()
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the contract of {fleet}").into())
    }

    /// The fleet contracts.
    pub(crate) async fn fleet(&mut self) -> Result {
        let efr = self
            .contract(
                "efr",
                Some("efr_contact"),
                "Maintenance of the Hiace delivery vans",
                "195",
            )
            .await?;
        self.driver
            .post(
                FLEET_EMAIL,
                &format!("/auto/fleet/{efr}/document"),
                &[("document", "EFR-ANAP-2026-FMC signed 2 pages")],
            )
            .await?;
        for (van, last_km) in [("van1", "35,000"), ("van2", "40,000")] {
            self.driver
                .post(
                    FLEET_EMAIL,
                    &format!("/auto/fleet/{efr}/vehicles"),
                    &[("vehicle", &self.vehicles[van].to_string())],
                )
                .await?;
            self.driver
                .post(
                    FLEET_EMAIL,
                    &format!("/auto/fleet/{efr}/plans"),
                    &[
                        ("vehicle", &self.vehicles[van].to_string()),
                        ("service", "periodic_service"),
                        ("interval_km", "10000"),
                        ("interval_months", "6"),
                        ("last_date", &day(-150).to_string()),
                        ("last_km", last_km),
                    ],
                )
                .await?;
        }
        self.driver
            .post(FLEET_EMAIL, &format!("/auto/fleet/{efr}/activate"), &[])
            .await?;

        let build = self
            .contract("gulfbuild", None, "Site pickups: service and brakes", "210")
            .await?;
        self.driver
            .post(
                FLEET_EMAIL,
                &format!("/auto/fleet/{build}/vehicles"),
                &[("vehicle", &self.vehicles["hilux"].to_string())],
            )
            .await?;
        Ok(())
    }
}
