//! The sales contract (wl-sales §6): what other modules may know about
//! campaigns without depending on the `wl-sales` crate.
//!
//! `wl-sales` emits [`CampaignActivated`] and [`CampaignEnded`]; `wl-notify`
//! and the verticals may listen. Campaigns are referred to by [`CampaignId`]
//! or as a [`Subject`] of type [`CAMPAIGN_SUBJECT_TYPE`].
//!
//! Leads and the acquisition of customers (BL-SAL-20): `wl-sales` emits the
//! lead events ([`LeadCreated`], [`LeadWon`], [`LeadLost`],
//! [`LeadWinReverted`]) and [`CustomerAcquired`] / [`CustomerAcquisitionChanged`].
//! A vertical whose first sale is its own record reports it through
//! [`CampaignAttribution`] and marks a lead won through [`LeadConversion`];
//! without `sales` both fall back to [`NoCampaignAttribution`] and
//! [`NoLeadConversion`], which do nothing.
//!
//! The purpose and the marketing channel are the stored names of the
//! `wl-sales` enums (e.g. `acquisition`, `meta`), so the kernel holds no copy
//! of those lists.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::parties::PartyId;
use crate::subject::Subject;

/// The subject type of a campaign, for [`Subject`] references.
pub const CAMPAIGN_SUBJECT_TYPE: &str = "sales.campaign";

/// The subject type of a campaign cost entry, for [`Subject`] references.
pub const CAMPAIGN_COST_ENTRY_SUBJECT_TYPE: &str = "sales.campaign_cost_entry";

/// The id of a campaign.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct CampaignId(pub i64);

impl CampaignId {
    /// The campaign as a [`Subject`]: `sales.campaign:12`.
    pub fn subject(self) -> Subject {
        Subject::new(CAMPAIGN_SUBJECT_TYPE, self.0)
    }
}

impl fmt::Display for CampaignId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<i64> for CampaignId {
    fn from(id: i64) -> Self {
        CampaignId(id)
    }
}

/// A campaign became active (`activate`, by hand or on its start date).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampaignActivated {
    /// The campaign.
    pub campaign: CampaignId,
    /// `acquisition` or `retention`.
    pub purpose: String,
    /// The marketing channel's stored name.
    pub marketing_channel: String,
    /// The first day.
    pub start_date: NaiveDate,
    /// The last day, if set.
    pub end_date: Option<NaiveDate>,
    /// The budget in fils of AED, if set.
    pub budget: Option<i64>,
}

impl Event for CampaignActivated {}

/// A campaign ended (`end`, by hand or after its end date).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampaignEnded {
    /// The campaign.
    pub campaign: CampaignId,
    /// `acquisition` or `retention`.
    pub purpose: String,
    /// The marketing channel's stored name.
    pub marketing_channel: String,
    /// The first day.
    pub start_date: NaiveDate,
    /// The last day, if set.
    pub end_date: Option<NaiveDate>,
    /// The budget in fils of AED, if set.
    pub budget: Option<i64>,
}

impl Event for CampaignEnded {}

/// The subject type of a lead, for [`Subject`] references.
pub const LEAD_SUBJECT_TYPE: &str = "sales.lead";

/// The id of a lead.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct LeadId(pub i64);

impl LeadId {
    /// The lead as a [`Subject`]: `sales.lead:5`.
    pub fn subject(self) -> Subject {
        Subject::new(LEAD_SUBJECT_TYPE, self.0)
    }
}

impl fmt::Display for LeadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<i64> for LeadId {
    fn from(id: i64) -> Self {
        LeadId(id)
    }
}

/// A lead was created (by staff, a web form or an import).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeadCreated {
    /// The lead.
    pub lead: LeadId,
    /// The party, if the lead has one yet.
    pub party: Option<PartyId>,
    /// The code of the lead source.
    pub source: String,
    /// The campaign it is attributed to, if any.
    pub campaign: Option<CampaignId>,
    /// The staff member who owns it.
    pub owner: Option<i64>,
}

impl Event for LeadCreated {}

/// A lead was won (`win`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeadWon {
    /// The lead.
    pub lead: LeadId,
    /// The party that became a customer.
    pub party: Option<PartyId>,
    /// The code of the lead source.
    pub source: String,
    /// The staff member who owns it.
    pub owner: Option<i64>,
    /// The vertical's conversion record.
    pub converted: Subject,
}

impl Event for LeadWon {}

/// A lead was lost (`lose`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeadLost {
    /// The lead.
    pub lead: LeadId,
    /// The party, if any.
    pub party: Option<PartyId>,
    /// The code of the lead source.
    pub source: String,
    /// The staff member who owns it.
    pub owner: Option<i64>,
    /// The lost reason, e.g. `duplicate` or `spam_or_invalid`.
    pub reason: String,
}

impl Event for LeadLost {}

