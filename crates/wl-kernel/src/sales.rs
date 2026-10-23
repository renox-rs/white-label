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

    /// Whether campaign `campaign` exists. A vertical that keeps its own
    /// rows by campaign id (05's advertising permit) asks before it stores
    /// one. The default says yes: no sales module, nothing to check.
    fn campaign_exists<'a>(
        &'a self,
        _state: &'a AppState,
        _campaign: CampaignId,
    ) -> SalesFuture<'a, bool> {
        Box::pin(async { Ok(true) })
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

    /// Moves the last day of the `active` package `package` on by `days` as
    /// the system, with `why` as the reason: the days the agreement it was
    /// sold for stood frozen are given back (10 `freeze`). It does not count
    /// against the extension limit of a user's `extend`. Answers the new last
    /// day, or `None` when nothing moved. The default moves nothing.
    fn extend_days<'a>(
        &'a self,
        _state: &'a AppState,
        _package: PackageId,
        _days: i64,
        _why: &'a str,
    ) -> SalesFuture<'a, Option<NaiveDate>> {
        Box::pin(async { Ok(None) })
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

/// A package of sessions to sell (`PackageSales::sell`): what a vertical asks
/// when an enrolment is for a package (10 `EDU.ENR.01`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SellPackage {
    /// Who uses the credits.
    pub holder: PartyId,
    /// Who pays, when not the holder.
    pub payer: Option<PartyId>,
    /// The code of the catalogue item.
    pub item: String,
    /// The name of the item as sold.
    pub item_name: String,
    /// The credits (sessions) it holds.
    pub credits: i64,
    /// The price paid, in fils.
    pub price: i64,
    /// The months it is valid after activation; `None` for the module's setting.
    pub validity_months: Option<i64>,
    /// The invoice it is paid with (`invoicing.invoice`), when drafted.
    pub invoice: Option<i64>,
    /// The staff user who sells it.
    pub sold_by: Option<i64>,
}

impl SellPackage {
    /// `credits` sessions of `item` for `holder`, priced `price` fils.
    pub fn new(holder: PartyId, item: &str, credits: i64, price: i64) -> Self {
        SellPackage {
            holder,
            payer: None,
            item: item.to_owned(),
            item_name: item.to_owned(),
            credits,
            price,
            validity_months: None,
            invoice: None,
            sold_by: None,
        }
    }

    /// Paid by `payer`.
    pub fn payer(mut self, payer: PartyId) -> Self {
        self.payer = Some(payer);
        self
    }

    /// With the item's name as sold.
    pub fn item_name(mut self, name: &str) -> Self {
        self.item_name = name.to_owned();
        self
    }

    /// Valid for `months` after activation.
    pub fn validity_months(mut self, months: i64) -> Self {
        self.validity_months = Some(months);
        self
    }

    /// Paid with invoice `invoice`.
    pub fn invoice(mut self, invoice: i64) -> Self {
        self.invoice = Some(invoice);
        self
    }

    /// Sold by the staff user `user`.
    pub fn sold_by(mut self, user: i64) -> Self {
        self.sold_by = Some(user);
        self
    }
}

/// What verticals ask to sell a prepaid package (wl-sales §4.4, §5.5):
/// the package starts `pending_payment` and activates when its invoice is
/// paid in full. `wl-sales` provides it; the credits are then used and given
/// back through [`PackageLedger`].
pub trait PackageSales: Send + Sync + 'static {
    /// Records a package as `pending_payment` and returns its id; `None`
    /// when `sales` is off. The default sells nothing.
    fn sell<'a>(
        &'a self,
        _state: &'a AppState,
        _sale: SellPackage,
    ) -> SalesFuture<'a, Option<PackageId>> {
        Box::pin(async { Ok(None) })
    }
}

/// The package sales when the `sales` module is off.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoPackageSales;

impl PackageSales for NoPackageSales {}

