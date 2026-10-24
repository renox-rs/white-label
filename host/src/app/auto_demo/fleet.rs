//! The fleet contracts: Al Noor Logistics (active: vans under contract, a
//! service that falls due and is booked by the daily run, job cards that carry
//! the fleet's purchase order and are billed once a month in one invoice),
//! Desert Rent a Car (suspended for unpaid invoices) and Sharjah Cargo
//! Express (ended: its last day passed, the daily run drafted the last
//! invoice and told the contact).

use auto::fleet_billing;
use auto::fleet_contracts::{self, NewContract};
use auto::fleet_daily::run_daily_on;
use auto::fleet_invoice::KIND_MONTHLY;
use auto::fleet_services::{self, NewPlan};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use wl_kernel::documents::today;

use super::super::demo_data::{at, day};
use super::{AISHA, FATIMA, NABIL, SAMI, Seed, TAREQ};

impl Seed {
    /// A fleet job: the van is checked in under its contract with the fleet's
    /// PO, serviced, passes the QC and is delivered (no invoice of its own:
    /// the month's invoice bills it).
    async fn fleet_job(
        &mut self,
        key: &'static str,
        car: &str,
        po: &str,
        tech: &'static str,
        from: i64,
        to: i64,
    ) -> Result {
        let (card, lines) = at!(self, day(from), {
            let card = self
                .check_in(car, "Hamdan Saeed", "Periodic service", tech, 0, 0)
                .await?;
            fleet_billing::set_po(&self.state, self.who(SAMI), card.id, po).await?;
            let lines = self
                .draft(
                    card.id,
                    tech,
                    &[("Periodic service", 2_000, 12_000)],
                    &[
                        ("Oil filter", "OIL-F", 1_000, 3_500),
                        ("Brake pad set", "BRK-PAD", 1_000, 12_000),
                    ],
                )
                .await?;
            self.send(card.id).await?;
            self.approve_link(card.id, "Hamdan Saeed").await?;
            self.work(
                tech,
                &[&lines.labour[0]],
                &[&lines.parts[0], &lines.parts[1]],
            )
            .await?;
            Ok::<_, renox::Error>((card, lines))
        })?;
        let _ = lines;
        at!(self, day(to), {
            self.quality(card.id, tech, true).await?;
            self.deliver(card.id, "Hamdan Saeed").await
        })?;
        self.cards.insert(key, card.id);
        Ok(())
    }

    /// A contract with its vans, its plans and the signed document, active.
    async fn fleet_contract(
        &self,
        title: &str,
        fleet: &str,
        vans: &[(&str, i64)],
        from: i64,
        until: i64,
    ) -> Result<auto::FleetContract> {
        let manager = self.who(FATIMA).clone();
        let fleet = self.party(fleet);
        let contact = self.party("hamdan");
        at!(self, day(from), {
            let contract = fleet_contracts::create(
                &self.state,
                &manager,
                NewContract::new(
                    fleet.0,
                    title,
                    today(),
                    today() + TimeDelta::days(until - from),
                )
                .contact(contact.0)
                .labour_rate(12_000)
                .fee(200_000),
            )
            .await?;
            for (van, last_km) in vans {
                let vehicle = self.vehicle(van);
                fleet_contracts::add_vehicle(&self.state, &manager, contract.id, vehicle.id)
                    .await?;
                fleet_services::add_plan(
                    &self.state,
                    &manager,
                    contract.id,
                    NewPlan::new(vehicle.id, "periodic_service")
                        .every_km(10_000)
                        .last_service(today() - TimeDelta::days(100), *last_km),
                )
                .await?;
            }
            fleet_contracts::attach_document(
                &self.state,
                &manager,
                contract.id,
                &format!("signed-{}", title.to_lowercase().replace(' ', "-")),
            )
            .await?;
            fleet_contracts::activate(&self.state, &manager, contract.id).await
        })
    }

    /// The three contracts.
    pub(crate) async fn fleets(&mut self) -> Result {
        // Active: three vans, the first falls due at 60,000 km.
        let active = self
            .fleet_contract(
                "Al Noor Logistics service 2026",
                "noor_logistics",
                &[("urvan1", 50_000), ("urvan2", 35_000), ("hiace", 70_000)],
                -60,
                300,
            )
            .await?;
        self.fleet_job("fleet_job_billed", "urvan2", "PO-4471", TAREQ, -30, -29)
            .await?;
        // The month's invoice: one invoice, a line for each approved line.
        at!(self, day(-15), {
            fleet_billing::invoice_period(
                &self.state,
                Some(self.who(AISHA)),
                active.id,
                day(-60),
                day(-16),
                KIND_MONTHLY,
            )
            .await
        })?;
        self.fleet_job("fleet_job_open", "hiace", "PO-4502", NABIL, -7, -6)
            .await?;
        // The van that falls due is booked for the fleet, a week ahead.
        at!(self, day(-1), { run_daily_on(&self.state, day(-1)).await })?;
        self.cards.insert("fleet_contract", active.id);

        // Suspended: the fleet has unpaid invoices.
        let suspended = self
            .fleet_contract(
                "Desert Rent a Car fleet 2026",
                "desert_rent",
                &[("yaris1", 30_000), ("yaris2", 28_000)],
                -90,
                270,
            )
            .await?;
        at!(self, day(-10), {
            fleet_contracts::suspend(
                &self.state,
                self.who(FATIMA),
                suspended.id,
                "Unpaid invoices",
            )
            .await
        })?;
        self.cards.insert("fleet_suspended", suspended.id);

        // Ended: the last day was eight days ago; a job was delivered under
        // it; the daily run ends the contract, drafts the last invoice and
        // tells the contact.
        let ended = self
            .fleet_contract(
                "Sharjah Cargo Express 2026",
                "sharjah_cargo",
                &[("canter", 90_000)],
                -120,
                -8,
            )
            .await?;
        self.fleet_job("fleet_job_final", "canter", "PO-7710", TAREQ, -50, -49)
            .await?;
        at!(self, day(-6), { run_daily_on(&self.state, day(-6)).await })?;
        self.cards.insert("fleet_ended", ended.id);
        Ok(())
    }
}