/// A won lead went back to `qualified` because its conversion record was
/// voided as entered in error (`revert_win`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeadWinReverted {
    /// The lead.
    pub lead: LeadId,
    /// The party, if any.
    pub party: Option<PartyId>,
    /// The voided conversion record.
    pub voided: Subject,
}

impl Event for LeadWinReverted {}

/// A party's first sale was linked: it is a new customer. Sent once per
/// party.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomerAcquired {
    /// The party.
    pub party: PartyId,
    /// The code of the source of the acquisition.
    pub source: String,
    /// The campaign, if any.
    pub campaign: Option<CampaignId>,
    /// The first sale.
    pub first_sale: Subject,
    /// Its value excluding VAT, in fils of AED.
    pub value: i64,
}

impl Event for CustomerAcquired {}

/// A party acquisition changed: its first sale was replaced or cleared, its
/// source or campaign was changed (`SAL.ACL.09`), or it was superseded by a
/// merge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomerAcquisitionChanged {
    /// The party.
    pub party: PartyId,
    /// The source code before.
    pub old_source: Option<String>,
    /// The source code after.
    pub new_source: Option<String>,
    /// The campaign before.
    pub old_campaign: Option<CampaignId>,
    /// The campaign after.
    pub new_campaign: Option<CampaignId>,
    /// The first sale before.
    pub old_first_sale: Option<Subject>,
    /// The first sale after.
    pub new_first_sale: Option<Subject>,
    /// Why: the reason of a staff change, or `first_sale_replaced`,
    /// `first_sale_cleared`, `first_sale_credited`, `lead_win_reverted`,
    /// `merged`.
    pub reason: String,
}

impl Event for CustomerAcquisitionChanged {}

/// The future the sales contract's methods return.
pub type SalesFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// What an attribution is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttributionTarget {
    /// A party's acquisition.
    Party(PartyId),
    /// A lead.
    Lead(LeadId),
}

/// Where a party or lead came from: [`CampaignAttribution::attribute`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attribute {
    /// The party or lead.
    pub target: AttributionTarget,
    /// The code of the lead source.
    pub source: String,
    /// The code of the campaign, if known (the web form link code or the
    /// promo code).
    pub campaign: Option<String>,
    /// What set it: `lead`, `staff`, `web_form`, `promo_code` or `import`.
    pub set_from: String,
}

impl Attribute {
    /// The acquisition of `party` came from `source`.
    pub fn party(party: PartyId, source: &str) -> Self {
        Attribute {
            target: AttributionTarget::Party(party),
            source: source.to_owned(),
            campaign: None,
            set_from: "staff".to_owned(),
        }
    }

    /// The lead `lead` came from `source`.
    pub fn lead(lead: LeadId, source: &str) -> Self {
        Attribute {
            target: AttributionTarget::Lead(lead),
            source: source.to_owned(),
            campaign: None,
            set_from: "staff".to_owned(),
        }
    }

    /// With the campaign code.
    pub fn campaign(mut self, code: &str) -> Self {
        self.campaign = Some(code.to_owned());
        self
    }

    /// Set from `from`.
    pub fn set_from(mut self, from: &str) -> Self {
        self.set_from = from.to_owned();
        self
    }
}

/// A sale of a party, reported as a candidate for its first sale:
/// [`CampaignAttribution::record_first_sale`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirstSale {
    /// The party that bought.
    pub party: PartyId,
    /// The sale: a confirmed sales order, a completed counter sale, an issued
    /// invoice, or the vertical's conversion record.
    pub subject: Subject,
    /// Its value excluding VAT, in fils of AED.
    pub value: i64,
    /// Its document date.
    pub date: NaiveDate,
    /// The branch of the sale, if any.
    pub branch: Option<Subject>,
    /// The legal entity of the sale, if known.
    pub legal_entity_id: Option<i64>,
}

impl FirstSale {
    /// A sale of `party`.
    pub fn new(party: PartyId, subject: Subject, value: i64, date: NaiveDate) -> Self {
        FirstSale {
            party,
            subject,
            value,
            date,
            branch: None,
            legal_entity_id: None,
        }
    }

    /// At `branch`.
    pub fn branch(mut self, branch: Subject) -> Self {
        self.branch = Some(branch);
        self
    }

    /// Of the legal entity `id`.
    pub fn legal_entity(mut self, id: i64) -> Self {
        self.legal_entity_id = Some(id);
        self
    }
}

/// What verticals ask of the campaign attribution (wl-sales §6). `wl-sales`
/// provides it; without that module, [`campaign_attribution`] gives
/// [`NoCampaignAttribution`].
pub trait CampaignAttribution: Send + Sync + 'static {
    /// Records where a party or lead came from. Never overwrites: a party
    /// that already has an acquisition keeps it. Returns whether it was
    /// recorded. The default records nothing.
    fn attribute<'a>(&'a self, _state: &'a AppState, _request: Attribute) -> SalesFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }

    /// Reports a sale of a party. The party's first sale is linked when it
    /// has none yet; a later call for the same party keeps the first and
    /// only remembers the sale, in case the first is voided, cancelled or
    /// credited in full. Returns whether `sale` is the party's first sale
    /// (also when it was already). The default does nothing.
    fn record_first_sale<'a>(
        &'a self,
        _state: &'a AppState,
        _sale: FirstSale,
    ) -> SalesFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }

    /// Tells that a reported sale was cancelled or voided; the nightly run
    /// links the party's next sale as the first. The default does nothing.
    fn sale_gone<'a>(&'a self, _state: &'a AppState, _sale: Subject) -> SalesFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