/// Makes `sales` the app's [`PackageSales`]; `wl-sales` calls it in `register`.
pub fn provide_package_sales(app: &mut Registry, sales: impl PackageSales) {
    let sales: Arc<dyn PackageSales> = Arc::new(sales);
    app.provide(sales);
}

/// The app's [`PackageSales`], or [`NoPackageSales`] when none is provided.
pub fn package_sales(state: &AppState) -> Arc<dyn PackageSales> {
    match state.provided::<Arc<dyn PackageSales>>() {
        Some(sales) => Arc::clone(&sales),
        None => Arc::new(NoPackageSales),
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
/// The subject type of a till session, for [`Subject`] references.
pub const TILL_SESSION_SUBJECT_TYPE: &str = "sales.till_session";

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

/// A till session was counted and closed (`close`, wl-sales §4.3,
/// `SAL.POS.01`). `wl-payments` creates the till's cash handover from it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TillSessionClosed {
    /// The till session (`sales.till_session:<id>`).
    pub session: i64,
    /// The till (register) the session ran on, e.g. `T1`. The cash holder.
    pub till: String,
    /// The session's cashier: the person recoveries are taken from.
    pub cashier_user: i64,
    /// The trading day of the session (Asia/Dubai).
    pub date: NaiveDate,
    /// The cash the till should hold, in fils.
    pub expected: i64,
    /// The cash counted in the session, in fils.
    pub counted: i64,
    /// The outlet or branch of the till, which bears a written-off loss.
    pub location: Option<String>,
    /// The legal entity of the cash.
    pub legal_entity_id: Option<i64>,
}

impl TillSessionClosed {
    /// The session as a [`Subject`].
    pub fn session_subject(&self) -> Subject {
        Subject::new(TILL_SESSION_SUBJECT_TYPE, self.session)
    }

    /// Counted less expected (negative: a shortage), in fils.
    pub fn variance(&self) -> i64 {
        self.counted - self.expected
    }
}

impl Event for TillSessionClosed {}

/// A till session as a vertical reads it (wl-sales §4.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TillView {
    /// The session (`sales.till_session:<id>`).
    pub id: i64,
    /// The till (register) it runs on.
    pub till: String,
    /// The outlet or branch of the till.
    pub location: Option<String>,
    /// The trading day.
    pub business_date: NaiveDate,
    /// `open`, `counting`, `closed` or `reviewed`.
    pub status: String,
    /// The cashier of the session.
    pub cashier_user: i64,
    /// The opening float, in fils.
    pub float: i64,
    /// The cash taken by counter sales on the session, in fils.
    pub cash_sales: i64,
    /// The card payments taken by counter sales on the session, in fils.
    pub card_sales: i64,
    /// The cash the till should hold (float plus cash sales), in fils.
    pub expected: i64,
    /// The cash counted at the close, in fils.
    pub counted: Option<i64>,
    /// Counted less expected, in fils (negative: a shortage).
    pub variance: Option<i64>,
}

/// What opens a till session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenTill {
    /// The till (register), e.g. `T1`.
    pub till: String,
    /// The outlet or branch.
    pub location: Option<String>,
    /// The legal entity of the cash.
    pub legal_entity_id: Option<i64>,
    /// The trading day.
    pub business_date: NaiveDate,
    /// The cashier of the session.
    pub cashier_user: i64,
    /// The opening float, in fils.
    pub float: i64,
}

