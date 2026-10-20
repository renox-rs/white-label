//! Filing a return (04 §3.3 `ACC.ACL.15`, §7.1 *Return* `file`; BL-ACC-19).
//!
//! A return is filed only by a member of staff whose FTA tax agent
//! registration is valid on the filing date and linked to the firm's tax
//! agency, or recorded as filed by the client through its own EmaraTax user
//! ("filed by the client's own staff"; L-16 is open: the app assumes the
//! filer is a registered agent or the client's own staff). The filing date is
//! the date on the FTA acknowledgement, entered by the filer.
//!
//! The check is the qualification guard `qualification_valid`
//! (`HRM.QUA.01`) of the kernel ([`wl_kernel::qualifications`]): the staff
//! user is mapped to their employee record by the workforce contract and
//! asked for a qualification of the kind [`TAX_AGENT_KIND`] in the role
//! [`TAX_AGENT_ROLE`] at the firm's legal entity. There is no override.
//! Decision: wl-hr has no Qualification records yet (BL-HRM-01); until it
//! provides the guard the answer is *no*, so only the client's own filing
//! can be recorded. The service gathers the answer into the input, because
//! the machine's guards read no service.
//!
//! For a blocked client the filing is an approval request to the MLRO
//! ([`crate::return_machine::file_policy`]); the filer is told only *On hold
//! — MLRO approval needed*. The approval is for this return only. After the
//! commit ([`crate::return_effects`]) the linked task is completed, the
//! client gets the payment reminder if tax is payable, and an expired client
//! KYC document flags the return for the MLRO ([`crate::documents`]).

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{Outcome, StateMachine, TransitionError};
use wl_kernel::Subject;
use wl_kernel::approvals::{self, submit::REQUEST_KEY};
use wl_kernel::documents::{guard, today};
use wl_kernel::entities::LegalEntity;
use wl_kernel::parties::{PartyStatus, directory};
use wl_kernel::qualifications::qualifications;
use wl_kernel::workforce::workforce;

use crate::documents::RETURN_FILE_GUARDED;
use crate::permissions;
use crate::return_machine::{FILE, ReturnInput};
use crate::tax_return::Return;
use crate::texts::english;

/// The document kind (qualification) of an FTA tax agent registration.
pub const TAX_AGENT_KIND: &str = "fta_tax_agent_registration";
/// The role the registration is asked for.
pub const TAX_AGENT_ROLE: &str = "tax_agent";
/// The subject type of the firm's legal entity, the scope of the
/// registration: it must be linked to the firm's tax agency.
pub const AGENCY_SUBJECT_TYPE: &str = "kernel.legal_entity";

/// What a filing is made of.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Filing {
    /// The FTA acknowledgement (its reference).
    pub acknowledgement: String,
    /// The date on the acknowledgement: the filing date.
    pub filed_on: NaiveDate,
    /// The client filed it through its own EmaraTax user.
    pub by_client: bool,
    /// The EmaraTax user the acknowledgement shows (by the client).
    pub client_user: Option<String>,
    /// The staff member who filed it, when not the user who records it.
    pub filer: Option<i64>,
    /// The tax payable on the return, in fils.
    pub tax_payable: i64,
}

impl Filing {
    /// A filing with the acknowledgement `acknowledgement` dated `filed_on`,
    /// by the user who records it as a tax agent.
    pub fn new(acknowledgement: &str, filed_on: NaiveDate) -> Self {
        Filing {
            acknowledgement: acknowledgement.to_owned(),
            filed_on,
            by_client: false,
            client_user: None,
            filer: None,
            tax_payable: 0,
        }
    }

    /// Filed by the client's own staff through the EmaraTax user `user`.
    pub fn by_client(mut self, user: &str) -> Self {
        self.by_client = true;
        self.client_user = Some(user.to_owned());
        self
    }

    /// Filed by the staff member `user`.
    pub fn filer(mut self, user: i64) -> Self {
        self.filer = Some(user);
        self
    }

    /// With `fils` of tax payable.
    pub fn tax_payable(mut self, fils: i64) -> Self {
        self.tax_payable = fils;
        self
    }
}

/// What a filing came to.
#[derive(Debug, Clone)]
pub struct Filed {
    /// The return after the commit: `filed`, or still `approved` while the
    /// MLRO decides.
    pub ret: Return,
    /// The approval request to the MLRO, when the client is blocked.
    pub approval_request: Option<i64>,
}