/// The campaign attribution when the `sales` module is off.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoCampaignAttribution;

impl CampaignAttribution for NoCampaignAttribution {}

/// Makes `attribution` the app's [`CampaignAttribution`]; `wl-sales` calls
/// it in `register`.
pub fn provide_campaign_attribution(app: &mut Registry, attribution: impl CampaignAttribution) {
    let attribution: Arc<dyn CampaignAttribution> = Arc::new(attribution);
    app.provide(attribution);
}

/// The app's [`CampaignAttribution`], or [`NoCampaignAttribution`] when none
/// is provided.
pub fn campaign_attribution(state: &AppState) -> Arc<dyn CampaignAttribution> {
    match state.provided::<Arc<dyn CampaignAttribution>>() {
        Some(attribution) => Arc::clone(&attribution),
        None => Arc::new(NoCampaignAttribution),
    }
}

/// What verticals ask to mark a lead won from their conversion event
/// (wl-sales §4.1). `wl-sales` provides it.
pub trait LeadConversion: Send + Sync + 'static {
    /// Runs `win` on `lead` with the vertical's conversion record. Returns
    /// whether the lead was won (it is `contacted` or `qualified`). The
    /// default wins nothing.
    fn win<'a>(
        &'a self,
        _state: &'a AppState,
        _lead: LeadId,
        _converted: Subject,
    ) -> SalesFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }

    /// Runs `win` on the newest open (`contacted` or `qualified`) lead of
    /// `party`. Returns whether a lead was won. The default wins nothing.
    fn win_for_party<'a>(
        &'a self,
        _state: &'a AppState,
        _party: PartyId,
        _converted: Subject,
    ) -> SalesFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }
}

/// The lead conversion when the `sales` module is off.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoLeadConversion;

impl LeadConversion for NoLeadConversion {}

/// Makes `conversion` the app's [`LeadConversion`]; `wl-sales` calls it in
/// `register`.
pub fn provide_lead_conversion(app: &mut Registry, conversion: impl LeadConversion) {
    let conversion: Arc<dyn LeadConversion> = Arc::new(conversion);
    app.provide(conversion);
}

/// The app's [`LeadConversion`], or [`NoLeadConversion`] when none is
/// provided.
pub fn lead_conversion(state: &AppState) -> Arc<dyn LeadConversion> {
    match state.provided::<Arc<dyn LeadConversion>>() {
        Some(conversion) => Arc::clone(&conversion),
        None => Arc::new(NoLeadConversion),
    }
}

/// The subject type of a prepaid package, for [`Subject`] references.
pub const PACKAGE_SUBJECT_TYPE: &str = "sales.package";

/// The id of a prepaid package.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct PackageId(pub i64);

impl PackageId {
    /// The package as a [`Subject`]: `sales.package:9`.
    pub fn subject(self) -> Subject {
        Subject::new(PACKAGE_SUBJECT_TYPE, self.0)
    }
}

impl fmt::Display for PackageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<i64> for PackageId {
    fn from(id: i64) -> Self {
        PackageId(id)
    }
}

/// A package became `active`: paid in full, or its first instalment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageActivated {
    /// The package.
    pub package: PackageId,
    /// The holder.
    pub party: PartyId,
    /// The credits it holds.
    pub credits: i64,
    /// The last day of validity.
    pub valid_to: Option<NaiveDate>,
}

impl Event for PackageActivated {}

/// Credits of a package were used (`consume`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageConsumed {
    /// The package.
    pub package: PackageId,
    /// The holder.
    pub party: PartyId,
    /// The credits used by this movement.
    pub quantity: i64,
    /// The credits left after it.
    pub remaining: i64,
    /// What consumed them (a booking, an attendance, a job card).
    pub source: Subject,
}

impl Event for PackageConsumed {}

/// A package's end date was moved (`extend`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageExtended {
    /// The package.
    pub package: PackageId,
    /// The holder.
    pub party: PartyId,
    /// The credits left.
    pub remaining: i64,
    /// The new last day of validity.
    pub valid_to: NaiveDate,
}

impl Event for PackageExtended {}

/// A package's validity ended with credits left (`expire`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageExpired {
    /// The package.
    pub package: PackageId,
    /// The holder.
    pub party: PartyId,
    /// The credits left unused.
    pub remaining: i64,
}

impl Event for PackageExpired {}

