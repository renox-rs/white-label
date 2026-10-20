//! The Guarantee (02 Construction §6, §7.1 *Guarantee*; BL-CON-09): an
//! advance guarantee, a performance bond or a retention bond of a project,
//! with its bank documents.
//!
//! ```text
//! draft --issue--> active
//! active --reduce--> active        (the advance guarantee: not below the unrecovered advance)
//! active --extend--> active        (the new expiry is later)
//! active --release--> released
//! active --expire--> expired       (the system, the day after the expiry date)
//! ```
//!
//! The accountant (`guarantees.manage`) registers the guarantee as a `draft`
//! ([`register`]) and runs `issue`, `reduce`, `extend` and `release`; each
//! needs its bank document, which is kept as a document of the guarantee
//! ([`GuaranteeDocument`]) with the transition it came with. Neither the
//! amount nor the expiry changes without one. `expire` is the system's, in
//! [`crate::expiry`], together with `CON.RUL.11` (the extension task 60 days
//! before expiry). `call` and `settle_call` come with the issue that builds
//! them (BL-CON-10); a `called` guarantee does not expire.
//!
//! Decisions where the spec is silent:
//!
//! - The documents are references (the way the termination notice and the
//!   handover certificate are); the shared documents module's own expiry
//!   reminders (`DOC.RUL.01`) are not reachable from this crate, so the
//!   guarantee's own rule `CON.RUL.11` is the one that runs here.
//! - The unrecovered advance is the project's advance paid less the advance
//!   recovered ([`Project::unrecovered_advance`]); the recovery on invoiced
//!   IPAs comes with the advance issues, until then both are set on the
//!   project.
//! - `reduce` must reduce: the new amount is below the current one and above
//!   0.
//! - A guarantee is issued only with an amount above 0, a beneficiary and an
//!   expiry date.

/// The subject type of a guarantee.
pub const SUBJECT_TYPE: &str = "construction.guarantee";

/// The `issue` transition's name.
pub const ISSUE: &str = "issue";
/// The `reduce` transition's name.
pub const REDUCE: &str = "reduce";
/// The `extend` transition's name.
pub const EXTEND: &str = "extend";
/// The `release` transition's name.
pub const RELEASE: &str = "release";
/// The `expire` transition's name.
pub const EXPIRE: &str = "expire";

/// What a guarantee secures.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GuaranteeKind {
    /// An advance payment guarantee.
    #[default]
    Advance,
    /// A performance bond.
    Performance,
    /// A bond that replaces a retention part.
    RetentionBond,
}

/// Where a guarantee stands (§7.1).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GuaranteeStatus {
    /// Requested from the bank.
    #[default]
    Draft,
    /// Issued and held by the beneficiary.
    Active,
    /// The beneficiary demanded payment.
    Called,
    /// Returned, cancelled by the bank, or ended after a call was paid
    /// (terminal).
    Released,
    /// Expiry date passed without extension (terminal).
    Expired,
}

impl GuaranteeStatus {
    /// The states in which the guarantee is not finished.
    pub const OPEN: [GuaranteeStatus; 2] = [GuaranteeStatus::Active, GuaranteeStatus::Called];
}