/// Whether the staff user `user_id` holds an FTA tax agent registration valid
/// on `on` and linked to the firm's tax agency. `Ok(Ok(number))` with the
/// registration's number, or `Ok(Err(problem))` naming the registration.
///
/// # Errors
///
/// The workforce's, the qualification guard's and the database's errors.
pub async fn tax_agent(
    state: &AppState,
    user_id: i64,
    on: NaiveDate,
) -> Result<std::result::Result<String, String>> {
    let name = match User::find(&state.db, user_id).await? {
        Some(user) => user.name,
        None => format!("#{user_id}"),
    };
    let Some(employee) = workforce(state)
        .employee_of_user(&state.db, user_id)
        .await?
    else {
        return Ok(Err(english(
            "accounting.filing.problem.not_staff",
            &[("name", &name)],
        )));
    };
    let entity = LegalEntity::default_entity(&state.db).await?;
    let agency = Subject::new(AGENCY_SUBJECT_TYPE, entity.id);
    let status = qualifications(state)
        .qualification_valid(
            &state.db,
            employee,
            TAX_AGENT_KIND,
            on,
            Some(TAX_AGENT_ROLE),
            Some(&agency),
        )
        .await?;
    if status.valid {
        return Ok(Ok(status
            .number
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| TAX_AGENT_KIND.to_owned())));
    }
    Ok(Err(match (status.number, status.valid_to) {
        (Some(number), Some(valid_to)) => english(
            "accounting.filing.problem.expired",
            &[
                ("name", &name),
                ("number", &number),
                ("valid_to", &valid_to),
                ("on", &on),
            ],
        ),
        _ => english(
            "accounting.filing.problem.missing",
            &[("name", &name), ("on", &on)],
        ),
    }))
}

/// `by` records return `id` as filed (`ACC.ACL.15`): by the tax agent
/// `filing.filer` (the user, by default) whose registration is valid on the
/// date of the acknowledgement, or by the client through its EmaraTax user.
/// For a blocked client the filing waits for the MLRO's approval and the
/// return stays `approved`.
///
/// # Errors
///
/// `Forbidden` without `returns.file`; a 422 without an acknowledgement or
/// its date (not in the future), for a filer whose registration is expired,
/// missing or not linked to the firm's agency (naming it), for a filing by
/// the client without its EmaraTax user, for a version corrected through the
/// next return, for a return that is not `approved`, and when a document
/// guard blocks the filing.
pub async fn file(state: &AppState, by: &AuthUser, id: i64, filing: Filing) -> Result<Filed> {
    if !by.has_permission(permissions::RETURNS_FILE) {
        return Err(Error::Forbidden);
    }
    let ret = Return::find_or_404(&state.db, id).await?;
    let filer = filing.filer.unwrap_or(by.id);
    let (registration, problem) = if filing.by_client {
        (None, None)
    } else {
        match tax_agent(state, filer, filing.filed_on).await? {
            Ok(number) => (Some(number), None),
            Err(problem) => (None, Some(problem)),
        }
    };
    // The document blocks of `return.file` refuse; the flags are written
    // once it is filed.
    if let Err(refusal) = guard(state)
        .check_as(
            state,
            RETURN_FILE_GUARDED,
            ret.id,
            filing.filed_on,
            Some(by.id),
        )
        .await?
    {
        return Err(refusal.into_error());
    }
    let blocked =
        directory(state).status(&state.db, ret.client()).await? == Some(PartyStatus::Blocked);
    let input = ReturnInput {
        acknowledgement: Some(filing.acknowledgement),
        filed_on: Some(filing.filed_on),
        filed_by_client: filing.by_client,
        client_user: filing.client_user,
        filer_user: (!filing.by_client).then_some(filer),
        tax_agent_registration: registration,
        filer_problem: problem,
        tax_payable: (filing.tax_payable > 0).then_some(filing.tax_payable),
        client_blocked: i64::from(blocked),
        ..ReturnInput::on(today())
    };
    let mut tx = state.db.begin().await?;
    let outcome = ret
        .transition(FILE)
        .by(by)
        .input(&input)
        .run_in(&mut tx)
        .await
        .map_err(TransitionError::into_error)?;
    tx.commit().await?;
    approvals::announce(state, &outcome).await?;
    let approval_request = match &outcome {
        Outcome::Pending(pending) => pending.data.get(REQUEST_KEY).and_then(Value::as_i64),
        Outcome::Done(_) => None,
    };
    outcome.emit(state).await?;
    Ok(Filed {
        ret: Return::find_or_404(&state.db, id).await?,
        approval_request,
    })
}