/// A request to use credits of a party's package (`PackageLedger::consume`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ConsumePackage {
    /// The holder.
    pub party: PartyId,
    /// The catalogue item the package was sold for.
    pub item: String,
    /// The credits to use.
    pub quantity: i64,
    /// What consumes them (a booking, an attendance, a job card). A source
    /// that consumed already changes nothing.
    pub source: Subject,
}

impl ConsumePackage {
    /// `quantity` credits of `party`'s package for `item`, used by `source`.
    pub fn new(party: PartyId, item: &str, quantity: i64, source: Subject) -> Self {
        ConsumePackage {
            party,
            item: item.to_owned(),
            quantity,
            source,
        }
    }
}

/// How a [`PackageLedger::consume`] ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PackageConsumption {
    /// The credits were used; `remaining` are left.
    Consumed {
        /// The package.
        package: PackageId,
        /// The credits left.
        remaining: i64,
    },
    /// The source consumed before; nothing changed.
    Already {
        /// The package.
        package: PackageId,
    },
    /// The package has a reopened, unpaid balance and the module is set to
    /// suspend consumption then: nothing was consumed. The source records
    /// "credit not consumed: unpaid" and its vertical decides what follows.
    Unpaid {
        /// The package.
        package: PackageId,
    },
    /// The party has no active package with enough credits for the item.
    NoCredit,
}

/// What verticals and `wl-scheduling` ask of the prepaid packages
/// (wl-sales §6). `wl-sales` provides it.
pub trait PackageLedger: Send + Sync + 'static {
    /// Uses `request.quantity` credits of the party's active package for the
    /// item that expires first. The default has no package.
    fn consume<'a>(
        &'a self,
        _state: &'a AppState,
        _request: ConsumePackage,
    ) -> SalesFuture<'a, PackageConsumption> {
        Box::pin(async { Ok(PackageConsumption::NoCredit) })
    }

    /// Gives back the credits `source` consumed (the booking was cancelled).
    /// Returns whether anything was given back. The default does nothing.
    fn reverse<'a>(&'a self, _state: &'a AppState, _source: Subject) -> SalesFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }

    /// The credits `party` can still use for `item` in active packages. The
    /// default is none.
    fn balance<'a>(&'a self, _db: &'a Db, _party: PartyId, _item: &'a str) -> SalesFuture<'a, i64> {
        Box::pin(async { Ok(0) })
    }
}

/// The package ledger when the `sales` module is off.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoPackageLedger;

impl PackageLedger for NoPackageLedger {}

/// Makes `ledger` the app's [`PackageLedger`]; `wl-sales` calls it in
/// `register`.
pub fn provide_package_ledger(app: &mut Registry, ledger: impl PackageLedger) {
    let ledger: Arc<dyn PackageLedger> = Arc::new(ledger);
    app.provide(ledger);
}

/// The app's [`PackageLedger`], or [`NoPackageLedger`] when none is provided.
pub fn package_ledger(state: &AppState) -> Arc<dyn PackageLedger> {
    match state.provided::<Arc<dyn PackageLedger>>() {
        Some(ledger) => Arc::clone(&ledger),
        None => Arc::new(NoPackageLedger),
    }
}

/// The self-pay price list of the service catalogue (`SAL.CAT.01`): what a
/// vertical quotes a party who pays a service itself, such as the estimate on
/// the self-pay consent of 05 (`CLN.ACL.15`). `wl-sales` provides it once the
/// catalogue is built; until then there is no price for any service.
pub trait SelfPayPrices: Send + Sync + 'static {
    /// The self-pay price in fils of the service `code` on `on`, or `None`
    /// when the list has none for that day. The default has no price.
    fn price_on<'a>(
        &'a self,
        _db: &'a Db,
        _code: &'a str,
        _on: NaiveDate,
    ) -> SalesFuture<'a, Option<i64>> {
        Box::pin(async { Ok(None) })
    }
}

/// The price list when nothing provides one: no service has a price.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoSelfPayPrices;

impl SelfPayPrices for NoSelfPayPrices {}

/// Makes `prices` the app's [`SelfPayPrices`]; `wl-sales` calls it in
/// `register` once it has the catalogue.
pub fn provide_self_pay_prices(app: &mut Registry, prices: impl SelfPayPrices) {
    let prices: Arc<dyn SelfPayPrices> = Arc::new(prices);
    app.provide(prices);
}

/// The app's [`SelfPayPrices`], or [`NoSelfPayPrices`] when none is provided.
pub fn self_pay_prices(state: &AppState) -> Arc<dyn SelfPayPrices> {
    match state.provided::<Arc<dyn SelfPayPrices>>() {
        Some(prices) => Arc::clone(&prices),
        None => Arc::new(NoSelfPayPrices),
    }
}

// ---------------------------------------------------------------------------
// Counter sales, sales orders, sales returns and loyalty points (wl-sales
// §4.2, §4.3, §4.5, §4.7, §6; base for BL-SAL-22).
// ---------------------------------------------------------------------------