/// What a vertical may ask of the till sessions without depending on
/// `wl-sales`: open one, read it, count and close it, reopen it and mark it
/// reviewed. A `by` of `None` runs the system twin of the transition (a seed,
/// a job, or a vertical that checked its own permission first). `wl-sales`
/// provides it; without it [`till_desk`] gives [`NoTillDesk`], which knows no
/// session.
pub trait TillDesk: Send + Sync + 'static {
    /// Opens a session; `None` when the sales module is off. 422 when the
    /// till already has a session for that day.
    fn open<'a>(
        &'a self,
        _state: &'a AppState,
        _by: Option<&'a AuthUser>,
        _till: OpenTill,
    ) -> SalesFuture<'a, Option<TillView>> {
        Box::pin(async { Ok(None) })
    }

    /// The session `id`, or `None`.
    fn find<'a>(&'a self, _state: &'a AppState, _id: i64) -> SalesFuture<'a, Option<TillView>> {
        Box::pin(async { Ok(None) })
    }

    /// The session of `till` for the trading day `date`, or `None`.
    fn of_day<'a>(
        &'a self,
        _state: &'a AppState,
        _till: &'a str,
        _date: NaiveDate,
    ) -> SalesFuture<'a, Option<TillView>> {
        Box::pin(async { Ok(None) })
    }

    /// Counts the session `id` and closes it with the `counted` cash
    /// (`start_count` and `close`); emits [`TillSessionClosed`] after the
    /// commit.
    fn submit_count<'a>(
        &'a self,
        _state: &'a AppState,
        _by: Option<&'a AuthUser>,
        _id: i64,
        _counted: i64,
    ) -> SalesFuture<'a, TillView> {
        Box::pin(async { Err(Error::NotFound) })
    }

    /// Reopens a closed session, with the reason.
    fn reopen<'a>(
        &'a self,
        _state: &'a AppState,
        _by: Option<&'a AuthUser>,
        _id: i64,
        _reason: &'a str,
    ) -> SalesFuture<'a, TillView> {
        Box::pin(async { Err(Error::NotFound) })
    }

    /// Marks a closed session reviewed.
    fn review<'a>(
        &'a self,
        _state: &'a AppState,
        _by: Option<&'a AuthUser>,
        _id: i64,
    ) -> SalesFuture<'a, TillView> {
        Box::pin(async { Err(Error::NotFound) })
    }
}

/// The desk when the `sales` module is off.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoTillDesk;

impl TillDesk for NoTillDesk {}

/// Makes `desk` the app's [`TillDesk`]; `wl-sales` calls it in `register`.
pub fn provide_till_desk(app: &mut Registry, desk: impl TillDesk) {
    let desk: Arc<dyn TillDesk> = Arc::new(desk);
    app.provide(desk);
}

/// The app's [`TillDesk`], or [`NoTillDesk`] when none is provided.
pub fn till_desk(state: &AppState) -> Arc<dyn TillDesk> {
    match state.provided::<Arc<dyn TillDesk>>() {
        Some(desk) => Arc::clone(&desk),
        None => Arc::new(NoTillDesk),
    }
}

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
    /// The outcome and fee of each line whose fulfilment had started
    /// (BL-SAL-07); empty when nothing had started.
    #[serde(default)]
    pub outcomes: Vec<LineOutcome>,
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
    /// The same value without VAT, in fils, as the lines returned carry it
    /// (a zero-rated line has none).
    #[serde(default)]
    pub net_value: i64,
    /// The credit note issued for it, when there is one.
    pub credit_note: Option<i64>,
    /// The complaint category of a *no goods* return (BL-SAL-14).
    #[serde(default)]
    pub complaint_category: Option<String>,
    /// The responsible party of a *no goods* return (BL-SAL-14).
    #[serde(default)]
    pub responsible_party: Option<String>,
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
    /// What happens to the started goods of the line (BL-SAL-08); `None`
    /// when the line's fulfilment had not started.
    #[serde(default)]
    pub outcome: Option<LineOutcomeKind>,
    /// The cancellation fee on the line, in fils (0 when none or waived).
    #[serde(default)]
    pub fee: i64,
    /// Whether the manager waived the fee.
    #[serde(default)]
    pub fee_waived: bool,
}

impl Event for SalesOrderLineCancelled {}