/// A bank guarantee or a bond.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "construction_guarantees", state_machine, index(project_id))]
pub struct Guarantee {
    /// The primary key.
    pub id: i64,
    /// The project.
    pub project_id: i64,
    /// What it secures.
    pub kind: GuaranteeKind,
    /// The bank.
    pub bank: String,
    /// The amount, in fils.
    pub amount: i64,
    /// Who holds it: the client or the main contractor.
    pub beneficiary: String,
    /// The expiry date.
    pub expires_on: Option<NaiveDate>,
    /// The expiry date the extension task was made for (`CON.RUL.11`).
    pub reminded_for: Option<NaiveDate>,
    /// The lifecycle state; changed only by a transition.
    pub status: GuaranteeStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Guarantee {
    /// A draft guarantee of `kind` for `project_id`.
    pub fn new(project_id: i64, kind: GuaranteeKind, amount: i64) -> Self {
        Guarantee {
            project_id,
            kind,
            amount,
            ..Guarantee::default()
        }
    }

    /// The guarantee as a [`Subject`]: `construction.guarantee:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }

    /// The scope roles are assigned in: the project of the guarantee.
    pub fn scope(&self) -> Scope {
        Scope::new(project::SUBJECT_TYPE, self.project_id)
    }
}

/// A bank document of a guarantee, kept with the transition it came with.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "construction_guarantee_documents", index(guarantee_id))]
pub struct GuaranteeDocument {
    /// The primary key.
    pub id: i64,
    /// The guarantee.
    pub guarantee_id: i64,
    /// `issue`, `reduce`, `extend` or `release`.
    pub transition: String,
    /// The document, a reference.
    pub reference: String,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// What a transition of a guarantee carries to its guard and effect; the
/// facts a guard cannot look up are read by the caller.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuaranteeInput {
    /// The bank document, a reference.
    #[serde(default)]
    pub document: String,
    /// The new amount (`reduce`), in fils.
    #[serde(default)]
    pub amount: Option<i64>,
    /// The new expiry date (`extend`).
    #[serde(default)]
    pub expires_on: Option<NaiveDate>,
    /// The unrecovered advance of the project (`reduce` of an advance
    /// guarantee), in fils.
    #[serde(default)]
    pub unrecovered: Option<i64>,
}

/// The input of a transition of a guarantee.
pub(crate) fn guarantee_input(ctx: &TransitionCtx) -> GuaranteeInput {
    ctx.input_as::<GuaranteeInput>().unwrap_or_default()
}

/// Why the bank document is missing, if it is.
fn document_problem(input: &GuaranteeInput) -> Option<String> {
    input
        .document
        .trim()
        .is_empty()
        .then(|| english("construction.guarantee.document_needed", &[]))
}

/// Why `guarantee` cannot be issued with `input`, if it cannot: the bank
/// document is attached, and the amount, the beneficiary and the expiry date
/// are set.
pub fn issue_problem(guarantee: &Guarantee, input: &GuaranteeInput) -> Option<String> {
    if let Some(problem) = document_problem(input) {
        return Some(problem);
    }
    if guarantee.amount <= 0 {
        return Some(english("construction.guarantee.amount_needed", &[]));
    }
    if guarantee.beneficiary.trim().is_empty() {
        return Some(english("construction.guarantee.beneficiary_needed", &[]));
    }
    if guarantee.expires_on.is_none() {
        return Some(english("construction.guarantee.expiry_needed", &[]));
    }
    None
}

/// Why `guarantee` cannot be reduced with `input`, if it cannot: the bank's
/// amendment (or the beneficiary's consent) is attached, the new amount is
/// above 0 and below the current one, and an advance guarantee does not go
/// below the unrecovered advance.
pub fn reduce_problem(guarantee: &Guarantee, input: &GuaranteeInput) -> Option<String> {
    if let Some(problem) = document_problem(input) {
        return Some(problem);
    }
    let Some(amount) = input.amount.filter(|amount| *amount > 0) else {
        return Some(english("construction.guarantee.amount_needed", &[]));
    };
    if amount >= guarantee.amount {
        return Some(english(
            "construction.guarantee.reduce_to_less",
            &[("amount", &aed(guarantee.amount))],
        ));
    }
    match input.unrecovered {
        Some(unrecovered) if guarantee.kind == GuaranteeKind::Advance && amount < unrecovered => {
            Some(english(
                "construction.guarantee.below_advance",
                &[("unrecovered", &aed(unrecovered))],
            ))
        }
        _ => None,
    }
}

/// Why `guarantee` cannot be extended with `input`, if it cannot: the bank's
/// amendment is attached and the new expiry is later than the current one.
pub fn extend_problem(guarantee: &Guarantee, input: &GuaranteeInput) -> Option<String> {
    if let Some(problem) = document_problem(input) {
        return Some(problem);
    }
    let Some(expires_on) = input.expires_on else {
        return Some(english("construction.guarantee.expiry_needed", &[]));
    };
    match guarantee.expires_on {
        Some(current) if expires_on <= current => Some(english(
            "construction.guarantee.extend_later",
            &[("date", &current)],
        )),
        _ => None,
    }
}

/// Why `guarantee` cannot be released with `input`, if it cannot: the
/// original is returned or the bank's cancellation letter is attached.
pub fn release_problem(input: &GuaranteeInput) -> Option<String> {
    input
        .document
        .trim()
        .is_empty()
        .then(|| english("construction.guarantee.release_document_needed", &[]))
}

/// [`release_problem`] as the check of a transition.
fn release_check(_guarantee: &Guarantee, input: &GuaranteeInput) -> Option<String> {
    release_problem(input)
}

/// Why `guarantee` does not expire on `today`, if it does not: its expiry date
/// has passed.
pub fn expire_problem(guarantee: &Guarantee, today: NaiveDate) -> Option<String> {
    match guarantee.expires_on {
        Some(expires_on) if today > expires_on => None,
        _ => Some(english("construction.guarantee.not_expired", &[])),
    }
}

/// Keeps the bank document of `transition` as a document of guarantee `id`,
/// inside `tx`.
async fn keep_document_in(
    tx: &mut Transaction,
    id: i64,
    transition: &str,
    input: &GuaranteeInput,
) -> Result {
    let mut document = GuaranteeDocument {
        guarantee_id: id,
        transition: transition.to_owned(),
        reference: input.document.trim().to_owned(),
        ..GuaranteeDocument::default()
    };
    document.insert(&mut *tx).await?;
    Ok(())
}

/// A user transition of the guarantee: `guarantees.manage` in the project's
/// scope, with its bank document kept and its own check.
fn managed(
    name: &'static str,
    label: &str,
    from: GuaranteeStatus,
    problem: fn(&Guarantee, &GuaranteeInput) -> Option<String>,
) -> Transition<Guarantee> {
    Transition::new(name)
        .label(label)
        .permission_in(permissions::GUARANTEES_MANAGE, |guarantee: &Guarantee| {
            guarantee.scope()
        })
        .from([from])
        .to(GuaranteeStatus::Active)
        .guard("bank_document", move |guarantee: &Guarantee, ctx| {
            let problem = problem(guarantee, &guarantee_input(ctx));
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(move |guarantee: &Guarantee, ctx, tx| {
            let id = guarantee.id;
            let input = guarantee_input(ctx);
            Box::pin(async move { keep_document_in(&mut *tx, id, name, &input).await })
        })
}

impl StateMachine for Guarantee {
    type State = GuaranteeStatus;
    const COLUMN: renox::db::Col<Guarantee, GuaranteeStatus> = Guarantee::STATUS;

    fn machine() -> Machine<Self> {
        use GuaranteeStatus::{Active, Called, Draft, Expired, Released};
        Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Active)
            .tone("success")
            .state(Called)
            .tone("error")
            .state(Released)
            .tone("success")
            .terminal()
            .state(Expired)
            .tone("neutral")
            .terminal()
            .transition(managed(ISSUE, "Issue", Draft, issue_problem).tone("primary"))
            .transition(managed(REDUCE, "Reduce", Active, reduce_problem).effect(
                |guarantee: &Guarantee, ctx, tx| {
                    let id = guarantee.id;
                    let amount = guarantee_input(ctx).amount;
                    Box::pin(async move {
                        Guarantee::query()
                            .where_(Guarantee::ID.eq(id))
                            .update(&mut *tx, &[("amount", &amount)])
                            .await?;
                        Ok(())
                    })
                },
            ))
            .transition(managed(EXTEND, "Extend", Active, extend_problem).effect(
                |guarantee: &Guarantee, ctx, tx| {
                    let id = guarantee.id;
                    let expires_on = guarantee_input(ctx).expires_on;
                    Box::pin(async move {
                        // The reminders restart from the new expiry.
                        let reminded: Option<NaiveDate> = None;
                        Guarantee::query()
                            .where_(Guarantee::ID.eq(id))
                            .update(
                                &mut *tx,
                                &[("expires_on", &expires_on), ("reminded_for", &reminded)],
                            )
                            .await?;
                        Ok(())
                    })
                },
            ))
            .transition(managed(RELEASE, "Release", Active, release_check).to(Released))
            .transition(
                Transition::new(EXPIRE)
                    .label("Expire")
                    .system()
                    .from([Active])
                    .to(Expired)
                    .guard("expiry_passed", |guarantee: &Guarantee, _ctx| {
                        let problem = expire_problem(guarantee, today());
                        Box::pin(async move { problem.map_or(Ok(()), Err) })
                    }),
            )
    }
}

/// What the accountant enters to register a guarantee.
#[derive(Debug, Clone)]
pub struct NewGuarantee {
    /// The project.
    pub project_id: i64,
    /// What it secures.
    pub kind: GuaranteeKind,
    /// The bank.
    pub bank: String,
    /// Who holds it.
    pub beneficiary: String,
    /// The amount, in fils.
    pub amount: i64,
    /// The expiry date.
    pub expires_on: Option<NaiveDate>,
}

/// `user` registers a guarantee of a project, requested from the bank: a
/// `draft` until the bank issues it.
///
/// # Errors
///
/// `Forbidden` without `guarantees.manage` in the project, `Guard` (422)
/// without a bank or with a negative amount.
pub async fn register(
    state: &AppState,
    user: &AuthUser,
    new: &NewGuarantee,
) -> std::result::Result<Guarantee, TransitionError> {
    let project = Project::find(&state.db, new.project_id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    if !user.has_permission_in(permissions::GUARANTEES_MANAGE, &project.scope()) {
        return Err(TransitionError::Forbidden);
    }
    if new.bank.trim().is_empty() {
        return Err(TransitionError::Guard(english(
            "construction.guarantee.bank_needed",
            &[],
        )));
    }
    if new.amount < 0 {
        return Err(TransitionError::Guard(english(
            "construction.guarantee.amount_needed",
            &[],
        )));
    }
    let mut guarantee = Guarantee {
        bank: new.bank.trim().to_owned(),
        beneficiary: new.beneficiary.trim().to_owned(),
        expires_on: new.expires_on,
        ..Guarantee::new(project.id, new.kind, new.amount)
    };
    guarantee.save(&state.db).await?;
    Ok(guarantee)
}

/// Runs the user transition `name` of guarantee `id` with `input`.
async fn run_user(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    name: &str,
    input: GuaranteeInput,
) -> std::result::Result<Guarantee, TransitionError> {
    let guarantee = Guarantee::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    let _outcome = guarantee
        .transition(name)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    Ok(Guarantee::find_or_404(&state.db, id).await?)
}

/// `user` issues the `draft` guarantee `id` once the bank sent its
/// guarantee.
///
/// # Errors
///
/// `Forbidden` without `guarantees.manage` in the project, `NotFrom` unless
/// it is a `draft`, `Guard` (422) without the bank document, the amount, the
/// beneficiary or the expiry date.
pub async fn issue(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    document: &str,
) -> std::result::Result<Guarantee, TransitionError> {
    let input = GuaranteeInput {
        document: document.to_owned(),
        ..GuaranteeInput::default()
    };
    run_user(state, user, id, ISSUE, input).await
}

/// `user` reduces the `active` guarantee `id` to `amount` fils with the bank's
/// amendment or the beneficiary's consent; an advance guarantee does not go
/// below the unrecovered advance.
///
/// # Errors
///
/// `Forbidden` without `guarantees.manage` in the project, `NotFrom` unless it
/// is `active`, `Guard` (422) without the document, with an amount that does
/// not reduce it, or (an advance guarantee) below the unrecovered advance,
/// which the message shows.
pub async fn reduce(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    amount: i64,
    document: &str,
) -> std::result::Result<Guarantee, TransitionError> {
    let guarantee = Guarantee::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    let unrecovered = if guarantee.kind == GuaranteeKind::Advance {
        let project = Project::find_or_404(&state.db, guarantee.project_id).await?;
        Some(project.unrecovered_advance())
    } else {
        None
    };
    let input = GuaranteeInput {
        document: document.to_owned(),
        amount: Some(amount),
        unrecovered,
        ..GuaranteeInput::default()
    };
    run_user(state, user, id, REDUCE, input).await
}

/// `user` extends the `active` guarantee `id` to `expires_on` with the bank's
/// amendment: the extension reminders start again from the new date.
///
/// # Errors
///
/// `Forbidden` without `guarantees.manage` in the project, `NotFrom` unless it
/// is `active`, `Guard` (422) without the amendment or with a date that is not
/// later than the current expiry.
pub async fn extend(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    expires_on: NaiveDate,
    document: &str,
) -> std::result::Result<Guarantee, TransitionError> {
    let input = GuaranteeInput {
        document: document.to_owned(),
        expires_on: Some(expires_on),
        ..GuaranteeInput::default()
    };
    run_user(state, user, id, EXTEND, input).await
}

/// `user` releases the `active` guarantee `id`: the original is returned or
/// the bank cancelled it.
///
/// # Errors
///
/// `Forbidden` without `guarantees.manage` in the project, `NotFrom` unless it
/// is `active`, `Guard` (422) without the original or the cancellation letter.
pub async fn release(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    document: &str,
) -> std::result::Result<Guarantee, TransitionError> {
    let input = GuaranteeInput {
        document: document.to_owned(),
        ..GuaranteeInput::default()
    };
    run_user(state, user, id, RELEASE, input).await
}

/// The bank documents of guarantee `id`, oldest first.
///
/// # Errors
///
/// A database error.
pub async fn documents_of(db: &Db, id: i64) -> Result<Vec<GuaranteeDocument>> {
    GuaranteeDocument::query()
        .where_(GuaranteeDocument::GUARANTEE_ID.eq(id))
        .order_by(GuaranteeDocument::ID)
        .get(db)
        .await
}

/// The guarantees of `project_id` that are `active`, oldest first.
///
/// # Errors
///
/// A database error.
pub async fn active_of(db: &Db, project_id: i64) -> Result<Vec<Guarantee>> {
    Guarantee::query()
        .where_(Guarantee::PROJECT_ID.eq(project_id))
        .where_(Guarantee::STATUS.eq(GuaranteeStatus::Active))
        .order_by(Guarantee::ID)
        .get(db)
        .await
}

/// The guarantees of `project_id` that are not finished.
///
/// # Errors
///
/// A database error.
pub async fn open_of(db: &Db, project_id: i64) -> Result<Vec<Guarantee>> {
    Guarantee::query()
        .where_(Guarantee::PROJECT_ID.eq(project_id))
        .where_(Guarantee::STATUS.is_in(GuaranteeStatus::OPEN))
        .order_by(Guarantee::ID)
        .get(db)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn guarantee(kind: GuaranteeKind, amount: i64) -> Guarantee {
        Guarantee {
            beneficiary: "Gulf Build Contracting LLC".into(),
            expires_on: Some(day(2027, 9, 30)),
            ..Guarantee::new(1, kind, amount)
        }
    }

    fn input(document: &str) -> GuaranteeInput {
        GuaranteeInput {
            document: document.into(),
            ..GuaranteeInput::default()
        }
    }

    #[test]
    fn a_guarantee_starts_a_draft() {
        let machine = Guarantee::machine();
        assert_eq!(machine.initial_states(), ["draft"]);
        assert_eq!(
            Guarantee::new(1, GuaranteeKind::Performance, 5_000).status,
            GuaranteeStatus::Draft
        );
        assert_eq!(GuaranteeKind::RetentionBond.as_str(), "retention_bond");
    }

    #[test]
    fn every_transition_of_the_lifecycle_is_declared() {
        let machine = Guarantee::machine();
        let table: [(&str, &str, &str, bool); 5] = [
            (ISSUE, "draft", "active", false),
            (REDUCE, "active", "active", false),
            (EXTEND, "active", "active", false),
            (RELEASE, "active", "released", false),
            (EXPIRE, "active", "expired", true),
        ];
        assert_eq!(machine.transitions().len(), table.len());
        for (name, from, to, by_system) in table {
            let transition = machine.find(name).expect(name);
            assert_eq!(transition.from_states(), [from], "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
            assert_eq!(transition.is_system(), by_system, "{name}");
        }
    }

    #[test]
    fn a_guarantee_is_issued_with_its_document_amount_beneficiary_and_expiry() {
        let mut bond = guarantee(GuaranteeKind::Performance, 50_000_000);
        assert_eq!(issue_problem(&bond, &input("BG-1.pdf")), None);
        assert!(issue_problem(&bond, &input(" ")).is_some_and(|text| text.contains("bank")));
        bond.expires_on = None;
        assert!(
            issue_problem(&bond, &input("BG-1.pdf")).is_some_and(|text| text.contains("expiry"))
        );
        bond.expires_on = Some(day(2027, 9, 30));
        bond.beneficiary = String::new();
        assert!(
            issue_problem(&bond, &input("BG-1.pdf"))
                .is_some_and(|text| text.contains("beneficiary"))
        );
        bond.beneficiary = "Client".into();
        bond.amount = 0;
        assert!(
            issue_problem(&bond, &input("BG-1.pdf")).is_some_and(|text| text.contains("amount"))
        );
    }

    #[test]
    fn an_advance_guarantee_does_not_go_below_the_unrecovered_advance() {
        let advance = guarantee(GuaranteeKind::Advance, 15_000_000);
        let reduce_to = |amount| GuaranteeInput {
            document: "AMENDMENT-1.pdf".into(),
            amount: Some(amount),
            unrecovered: Some(9_000_000),
            ..GuaranteeInput::default()
        };
        assert!(
            reduce_problem(&advance, &reduce_to(8_000_000))
                .is_some_and(|text| text.contains("AED 90,000.00"))
        );
        assert_eq!(reduce_problem(&advance, &reduce_to(9_000_000)), None);
        // A performance bond has no unrecovered advance to protect.
        let bond = guarantee(GuaranteeKind::Performance, 15_000_000);
        assert_eq!(reduce_problem(&bond, &reduce_to(1_000_000)), None);
        // It must reduce, with a document.
        assert!(reduce_problem(&advance, &reduce_to(15_000_000)).is_some());
        assert!(reduce_problem(&advance, &reduce_to(0)).is_some());
        let mut undocumented = reduce_to(9_000_000);
        undocumented.document = String::new();
        assert!(reduce_problem(&advance, &undocumented).is_some());
    }

    #[test]
    fn an_extension_is_later_than_the_expiry_and_has_its_amendment() {
        let bond = guarantee(GuaranteeKind::Performance, 50_000_000);
        let extend_to = |date| GuaranteeInput {
            document: "AMENDMENT-2.pdf".into(),
            expires_on: Some(date),
            ..GuaranteeInput::default()
        };
        assert_eq!(extend_problem(&bond, &extend_to(day(2028, 7, 31))), None);
        assert!(
            extend_problem(&bond, &extend_to(day(2027, 9, 30)))
                .is_some_and(|text| text.contains("2027-09-30"))
        );
        assert!(extend_problem(&bond, &extend_to(day(2027, 1, 1))).is_some());
        assert!(extend_problem(&bond, &input("A.pdf")).is_some());
        let mut undocumented = extend_to(day(2028, 7, 31));
        undocumented.document = String::new();
        assert!(extend_problem(&bond, &undocumented).is_some());
    }

    #[test]
    fn a_release_needs_the_original_or_the_cancellation_letter() {
        assert!(release_problem(&input("")).is_some());
        assert_eq!(release_problem(&input("CANCELLATION-LETTER.pdf")), None);
    }

    #[test]
    fn a_guarantee_expires_the_day_after_its_expiry_date() {
        let bond = guarantee(GuaranteeKind::Performance, 50_000_000);
        assert!(expire_problem(&bond, day(2027, 9, 30)).is_some());
        assert_eq!(expire_problem(&bond, day(2027, 10, 1)), None);
        let open = Guarantee::new(1, GuaranteeKind::Advance, 1);
        assert!(expire_problem(&open, day(2030, 1, 1)).is_some());
    }
}