/// The subject type of a counter sale, for [`Subject`] references.
pub const COUNTER_SALE_SUBJECT_TYPE: &str = "sales.counter_sale";
/// The subject type of a sales order, for [`Subject`] references.
pub const SALES_ORDER_SUBJECT_TYPE: &str = "sales.sales_order";
/// The subject type of a sales return, for [`Subject`] references.
pub const SALES_RETURN_SUBJECT_TYPE: &str = "sales.sales_return";
/// The subject type of a loyalty account, for [`Subject`] references.
pub const LOYALTY_ACCOUNT_SUBJECT_TYPE: &str = "sales.loyalty_account";

/// Defines a typed id of a sales record: a plain number in JSON, a
/// [`Subject`] on demand.
macro_rules! sales_id {
    ($(#[$meta:meta])* $name:ident, $subject_type:expr) => {
        $(#[$meta])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize,
            Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub i64);

        impl $name {
            /// The record as a [`Subject`].
            pub fn subject(self) -> Subject {
                Subject::new($subject_type, self.0)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl From<i64> for $name {
            fn from(id: i64) -> Self {
                $name(id)
            }
        }
    };
}

sales_id!(
    /// The id of a counter sale.
    CounterSaleId,
    COUNTER_SALE_SUBJECT_TYPE
);
sales_id!(
    /// The id of a sales order.
    SalesOrderId,
    SALES_ORDER_SUBJECT_TYPE
);
sales_id!(
    /// The id of a sales return.
    SalesReturnId,
    SALES_RETURN_SUBJECT_TYPE
);
sales_id!(
    /// The id of a loyalty account.
    LoyaltyAccountId,
    LOYALTY_ACCOUNT_SUBJECT_TYPE
);

/// A counter sale was paid (or charged to the customer account) and
/// completed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CounterSaleCompleted {
    /// The sale.
    pub sale: CounterSaleId,
    /// The customer, when the sale names one.
    pub party: Option<PartyId>,
    /// The till session it was rung up on, when there is one.
    pub till_session: Option<i64>,
    /// The total with VAT, in fils.
    pub gross_total: i64,
    /// What was taken per payment method (`cash`, `card`, `account`…), in
    /// fils.
    pub by_method: Vec<(String, i64)>,
}

impl Event for CounterSaleCompleted {}

/// A sales order reached `confirmed` for the first time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SalesOrderConfirmed {
    /// The order.
    pub order: SalesOrderId,
    /// The customer.
    pub party: PartyId,
    /// The channel it came through.
    pub channel: String,
}

impl Event for SalesOrderConfirmed {}

/// A quantity of one order line, in thousandths (1000 = 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FulfilledLine {
    /// The sale line (its id in `wl-sales`).
    pub line: i64,
    /// The quantity, in thousandths.
    pub quantity_milli: i64,
}

/// Quantities of an order were delivered or collected (`fulfil`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SalesOrderFulfilled {
    /// The order.
    pub order: SalesOrderId,
    /// The customer.
    pub party: PartyId,
    /// The quantities fulfilled by this hand-over.
    pub lines: Vec<FulfilledLine>,
    /// The hand-over: a dispatch note, a delivery, a collection.
    pub source: Subject,
    /// Whether nothing is open any more: the order is `fulfilled`.
    pub complete: bool,
}

impl Event for SalesOrderFulfilled {}

/// Quantities that were fulfilled no longer are (`reverse_fulfilment`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SalesOrderFulfilmentReversed {
    /// The order.
    pub order: SalesOrderId,
    /// The customer.
    pub party: PartyId,
    /// The quantities no longer fulfilled.
    pub lines: Vec<FulfilledLine>,
    /// The hand-over that was reversed or lost.
    pub source: Subject,
    /// `reversed` or `lost`.
    pub cause: String,
}

impl Event for SalesOrderFulfilmentReversed {}

/// A sales order was cancelled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SalesOrderCancelled {
    /// The order.
    pub order: SalesOrderId,
    /// The customer.
    pub party: PartyId,
    /// The reason.
    pub reason: String,
}

impl Event for SalesOrderCancelled {}

/// A sales order was rejected by the seller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SalesOrderRejected {
    /// The order.
    pub order: SalesOrderId,
    /// The customer.
    pub party: PartyId,
    /// The reason.
    pub reason: String,
}

impl Event for SalesOrderRejected {}

/// A sales order is closed: everything fulfilled is invoiced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SalesOrderClosed {
    /// The order.
    pub order: SalesOrderId,
    /// The customer.
    pub party: PartyId,
}

impl Event for SalesOrderClosed {}

/// A sales return was resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReturnResolved {
    /// The return.
    pub sales_return: SalesReturnId,
    /// `goods` or `no_goods`.
    pub kind: String,
    /// The sale it returns: a counter sale or a sales order.
    pub sale: Subject,
    /// `credit_note`, `refund`, `replacement` or `refunded_by_channel`.
    pub resolution: String,
    /// The value returned with VAT, in fils.
    pub value: i64,
    /// The credit note issued for it, when there is one.
    pub credit_note: Option<i64>,
}