/// What happens to the goods of a line whose fulfilment had started when the
/// line (or the order) is cancelled (wl-sales §4.2 *Cancelling after
/// fulfilment started*). `wl-inventory` and the vertical act on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineOutcomeKind {
    /// Picked goods go back to stock and the reservation is released.
    ReturnToStock,
    /// The work order runs to the end and its output goes to free stock.
    FinishToStock,
    /// Work in progress or prepared goods are written off.
    Scrap,
}

impl LineOutcomeKind {
    /// Every outcome, in the order the screens offer them.
    pub const ALL: [LineOutcomeKind; 3] = [
        LineOutcomeKind::ReturnToStock,
        LineOutcomeKind::FinishToStock,
        LineOutcomeKind::Scrap,
    ];

    /// The stored name: `return_to_stock`, `finish_to_stock` or `scrap`.
    pub fn as_str(self) -> &'static str {
        match self {
            LineOutcomeKind::ReturnToStock => "return_to_stock",
            LineOutcomeKind::FinishToStock => "finish_to_stock",
            LineOutcomeKind::Scrap => "scrap",
        }
    }

    /// The outcome called `text`.
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str() == text.trim())
    }

    /// Whether choosing it needs a manager on its own (`SAL.ACL.05`: *finish
    /// to stock* and *scrap*).
    pub fn needs_manager(self) -> bool {
        !matches!(self, LineOutcomeKind::ReturnToStock)
    }
}

/// The outcome of one started line of a cancellation, as the events carry it
/// (BL-SAL-07, BL-SAL-08, BL-SAL-09).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineOutcome {
    /// The sale line.
    pub line: i64,
    /// What happens to its goods.
    pub kind: LineOutcomeKind,
    /// The cancellation fee on the line, in fils (0 when none or waived).
    pub fee: i64,
    /// Whether the manager waived the fee.
    pub fee_waived: bool,
}

/// One line of an order changed by an amendment (`SalesOrderAmended`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AmendedLine {
    /// The sale line (its id in `wl-sales`).
    pub line: i64,
    /// The quantity before, in thousandths (0 for an added line).
    pub quantity_before_milli: i64,
    /// The quantity after, in thousandths.
    pub quantity_after_milli: i64,
    /// The unit price before, in fils.
    pub unit_price_before: i64,
    /// The unit price after, in fils.
    pub unit_price_after: i64,
    /// The part of the added quantity held back from fulfilment until the
    /// payment link of a prepaid increase is paid, in thousandths.
    pub held_milli: i64,
}

/// A sales order was amended: a new revision is in force (BL-SAL-05).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SalesOrderAmended {
    /// The order.
    pub order: SalesOrderId,
    /// The customer.
    pub party: PartyId,
    /// The new revision.
    pub revision: i64,
    /// The lines that changed.
    pub lines: Vec<AmendedLine>,
    /// The total with VAT before, in fils.
    pub value_before: i64,
    /// The total with VAT after, in fils.
    pub value_after: i64,
    /// The reason.
    pub reason: String,
}

impl Event for SalesOrderAmended {}

/// Quantities held back by a prepaid increase are released to fulfilment
/// (BL-SAL-06): the payment link was paid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SalesOrderLinesReleased {
    /// The order.
    pub order: SalesOrderId,
    /// The customer.
    pub party: PartyId,
    /// The lines and the quantities now released.
    pub lines: Vec<FulfilledLine>,
}

impl Event for SalesOrderLinesReleased {}

/// What the vertical knows about the fulfilment of an order: which lines
/// have started (goods picked, a work order or production batch started,
/// the kitchen started the line). The module that does the work provides
/// it (`wl-inventory` for picking, the vertical for its own); the default
/// knows no started line (BL-SAL-05, BL-SAL-08, BL-SAL-09).
pub trait FulfilmentProgress: Send + Sync + 'static {
    /// Of the sale lines `lines` of `order`, those whose fulfilment started.
    fn started_lines<'a>(
        &'a self,
        _state: &'a AppState,
        _order: SalesOrderId,
        _lines: &'a [i64],
    ) -> SalesFuture<'a, Vec<i64>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// The progress when nothing provides one: no line has started.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoFulfilmentProgress;

