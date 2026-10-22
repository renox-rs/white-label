//! BL-MPW-10 (#424), `MPW.ACL.15`: a worker is not at two client sites at the
//! same hours. `activate` and `extend` are refused when the worker has an
//! `active` or `suspended` deployment whose shift times overlap, through the
//! real routes; windows that touch do not overlap, a `planned`, `ended` or
//! `cancelled` deployment never blocks, and two activations at once leave one.

mod support;

use manpower::{ClientContract, ContractStatus, Deployment, DeploymentStatus, Site};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::workforce::EmployeeId;

use support::{Booted, TOWER, boot, date};

/// A deployment of `employee` at `site` in `status`, working `days` (none:
/// every day) from `from` to `to`, from `start` to `end` o'clock on 1 February
/// 2027 or the date given, as a fixture.
#[allow(clippy::too_many_arguments)]
async fn shift(
    t: &Booted,
    contract: &ClientContract,
    site: &Site,
    employee: EmployeeId,
    status: DeploymentStatus,
    days: Option<&str>,
    window: (&str, &str),
    range: (NaiveDate, Option<NaiveDate>),
) -> Deployment {
    let mut deployment = Deployment {
        employee_id: employee.0,
        client_contract_id: contract.id,
        client_party_id: contract.client_party_id,
        site_id: site.id,
        role: "guard".into(),
        shift_type: "day".into(),
        shift_start: Some(window.0.into()),
        shift_end: Some(window.1.into()),
        work_days: days.map(str::to_owned),
        start_date: range.0,
        end_date: range.1,
        status,
        ..Deployment::default()
    };
    let mut tx = t.db().begin().await.unwrap();
    deployment
        .insert_in_state(&mut tx, status, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    deployment
}

const MON_SAT: &str = "mon,tue,wed,thu,fri,sat";

/// What the tests share: Amal, the contract, two sites and worker W.
struct World {
    t: Booted,
    amal: User,
    contract: ClientContract,
    a: Site,
    b: Site,
    w: EmployeeId,
}

async fn world() -> World {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    let a = t.site(TOWER, "Tower A").await;
    let b = t.site(TOWER, "Tower B").await;
    let w = t.staff.employ(&t.parties, 7, "Wasim Khan", true);
    World {
        t,
        amal,
        contract,
        a,
        b,
        w,
    }
}

impl World {
    /// `path` of deployment `d` on `day`, posted by Amal.
    async fn act(
        &self,
        day: NaiveDate,
        d: &Deployment,
        action: &str,
        form: &[(&str, &str)],
    ) -> u16 {
        let path = format!("/manpower/deployments/{}/{action}", d.id);
        self.t
            .on(day, async { self.t.post(&self.amal, &path, form).await })
            .await
            .status
            .as_u16()
    }
}

/// AC1: W is active at site A Monday to Saturday 07:00-15:00 to 2027-03-31;
/// activating W at site B 14:00-22:00 from 2027-02-01 is refused naming the
/// deployment at site A, and nothing changes.
#[renox::test]
async fn an_overlapping_shift_at_another_site_is_refused_naming_the_deployment() {
    let w = world().await;
    let at_a = shift(
        &w.t,
        &w.contract,
        &w.a,
        w.w,
        DeploymentStatus::Active,
        Some(MON_SAT),
        ("07:00", "15:00"),
        (date(2027, 1, 4), Some(date(2027, 3, 31))),
    )
    .await;
    let at_b = shift(
        &w.t,
        &w.contract,
        &w.b,
        w.w,
        DeploymentStatus::Planned,
        Some(MON_SAT),
        ("14:00", "22:00"),
        (date(2027, 2, 1), Some(date(2027, 6, 30))),
    )
    .await;

    w.t.on(date(2027, 2, 1), async {
        let refused =
            w.t.post(
                &w.amal,
                &format!("/manpower/deployments/{}/activate", at_b.id),
                &[],
            )
            .await;
        refused.assert_status(422);
        let body = refused.text();
        assert!(body.contains(&format!("deployment #{}", at_a.id)), "{body}");
        assert!(body.contains("Tower A"), "{body}");
        assert!(body.contains("07:00-15:00"), "{body}");
    })
    .await;
    assert_eq!(
        w.t.reload_deployment(at_b.id).await.status,
        DeploymentStatus::Planned
    );
}

/// AC2: the same, but site B 15:00-23:00 (the windows touch): allowed.
#[renox::test]
async fn windows_that_touch_do_not_overlap() {
    let w = world().await;
    shift(
        &w.t,
        &w.contract,
        &w.a,
        w.w,
        DeploymentStatus::Active,
        Some(MON_SAT),
        ("07:00", "15:00"),
        (date(2027, 1, 4), Some(date(2027, 3, 31))),
    )
    .await;
    let at_b = shift(
        &w.t,
        &w.contract,
        &w.b,
        w.w,
        DeploymentStatus::Planned,
        Some(MON_SAT),
        ("15:00", "23:00"),
        (date(2027, 2, 1), Some(date(2027, 6, 30))),
    )
    .await;
    assert_eq!(w.act(date(2027, 2, 1), &at_b, "activate", &[]).await, 303);
    assert_eq!(
        w.t.reload_deployment(at_b.id).await.status,
        DeploymentStatus::Active
    );
}

/// AC3: W's deployment at site B (Fridays 08:00-12:00) ends 2027-02-26; site
/// A (Fridays 07:00-15:00) runs from 2027-03-01. Extending B into March is
/// refused; extending it over the weekend, where A does not work, is allowed.
#[renox::test]
async fn extending_into_dates_where_the_other_site_runs_is_refused() {
    let w = world().await;
    let at_b = shift(
        &w.t,
        &w.contract,
        &w.b,
        w.w,
        DeploymentStatus::Active,
        Some("fri"),
        ("08:00", "12:00"),
        (date(2027, 2, 1), Some(date(2027, 2, 26))),
    )
    .await;
    shift(
        &w.t,
        &w.contract,
        &w.a,
        w.w,
        DeploymentStatus::Active,
        Some("fri"),
        ("07:00", "15:00"),
        (date(2027, 3, 1), Some(date(2027, 6, 30))),
    )
    .await;

    let day = date(2027, 2, 10);
    assert_eq!(
        w.act(day, &at_b, "extend", &[("end_date", "2027-03-31")])
            .await,
        422
    );
    assert_eq!(
        w.t.reload_deployment(at_b.id).await.end_date,
        Some(date(2027, 2, 26))
    );
    assert_eq!(
        w.act(day, &at_b, "extend", &[("end_date", "2027-02-28")])
            .await,
        303
    );
    assert_eq!(
        w.t.reload_deployment(at_b.id).await.end_date,
        Some(date(2027, 2, 28))
    );
}

/// AC4: W's deployment at site A is `suspended`: the overlapping activation at
/// site B is still refused.
#[renox::test]
async fn a_suspended_deployment_still_blocks() {
    let w = world().await;
    shift(
        &w.t,
        &w.contract,
        &w.a,
        w.w,
        DeploymentStatus::Suspended,
        None,
        ("07:00", "15:00"),
        (date(2027, 1, 4), Some(date(2027, 3, 31))),
    )
    .await;
    let at_b = shift(
        &w.t,
        &w.contract,
        &w.b,
        w.w,
        DeploymentStatus::Planned,
        None,
        ("10:00", "18:00"),
        (date(2027, 2, 1), Some(date(2027, 6, 30))),
    )
    .await;
    assert_eq!(w.act(date(2027, 2, 1), &at_b, "activate", &[]).await, 422);
}

/// Must not: a deployment that is `planned`, `ended` or `cancelled` never
/// blocks; nor does one on other weekdays or in other dates.
#[renox::test]
async fn only_active_and_suspended_deployments_on_common_days_block() {
    let w = world().await;
    for status in [
        DeploymentStatus::Planned,
        DeploymentStatus::Ended,
        DeploymentStatus::Cancelled,
    ] {
        shift(
            &w.t,
            &w.contract,
            &w.a,
            w.w,
            status,
            None,
            ("07:00", "15:00"),
            (date(2027, 1, 4), Some(date(2027, 3, 31))),
        )
        .await;
    }
    // Active, but on Sundays only, and in other dates.
    shift(
        &w.t,
        &w.contract,
        &w.a,
        w.w,
        DeploymentStatus::Active,
        Some("sun"),
        ("07:00", "15:00"),
        (date(2027, 1, 4), Some(date(2027, 3, 31))),
    )
    .await;
    shift(
        &w.t,
        &w.contract,
        &w.a,
        w.w,
        DeploymentStatus::Active,
        None,
        ("07:00", "15:00"),
        (date(2027, 8, 2), Some(date(2027, 9, 30))),
    )
    .await;
    let at_b = shift(
        &w.t,
        &w.contract,
        &w.b,
        w.w,
        DeploymentStatus::Planned,
        Some(MON_SAT),
        ("08:00", "16:00"),
        (date(2027, 2, 1), Some(date(2027, 6, 30))),
    )
    .await;
    assert_eq!(w.act(date(2027, 2, 1), &at_b, "activate", &[]).await, 303);
}

/// A night shift (22:00-06:00) reaches into the next morning: a day shift that
/// starts at 05:00 meets it, one that starts at 06:00 does not.
#[renox::test]
async fn a_shift_through_midnight_is_compared_across_the_day_change() {
    let w = world().await;
    shift(
        &w.t,
        &w.contract,
        &w.a,
        w.w,
        DeploymentStatus::Active,
        None,
        ("22:00", "06:00"),
        (date(2027, 1, 4), Some(date(2027, 6, 30))),
    )
    .await;
    let early = shift(
        &w.t,
        &w.contract,
        &w.b,
        w.w,
        DeploymentStatus::Planned,
        None,
        ("05:00", "13:00"),
        (date(2027, 2, 1), Some(date(2027, 6, 30))),
    )
    .await;
    let after = shift(
        &w.t,
        &w.contract,
        &w.b,
        w.w,
        DeploymentStatus::Planned,
        None,
        ("06:00", "14:00"),
        (date(2027, 2, 1), Some(date(2027, 6, 30))),
    )
    .await;
    assert_eq!(w.act(date(2027, 2, 1), &early, "activate", &[]).await, 422);
    assert_eq!(w.act(date(2027, 2, 1), &after, "activate", &[]).await, 303);
}

/// Concurrency: two overlapping planned deployments of one worker are
/// activated at once; one goes active, the other is refused.
#[renox::test]
async fn the_second_of_two_parallel_activations_is_refused() {
    let w = world().await;
    let first = shift(
        &w.t,
        &w.contract,
        &w.a,
        w.w,
        DeploymentStatus::Planned,
        None,
        ("07:00", "15:00"),
        (date(2027, 2, 1), Some(date(2027, 6, 30))),
    )
    .await;
    let second = shift(
        &w.t,
        &w.contract,
        &w.b,
        w.w,
        DeploymentStatus::Planned,
        None,
        ("10:00", "18:00"),
        (date(2027, 2, 1), Some(date(2027, 6, 30))),
    )
    .await;
    let (one, two) =
        w.t.on(date(2027, 2, 1), async {
            let one = format!("/manpower/deployments/{}/activate", first.id);
            let two = format!("/manpower/deployments/{}/activate", second.id);
            renox::tokio::join!(w.t.post(&w.amal, &one, &[]), w.t.post(&w.amal, &two, &[]))
        })
        .await;
    let mut codes = [one.status.as_u16(), two.status.as_u16()];
    codes.sort_unstable();
    assert_eq!(codes, [303, 422], "one activation is refused");
    let active = Deployment::query()
        .where_(Deployment::STATUS.eq(DeploymentStatus::Active))
        .get(w.t.db())
        .await
        .unwrap();
    assert_eq!(active.len(), 1);
}

/// Planning keeps the working days (lower case) and refuses a list that is not
/// weekdays or a shift time that is not HH:MM.
#[renox::test]
async fn the_working_days_and_the_shift_times_are_checked_when_planning() {
    let w = world().await;
    let contract = w.contract.id.to_string();
    let site = w.a.id.to_string();
    let employee = w.w.0.to_string();
    let form = |days: &str, start: &str| {
        vec![
            ("employee_id", employee.clone()),
            ("client_contract_id", contract.clone()),
            ("site_id", site.clone()),
            ("role", "guard".to_owned()),
            ("shift_type", "day".to_owned()),
            ("shift_start", start.to_owned()),
            ("shift_end", "15:00".to_owned()),
            ("work_days", days.to_owned()),
            ("start_date", "2027-02-01".to_owned()),
        ]
    };
    for (days, start) in [("mon,funday", "07:00"), ("mon", "7 o'clock")] {
        let fields = form(days, start);
        let borrowed: Vec<(&str, &str)> = fields.iter().map(|(k, v)| (*k, v.as_str())).collect();
        w.t.post(&w.amal, "/manpower/deployments", &borrowed)
            .await
            .assert_status(422);
    }
    let fields = form("Mon, FRI", "07:00");
    let borrowed: Vec<(&str, &str)> = fields.iter().map(|(k, v)| (*k, v.as_str())).collect();
    w.t.post(&w.amal, "/manpower/deployments", &borrowed)
        .await
        .assert_status(303);
    let saved = Deployment::query().first(w.t.db()).await.unwrap().unwrap();
    assert_eq!(saved.work_days.as_deref(), Some("mon, fri"));
}