impl Event for ReturnResolved {}

/// A sales return resolved *refunded by the channel* was taken back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReturnReversed {
    /// The return.
    pub sales_return: SalesReturnId,
    /// The sale it returned.
    pub sale: Subject,
    /// The value, with VAT, in fils.
    pub value: i64,
    /// Why.
    pub reason: String,
}

impl Event for ReturnReversed {}

/// Loyalty points (or visits) were earned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointsEarned {
    /// The loyalty account.
    pub account: LoyaltyAccountId,
    /// The member.
    pub party: PartyId,
    /// The points (or visits).
    pub points: i64,
    /// Their value in fils (points × the point value).
    pub value: i64,
    /// The sale that earned them.
    pub source: Subject,
}

impl Event for PointsEarned {}

/// Earned loyalty points (or a visit) were taken back, in proportion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointsReversed {
    /// The loyalty account.
    pub account: LoyaltyAccountId,
    /// The member.
    pub party: PartyId,
    /// The points (or visits) taken back, positive.
    pub points: i64,
    /// Their value in fils.
    pub value: i64,
    /// The record that caused it: a sales return, a credit note, a payment
    /// reversal, a write-off, a reopened invoice, a reversed fulfilment.
    pub source: Subject,
}

impl Event for PointsReversed {}

/// Loyalty points were redeemed on a sale (`redeem`, BL-SAL-23).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointsRedeemed {
    /// The loyalty account.
    pub account: LoyaltyAccountId,
    /// The member.
    pub party: PartyId,
    /// The points spent, positive.
    pub points: i64,
    /// Their value in fils (points × the point value).
    pub value: i64,
    /// The sale they paid part of.
    pub source: Subject,
}

impl Event for PointsRedeemed {}

/// Redeemed points were given back (`reverse_redeem`, BL-SAL-23): a sale
/// was returned, an order was cancelled or rejected, or a line was cancelled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointsGivenBack {
    /// The loyalty account.
    pub account: LoyaltyAccountId,
    /// The member.
    pub party: PartyId,
    /// The points given back, positive.
    pub points: i64,
    /// Their value in fils.
    pub value: i64,
    /// The record that caused it: a sales return or an order.
    pub source: Subject,
}

impl Event for PointsGivenBack {}

/// The rest of an earn lot expired after its date (`expire`, `SAL.RUL.08`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointsExpired {
    /// The loyalty account.
    pub account: LoyaltyAccountId,
    /// The member.
    pub party: PartyId,
    /// The points that expired, positive.
    pub points: i64,
    /// Their value in fils.
    pub value: i64,
    /// The last day the lot could be redeemed.
    pub lot_expired_on: NaiveDate,
}

impl Event for PointsExpired {}

/// A balance was corrected by hand (`adjust`, `SAL.ACL.07`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointsAdjusted {
    /// The loyalty account.
    pub account: LoyaltyAccountId,
    /// The member.
    pub party: PartyId,
    /// The points added (positive) or taken off (negative).
    pub points: i64,
    /// Their value in fils, signed like the points.
    pub value: i64,
    /// The reason given.
    pub reason: String,
}

impl Event for PointsAdjusted {}

/// A party merge moved points from one loyalty account to another (`transfer`
/// movements, wl-sales §4.7). The lots keep their expiry dates; nothing is
/// forfeited.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointsTransferred {
    /// The account the points left (closed *merged into*).
    pub from: LoyaltyAccountId,
    /// The account they went to.
    pub to: LoyaltyAccountId,
    /// The surviving member.
    pub party: PartyId,
    /// The points moved (negative when the merged account owed points).
    pub points: i64,
    /// Their value in fils, signed like the points.
    pub value: i64,
}

impl Event for PointsTransferred {}

/// A line of a sales order was cancelled (`cancel_line`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SalesOrderLineCancelled {
    /// The order.
    pub order: SalesOrderId,
    /// The customer.
    pub party: PartyId,
    /// The sale line (its id in `wl-sales`).
    pub line: i64,
    /// The quantity cancelled, in thousandths.
    pub quantity_milli: i64,
    /// The reason.
    pub reason: String,
}

impl Event for SalesOrderLineCancelled {}

/// A loyalty redemption to price (`DiscountEngine::quote_redemption`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuoteRedemption {
    /// The member's loyalty account.
    pub account: LoyaltyAccountId,
    /// The counter sale or sales order the points would pay part of.
    pub sale: Subject,
    /// The points the member wants to spend.
    pub points: i64,
}

/// What a loyalty redemption comes to: the discount line it adds to the sale.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedemptionQuote {
    /// The points the line is worth.
    pub points: i64,
    /// What the points are worth (points × the point value), in fils, with
    /// VAT: the amount the sale's total goes down by.
    pub value: i64,
    /// The negative price of the line before VAT, in fils.
    pub line_net: i64,
    /// The VAT of the line (negative), in fils.
    pub line_vat: i64,
    /// The most points the programme's maximum share of this sale allows.
    pub maximum_points: i64,
}