impl FulfilmentProgress for NoFulfilmentProgress {}

/// Makes `progress` the app's [`FulfilmentProgress`].
pub fn provide_fulfilment_progress(app: &mut Registry, progress: impl FulfilmentProgress) {
    let progress: Arc<dyn FulfilmentProgress> = Arc::new(progress);
    app.provide(progress);
}

/// The app's [`FulfilmentProgress`], or [`NoFulfilmentProgress`] when none is
/// provided.
pub fn fulfilment_progress(state: &AppState) -> Arc<dyn FulfilmentProgress> {
    match state.provided::<Arc<dyn FulfilmentProgress>>() {
        Some(progress) => Arc::clone(&progress),
        None => Arc::new(NoFulfilmentProgress),
    }
}

/// A sales order was placed through a channel that needs the seller to accept
/// it, or its online payment was confirmed on such a channel: it waits in
/// `pending_acceptance` (06: the order is `placed`). `SalesOrderConfirmed`
/// follows when the seller accepts it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SalesOrderPlaced {
    /// The order.
    pub order: SalesOrderId,
    /// The customer.
    pub party: PartyId,
    /// The channel it came through.
    pub channel: String,
}

impl Event for SalesOrderPlaced {}

/// One line of a sales order, as a vertical reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderLineView {
    /// The sale line (its id in `wl-sales`).
    pub id: i64,
    /// The position, from 1.
    pub line_no: i64,
    /// The code of the item as sold.
    pub item_code: String,
    /// The description as sold.
    pub description: String,
    /// Whether the line is an item (a loyalty redemption line is not).
    pub is_item: bool,
    /// The quantity, in thousandths.
    pub quantity_milli: i64,
    /// The quantity handed over, in thousandths.
    pub fulfilled_milli: i64,
    /// The quantity cancelled, in thousandths.
    pub cancelled_milli: i64,
    /// The price of one unit before VAT, in fils.
    pub unit_price: i64,
    /// The line with VAT, in fils.
    pub gross: i64,
}

impl OrderLineView {
    /// The quantity still open, in thousandths.
    pub fn open_milli(&self) -> i64 {
        (self.quantity_milli - self.fulfilled_milli - self.cancelled_milli).max(0)
    }
}

/// A sales order, as a vertical reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderView {
    /// The order.
    pub id: SalesOrderId,
    /// The number, e.g. `SO-2026-0001`.
    pub number: String,
    /// The customer.
    pub party: PartyId,
    /// The channel it came through.
    pub channel: String,
    /// The number in the channel's own system (an aggregator's).
    pub external_number: Option<String>,
    /// `on_account`, `online`, `cash_on_delivery`, `cash` or `card`.
    pub payment_mode: String,
    /// Whether it was paid online in full before it was placed.
    pub prepaid: bool,
    /// The ISO currency code.
    pub currency: String,
    /// The total with VAT, in fils.
    pub gross_total: i64,
    /// The legal entity whose order it is.
    pub legal_entity: Option<i64>,
    /// The order's status, by its stored name.
    pub status: String,
    /// The revision in force (1 until the order is amended).
    pub revision: i64,
    /// What was refunded after the order was handed over, in fils (sales
    /// returns of kind *no goods*, BL-SAL-14).
    pub refunded_amount: i64,
    /// Whether the channel is an aggregator's: its orders are changed on the
    /// aggregator's own tablet and it refunds its customer itself.
    pub aggregator: bool,
    /// The lines, in order.
    pub lines: Vec<OrderLineView>,
}

