//! Document numbering (wl-kernel §5.5): sequences per legal entity and
//! document type, such as `INV-ACME-2026-0001`, on Renox's [`Sequence`].
//!
//! Gapless series (tax invoices, credit notes, receipts) take their number
//! inside the caller's transaction, so a rollback gives it back. Others
//! (tickets, orders) may skip numbers and never lock.

use renox::chrono::NaiveDate;
use renox::db::{NumberFormat, Period, Sequence, Transaction};
use renox::prelude::*;

use crate::entities::LegalEntity;

/// When a series starts again at 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reset {
    /// Never.
    Never,
    /// On 1 January.
    Yearly,
    /// On the 1st of each month.
    Monthly,
}

/// One numbering series, e.g. tax invoices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Series {
    /// The key, e.g. `invoice`.
    pub key: &'static str,
    /// The pattern: `{prefix}`, `{entity}`, `{yyyy}`, `{yy}`, `{mm}` and
    /// `{0000}` (the number, zero-padded).
    pub pattern: &'static str,
    /// The default prefix, e.g. `INV`. A client may change it.
    pub prefix: &'static str,
    /// When the count restarts.
    pub reset: Reset,
    /// Whether numbers may never be skipped.
    pub gapless: bool,
    /// Whether each legal entity counts on its own. Series that are not
    /// legal documents (tickets) are numbered across the deployment.
    pub per_entity: bool,
}

impl Series {
    fn sequence(&self, entity: Option<&LegalEntity>, on: NaiveDate) -> Sequence {
        let mut seq = Sequence::new(self.key);
        if self.per_entity
            && let Some(entity) = entity
        {
            seq = seq.scope(entity.id);
        }
        match self.reset {
            Reset::Never => seq,
            Reset::Yearly => seq.period(Period::of_year(on)),
            Reset::Monthly => seq.period(Period::of_month(on)),
        }
    }

    fn render(&self, prefix: &str, entity: Option<&LegalEntity>, n: i64, on: NaiveDate) -> String {
        NumberFormat::new(self.pattern)
            .token("prefix", prefix)
            .token("entity", entity.map_or("", |e| e.code.as_str()))
            .render(n, on)
    }

    /// The next number of a gapless series, taken inside `tx`. Use it in the
    /// transaction that issues the document, and only then: a number taken
    /// earlier (at submit, say) would leave a gap if the request were
    /// rejected.
    pub async fn next(
        &self,
        tx: &mut Transaction,
        prefix: Option<&str>,
        entity: Option<&LegalEntity>,
        on: NaiveDate,
    ) -> Result<String> {
        self.check(entity)?;
        let n = self.sequence(entity, on).next(tx).await?;
        Ok(self.render(prefix.unwrap_or(self.prefix), entity, n, on))
    }

    /// The next number of a series that may skip numbers, in its own
    /// transaction.
    pub async fn next_allowing_gaps(
        &self,
        db: &Db,
        prefix: Option<&str>,
        entity: Option<&LegalEntity>,
        on: NaiveDate,
    ) -> Result<String> {
        self.check(entity)?;
        let n = self.sequence(entity, on).next_allowing_gaps(db).await?;
        Ok(self.render(prefix.unwrap_or(self.prefix), entity, n, on))
    }

    fn check(&self, entity: Option<&LegalEntity>) -> Result<()> {
        if self.per_entity && entity.is_none() {
            return Err(renox::anyhow::anyhow!(
                "series `{}` is numbered per legal entity",
                self.key
            )
            .into());
        }
        if let Some(entity) = entity
            && !entity.is_active()
        {
            return Err(renox::anyhow::anyhow!("legal entity {} is inactive", entity.code).into());
        }
        Ok(())
    }
}