/// What the sales and POS flows ask of the discount rules (wl-sales §6
/// `DiscountEngine`). `wl-sales` provides it; this base answers the loyalty
/// redemption line (`SAL.DSC.01` rules and promo codes are BL-FNB / BL-EDU
/// work and are not in the trait yet).
pub trait DiscountEngine: Send + Sync + 'static {
    /// Prices the redemption of `request.points` on the sale: the discount
    /// line after the other discounts, within the minimum points, the maximum
    /// share of the sale and the member's available points. Fails (422) with
    /// the reason when it cannot be redeemed. The default has no engine.
    fn quote_redemption<'a>(
        &'a self,
        _state: &'a AppState,
        _request: QuoteRedemption,
    ) -> SalesFuture<'a, RedemptionQuote> {
        Box::pin(async {
            Err(Error::Status(
                StatusCode::UNPROCESSABLE_ENTITY,
                "Loyalty redemption needs the sales module.".to_owned(),
            ))
        })
    }
}

/// The discount engine when the `sales` module is off.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoDiscountEngine;

impl DiscountEngine for NoDiscountEngine {}

/// Makes `engine` the app's [`DiscountEngine`]; `wl-sales` calls it in
/// `register`.
pub fn provide_discount_engine(app: &mut Registry, engine: impl DiscountEngine) {
    let engine: Arc<dyn DiscountEngine> = Arc::new(engine);
    app.provide(engine);
}

/// The app's [`DiscountEngine`], or [`NoDiscountEngine`] when none is
/// provided.
pub fn discount_engine(state: &AppState) -> Arc<dyn DiscountEngine> {
    match state.provided::<Arc<dyn DiscountEngine>>() {
        Some(engine) => Arc::clone(&engine),
        None => Arc::new(NoDiscountEngine),
    }
}

/// A hand-over of an order's goods (`OrderFulfilment::fulfil`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FulfilOrder {
    /// The order.
    pub order: SalesOrderId,
    /// The accepted quantities.
    pub lines: Vec<FulfilledLine>,
    /// The hand-over (a dispatch note, a delivery, a collection); a source
    /// that fulfilled already changes nothing.
    pub source: Subject,
}

/// A hand-over that no longer holds (`OrderFulfilment::reverse`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReverseFulfilment {
    /// The order.
    pub order: SalesOrderId,
    /// The quantities no longer fulfilled.
    pub lines: Vec<FulfilledLine>,
    /// The hand-over that was reversed or lost.
    pub source: Subject,
    /// `reversed` or `lost`.
    pub cause: String,
}

/// How an [`OrderFulfilment`] call ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FulfilmentOutcome {
    /// The quantities were fulfilled; `complete` when nothing is open.
    Fulfilled {
        /// The order.
        order: SalesOrderId,
        /// Whether the order is `fulfilled`.
        complete: bool,
    },
    /// The source fulfilled before; nothing changed.
    Already {
        /// The order.
        order: SalesOrderId,
    },
    /// The fulfilment was reversed.
    Reversed {
        /// The order.
        order: SalesOrderId,
    },
    /// There is no such order, or the module is off.
    NoOrder,
}

/// What `wl-inventory`, `wl-delivery` and the verticals' own fulfilment ask
/// of a sales order (wl-sales §6 `OrderFulfilment`). `wl-sales` provides it.
pub trait OrderFulfilment: Send + Sync + 'static {
    /// Fulfils the accepted quantities of an order; idempotent per source.
    /// The default has no order.
    fn fulfil<'a>(
        &'a self,
        _state: &'a AppState,
        _request: FulfilOrder,
    ) -> SalesFuture<'a, FulfilmentOutcome> {
        Box::pin(async { Ok(FulfilmentOutcome::NoOrder) })
    }

    /// Takes quantities of an earlier hand-over off the order again. The
    /// default has no order.
    fn reverse<'a>(
        &'a self,
        _state: &'a AppState,
        _request: ReverseFulfilment,
    ) -> SalesFuture<'a, FulfilmentOutcome> {
        Box::pin(async { Ok(FulfilmentOutcome::NoOrder) })
    }
}

/// The order fulfilment when the `sales` module is off.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoOrderFulfilment;

impl OrderFulfilment for NoOrderFulfilment {}

/// Makes `fulfilment` the app's [`OrderFulfilment`]; `wl-sales` calls it in
/// `register`.
pub fn provide_order_fulfilment(app: &mut Registry, fulfilment: impl OrderFulfilment) {
    let fulfilment: Arc<dyn OrderFulfilment> = Arc::new(fulfilment);
    app.provide(fulfilment);
}