/// An order cancelled by its vertical as the system, with no approval: the
/// vertical decided it (06: a failed own-rider delivery, `close_failed`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelByVertical {
    /// The order.
    pub order: SalesOrderId,
    /// Why; the reason of the events and of the stock movements.
    pub reason: String,
    /// What happens to the goods of each line the fulfilment had started.
    pub outcomes: Vec<(i64, LineOutcomeKind)>,
    /// Keep what the customer paid: `wl-sales` requests no refund and
    /// drafts no fee document; the vertical decides the refund itself.
    pub keep_payment: bool,
    /// The user the cancellation is on behalf of, if any.
    pub by: Option<i64>,
}

/// What a change asked of the sales order did (06: `amend`, `cancel_line`,
/// `cancel`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeOutcome {
    /// The change is made.
    Done,
    /// A manager must approve first (`SAL.ACL.01`, `SAL.ACL.05`): the request
    /// `request` waits and nothing changed yet. The vertical hears of the
    /// change when it is made (`SalesOrderAmended`, `SalesOrderLineCancelled`,
    /// `SalesOrderCancelled`).
    Waiting {
        /// The approval request (`ApprovalRequest` id).
        request: i64,
    },
}

/// The new quantity and price of one line of an amendment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmendChange {
    /// The sale line.
    pub line: i64,
    /// The quantity it will have, in thousandths (0 drops the line).
    pub quantity_milli: i64,
    /// The price of one unit before VAT, in fils.
    pub unit_price: i64,
}

/// A line an amendment adds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmendAdd {
    /// The item code.
    pub item_code: String,
    /// The description.
    pub description: String,
    /// The quantity, in thousandths.
    pub quantity_milli: i64,
    /// The price of one unit before VAT, in fils.
    pub unit_price: i64,
    /// The sale line whose VAT category the new line takes (a substitute is
    /// taxed as the item it replaces); `None`: standard rated.
    pub vat_like: Option<i64>,
}

/// An amendment of a placed order as a new revision (BL-SAL-05).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmendOrder {
    /// The order.
    pub order: SalesOrderId,
    /// Why.
    pub reason: String,
    /// The revision the user saw; 0 when not given.
    pub seen_revision: i64,
    /// The lines that change; the discount of a line stays.
    pub changes: Vec<AmendChange>,
    /// The lines that are added.
    pub added: Vec<AmendAdd>,
}

/// A cancelled line (BL-SAL-08).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelLine {
    /// The order.
    pub order: SalesOrderId,
    /// The sale line.
    pub line: i64,
    /// The quantity cancelled, in thousandths; 0 cancels all that is open.
    pub quantity_milli: i64,
    /// Why.
    pub reason: String,
    /// What happens to the goods when the line's fulfilment started.
    pub outcome: Option<LineOutcomeKind>,
}

/// A cancelled order, as a user asks for it (BL-SAL-07).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelOrder {
    /// The order.
    pub order: SalesOrderId,
    /// Why.
    pub reason: String,
    /// What happens to the goods of each line the fulfilment had started.
    pub outcomes: Vec<(i64, LineOutcomeKind)>,
}

/// A refund after completion with nothing coming back, raised and resolved
/// at once by the vertical as the system (BL-SAL-14): 06 reverses a delivery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoGoodsRefund {
    /// The order.
    pub order: SalesOrderId,
    /// The sale lines and the amount with VAT refunded of each, in fils.
    pub lines: Vec<(i64, i64)>,
    /// The complaint category (a stored name of the sales settings).
    pub category: String,
    /// The responsible party (a stored name of the sales settings).
    pub party: String,
    /// Why, in words.
    pub reason: String,
    /// Resolve by credit note only: no refund payment is asked (cash on
    /// delivery that nobody paid).
    pub credit_only: bool,
}

/// A refund an aggregator made to its customer itself and deducted from its
/// payout, recorded by the vertical as the system (BL-SAL-14, resolution
/// *refunded by the channel*): no approval, no refund payment, no credit note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelRefund {
    /// The order.
    pub order: SalesOrderId,
    /// The sale lines and the amount with VAT refunded of each, in fils.
    pub lines: Vec<(i64, i64)>,
    /// The complaint category (a stored name of the sales settings).
    pub category: String,
    /// The responsible party (a stored name of the sales settings).
    pub party: String,
    /// Why, in words (the aggregator's reason).
    pub reason: String,
}

/// What a vertical may ask of the sales orders without depending on
/// `wl-sales`: read an order with its lines, accept or reject it as a user,
/// cancel it as the system, change it as a user (amend, cancel a line or the
/// order) and refund it after completion. `wl-sales` provides it; without that module
/// [`order_desk`] gives [`NoSalesOrderDesk`], which knows no order.
pub trait SalesOrderDesk: Send + Sync + 'static {
    /// The order `order` with its lines, or `None`.
    fn find<'a>(
        &'a self,
        _state: &'a AppState,
        _order: SalesOrderId,
    ) -> SalesFuture<'a, Option<OrderView>> {
        Box::pin(async { Ok(None) })
    }

    /// Accepts the order that waits in `pending_acceptance` as `user`
    /// (`sales_orders.accept`).
    fn accept<'a>(
        &'a self,
        _state: &'a AppState,
        _user: &'a AuthUser,
        _order: SalesOrderId,
    ) -> SalesFuture<'a, ()> {
        Box::pin(async { Err(Error::NotFound) })
    }

    /// Rejects the order that waits in `pending_acceptance` as `user`, with a
    /// reason (`sales_orders.accept`).
    fn reject<'a>(
        &'a self,
        _state: &'a AppState,
        _user: &'a AuthUser,
        _order: SalesOrderId,
        _reason: &'a str,
    ) -> SalesFuture<'a, ()> {
        Box::pin(async { Err(Error::NotFound) })
    }

    /// Cancels the whole order as the system (see [`CancelByVertical`]).
    fn cancel<'a>(
        &'a self,
        _state: &'a AppState,
        _request: CancelByVertical,
    ) -> SalesFuture<'a, ()> {
        Box::pin(async { Err(Error::NotFound) })
    }

    /// Amends the order as `user` (`sales_orders.amend`); a started line asks
    /// a manager first.
    fn amend<'a>(
        &'a self,
        _state: &'a AppState,
        _user: &'a AuthUser,
        _request: AmendOrder,
    ) -> SalesFuture<'a, ChangeOutcome> {
        Box::pin(async { Err(Error::NotFound) })
    }

    /// Cancels one line as `user` (`sales_orders.cancel`); a started line
    /// needs an outcome and a manager.
    fn cancel_line<'a>(
        &'a self,
        _state: &'a AppState,
        _user: &'a AuthUser,
        _request: CancelLine,
    ) -> SalesFuture<'a, ChangeOutcome> {
        Box::pin(async { Err(Error::NotFound) })
    }

    /// Cancels the whole order as `user` (`sales_orders.cancel`); a started
    /// line needs an outcome, and a started line or a paid order above the
    /// limit needs a manager.
    fn cancel_order<'a>(
        &'a self,
        _state: &'a AppState,
        _user: &'a AuthUser,
        _request: CancelOrder,
    ) -> SalesFuture<'a, ChangeOutcome> {
        Box::pin(async { Err(Error::NotFound) })
    }

    /// Raises a *no goods* return of the handed-over order and resolves it as
    /// the system, with a refund or by credit note only. Returns the return's
    /// id.
    fn refund_no_goods<'a>(
        &'a self,
        _state: &'a AppState,
        _request: NoGoodsRefund,
    ) -> SalesFuture<'a, i64> {
        Box::pin(async { Err(Error::NotFound) })
    }

    /// The order of `channel` whose number in the channel's own system is
    /// `number` (trimmed, case-insensitive), with its lines, or `None`.
    fn find_external<'a>(
        &'a self,
        _state: &'a AppState,
        _channel: &'a str,
        _number: &'a str,
    ) -> SalesFuture<'a, Option<OrderView>> {
        Box::pin(async { Ok(None) })
    }

    /// Records a refund the aggregator made itself: raises a *no goods* return
    /// of the order and resolves it *refunded by the channel* as the system.
    /// Returns the return's id.
    fn refund_by_channel<'a>(
        &'a self,
        _state: &'a AppState,
        _request: ChannelRefund,
    ) -> SalesFuture<'a, i64> {
        Box::pin(async { Err(Error::NotFound) })
    }

    /// Takes the channel refund `sales_return` back as the system (a voided
    /// statement, or a refund the aggregator reversed): the order's amount
    /// refunded goes down; no money moves.
    fn reverse_by_channel<'a>(
        &'a self,
        _state: &'a AppState,
        _sales_return: i64,
        _reason: &'a str,
    ) -> SalesFuture<'a, ()> {
        Box::pin(async { Err(Error::NotFound) })
    }
}