/// The app's [`OrderFulfilment`], or [`NoOrderFulfilment`] when none is
/// provided.
pub fn order_fulfilment(state: &AppState) -> Arc<dyn OrderFulfilment> {
    match state.provided::<Arc<dyn OrderFulfilment>>() {
        Some(fulfilment) => Arc::clone(&fulfilment),
        None => Arc::new(NoOrderFulfilment),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_campaign_id_is_a_subject_and_a_plain_number() {
        let id = CampaignId::from(12);
        assert_eq!(id.to_string(), "12");
        assert_eq!(id.subject(), Subject::new("sales.campaign", 12));
        assert_eq!(renox::serde_json::to_string(&id).unwrap(), "12");
        assert_eq!(CampaignId::default(), CampaignId(0));
    }

    #[test]
    fn a_package_id_is_a_subject_and_a_plain_number() {
        let id = PackageId::from(9);
        assert_eq!(id.subject(), Subject::new("sales.package", 9));
        assert_eq!(renox::serde_json::to_string(&id).unwrap(), "9");
        assert_eq!(PACKAGE_SUBJECT_TYPE, "sales.package");
    }

    #[test]
    fn the_subject_types_are_stable() {
        assert_eq!(CAMPAIGN_SUBJECT_TYPE, "sales.campaign");
        assert_eq!(
            CAMPAIGN_COST_ENTRY_SUBJECT_TYPE,
            "sales.campaign_cost_entry"
        );
    }

    #[test]
    fn the_campaign_events_round_trip_through_json() {
        let day = |m, d| NaiveDate::from_ymd_opt(2026, m, d).unwrap();
        let activated = CampaignActivated {
            campaign: CampaignId(1),
            purpose: "acquisition".into(),
            marketing_channel: "meta".into(),
            start_date: day(3, 1),
            end_date: Some(day(3, 31)),
            budget: Some(500_000),
        };
        let json = renox::serde_json::to_string(&activated).unwrap();
        assert_eq!(
            renox::serde_json::from_str::<CampaignActivated>(&json).unwrap(),
            activated
        );
        let ended = CampaignEnded {
            campaign: CampaignId(1),
            purpose: "acquisition".into(),
            marketing_channel: "meta".into(),
            start_date: day(3, 1),
            end_date: None,
            budget: None,
        };
        let json = renox::serde_json::to_string(&ended).unwrap();
        assert_eq!(
            renox::serde_json::from_str::<CampaignEnded>(&json).unwrap(),
            ended
        );
    }

    #[test]
    fn a_lead_id_is_a_subject_and_the_attribution_builders_set_their_parts() {
        assert_eq!(LeadId(5).subject(), Subject::new("sales.lead", 5));
        let request = Attribute::party(PartyId(3), "paid_ads")
            .campaign("SPRING26")
            .set_from("web_form");
        assert_eq!(request.target, AttributionTarget::Party(PartyId(3)));
        assert_eq!(request.campaign.as_deref(), Some("SPRING26"));
        assert_eq!(request.set_from, "web_form");
        assert_eq!(Attribute::lead(LeadId(1), "walk_in").set_from, "staff");
        let day = NaiveDate::from_ymd_opt(2026, 4, 2).unwrap();
        let sale = FirstSale::new(
            PartyId(3),
            Subject::new("invoicing.invoice", 9),
            200_000,
            day,
        )
        .legal_entity(1);
        assert_eq!(sale.legal_entity_id, Some(1));
        assert!(sale.branch.is_none());
    }

    #[test]
    fn the_acquisition_events_round_trip_through_json() {
        let acquired = CustomerAcquired {
            party: PartyId(3),
            source: "paid_ads".into(),
            campaign: Some(CampaignId(1)),
            first_sale: Subject::new("invoicing.invoice", 9),
            value: 200_000,
        };
        let json = renox::serde_json::to_string(&acquired).unwrap();
        assert_eq!(
            renox::serde_json::from_str::<CustomerAcquired>(&json).unwrap(),
            acquired
        );
        let won = LeadWon {
            lead: LeadId(2),
            party: Some(PartyId(3)),
            source: "referral".into(),
            owner: None,
            converted: Subject::new("test.conversion", 1),
        };
        let json = renox::serde_json::to_value(&won).unwrap();
        assert_eq!(json["lead"], 2);
        assert!(json["owner"].is_null());
    }

    #[test]
    fn the_sale_ids_are_subjects_and_plain_numbers() {
        assert_eq!(
            CounterSaleId(4).subject(),
            Subject::new("sales.counter_sale", 4)
        );
        assert_eq!(
            SalesOrderId(5).subject(),
            Subject::new("sales.sales_order", 5)
        );
        assert_eq!(
            SalesReturnId(6).subject(),
            Subject::new("sales.sales_return", 6)
        );
        assert_eq!(
            LoyaltyAccountId(7).subject(),
            Subject::new("sales.loyalty_account", 7)
        );
        assert_eq!(renox::serde_json::to_string(&SalesOrderId(5)).unwrap(), "5");
    }

    #[test]
    fn the_default_order_fulfilment_has_no_order() {
        fn _object_safe(_: &dyn OrderFulfilment) {}
        _object_safe(&NoOrderFulfilment);
    }
}