/// The desk when the `sales` module is off.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoSalesOrderDesk;

impl SalesOrderDesk for NoSalesOrderDesk {}

/// Makes `desk` the app's [`SalesOrderDesk`]; `wl-sales` calls it in
/// `register`.
pub fn provide_order_desk(app: &mut Registry, desk: impl SalesOrderDesk) {
    let desk: Arc<dyn SalesOrderDesk> = Arc::new(desk);
    app.provide(desk);
}

/// The app's [`SalesOrderDesk`], or [`NoSalesOrderDesk`] when none is
/// provided.
pub fn order_desk(state: &AppState) -> Arc<dyn SalesOrderDesk> {
    match state.provided::<Arc<dyn SalesOrderDesk>>() {
        Some(desk) => Arc::clone(&desk),
        None => Arc::new(NoSalesOrderDesk),
    }
}

/// What a customer account owes and may owe (`SAL.RUL.01`, BL-SAL-10): the
/// exposure counts the open orders, invoices and cheques, without the order
/// being checked; the caller adds that order at the total it has now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreditPosition {
    /// The customer's exposure without the order being checked, in fils.
    pub exposure: i64,
    /// The customer's credit limit, in fils; `None` when it has none.
    pub limit: Option<i64>,
    /// The exposure of the customer's group without that order, when the
    /// limit is combined.
    pub group_exposure: Option<i64>,
    /// The group's combined limit.
    pub group_limit: Option<i64>,
}

/// The credit position of a customer. The module that holds the credit
/// limits provides it (BL-SAL-10); the default knows none, so no order is
/// held for credit.
pub trait CreditExposure: Send + Sync + 'static {
    /// The position of `party` in `currency` without the order `excluding`,
    /// or `None` without a limit.
    fn position<'a>(
        &'a self,
        _state: &'a AppState,
        _party: PartyId,
        _currency: &'a str,
        _excluding: &'a Subject,
    ) -> SalesFuture<'a, Option<CreditPosition>> {
        Box::pin(async { Ok(None) })
    }
}

/// The exposure when nothing provides one: no customer has a limit.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoCreditExposure;

impl CreditExposure for NoCreditExposure {}

/// Makes `exposure` the app's [`CreditExposure`].
pub fn provide_credit_exposure(app: &mut Registry, exposure: impl CreditExposure) {
    let exposure: Arc<dyn CreditExposure> = Arc::new(exposure);
    app.provide(exposure);
}

/// The app's [`CreditExposure`], or [`NoCreditExposure`] when none is
/// provided.
pub fn credit_exposure(state: &AppState) -> Arc<dyn CreditExposure> {
    match state.provided::<Arc<dyn CreditExposure>>() {
        Some(exposure) => Arc::clone(&exposure),
        None => Arc::new(NoCreditExposure),
    }
}

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
