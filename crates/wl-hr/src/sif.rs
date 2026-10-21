//! SIF files of a payroll run (wl-hr §3 *Payroll run*: SIF files, one per
//! submission, versioned; `HRM.WPS.01`, issues #197 and #202).
//!
//! A version is a [`SifFile`] with the lines it carries ([`SifLine`], with
//! the net each was sent with). Versions are only added: a resubmit or a
//! regeneration makes version n+1, and no earlier file is overwritten or
//! deleted. The file is named with the MoHRE establishment ID of the run's
//! legal entity as it is when the version is generated
//! ([`crate::establishment`]).
//!
//! Decision: [`content`] is a plain salary information file (a header line
//! and one record per line); the layout of each bank or exchange house (a
//! "SIF variant", §7) is not built.
//!
//! Held lines and lines with a net of zero or less are left out. When no line
//! is left no file is made and the run's version does not move (a
//! supplementary run with only negative differences sends nothing).

use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::establishment;
use crate::payroll::{LineStatus, PayrollLine, PayrollRun};

/// One version of the salary file of a run.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_hr_sif_files", unique(run_id, version))]
pub struct SifFile {
    /// The primary key.
    pub id: i64,
    /// The run.
    #[model(references = "wl_hr_payroll_runs")]
    pub run_id: i64,
    /// 1, 2, ...
    pub version: i64,
    /// The MoHRE establishment ID the file is named with.
    pub establishment_id: Option<String>,
    /// The file's name.
    pub file_name: String,
    /// The file.
    pub content: String,
    /// How many lines it carries.
    pub line_count: i64,
    /// The sum of their nets, AED fils.
    pub total: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A line of a SIF version, with the net it was sent with.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_hr_sif_lines", index(sif_file_id), index(line_id))]
pub struct SifLine {
    /// The primary key.
    pub id: i64,
    /// The file.
    #[model(references = "wl_hr_sif_files")]
    pub sif_file_id: i64,
    /// The payroll line.
    #[model(references = "wl_hr_payroll_lines")]
    pub line_id: i64,
    /// The employee.
    pub employee_id: i64,
    /// The net sent, AED fils.
    pub net_amount: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl SifFile {
    /// The versions of run `run_id`, oldest first.
    pub async fn of_run(db: &Db, run_id: i64) -> Result<Vec<SifFile>> {
        SifFile::query()
            .where_(SifFile::RUN_ID.eq(run_id))
            .order_by(SifFile::VERSION)
            .get(db)
            .await
    }

    /// Version `version` of run `run_id`, inside `tx`.
    pub async fn version_in(
        tx: &mut Transaction,
        run_id: i64,
        version: i64,
    ) -> Result<Option<SifFile>> {
        SifFile::query()
            .where_(SifFile::RUN_ID.eq(run_id))
            .where_(SifFile::VERSION.eq(version))
            .first(&mut *tx)
            .await
    }

    /// The lines the file carries, oldest first, inside `tx`.
    pub async fn lines_in(&self, tx: &mut Transaction) -> Result<Vec<SifLine>> {
        SifLine::query()
            .where_(SifLine::SIF_FILE_ID.eq(self.id))
            .order_by(SifLine::ID)
            .get(&mut *tx)
            .await
    }
}

/// The name of version `version` of the file of `period`, with the
/// establishment ID (`NOID` when none is recorded).
pub fn file_name(establishment_id: Option<&str>, period: &str, version: i64) -> String {
    format!(
        "{}_{period}_v{version}.sif",
        establishment_id.unwrap_or("NOID")
    )
}

/// The text of a file: a header with the establishment, the period, the count
/// and the total, then one record per line.
pub fn content(establishment_id: Option<&str>, period: &str, lines: &[(i64, i64, i64)]) -> String {
    let total: i64 = lines.iter().map(|(_, _, net)| net).sum();
    let mut text = format!(
        "SCR,{},{period},{},{total}\n",
        establishment_id.unwrap_or("NOID"),
        lines.len()
    );
    for (line, employee, net) in lines {
        text.push_str(&format!("EDR,{employee},{line},{net}\n"));
    }
    text
}

/// Generates the next SIF version of run `run_id` inside `tx`: every `included`
/// line with a net above zero goes into it, or, with `only`, those of it that
/// are in `only`. No file when no line is left. The lines get the version.
pub async fn generate_in(tx: &mut Transaction, run_id: i64, only: Option<&[i64]>) -> Result {
    let Some(run) = PayrollRun::find(&mut *tx, run_id).await? else {
        return Ok(());
    };
    let lines: Vec<PayrollLine> = PayrollLine::query()
        .where_(PayrollLine::RUN_ID.eq(run_id))
        .where_(PayrollLine::STATUS.eq(LineStatus::Included))
        .order_by(PayrollLine::ID)
        .get(&mut *tx)
        .await?
        .into_iter()
        .filter(|l| l.net_amount > 0)
        .filter(|l| only.is_none_or(|ids| ids.contains(&l.id)))
        .collect();
    if lines.is_empty() {
        return Ok(());
    }
    let version = run.sif_version + 1;
    let establishment_id = establishment::id_of_in(&mut *tx, run.legal_entity_id).await?;
    let records: Vec<(i64, i64, i64)> = lines
        .iter()
        .map(|l| (l.id, l.employee_id, l.net_amount))
        .collect();
    let mut file = SifFile {
        run_id,
        version,
        establishment_id: establishment_id.clone(),
        file_name: file_name(establishment_id.as_deref(), &run.period, version),
        content: content(establishment_id.as_deref(), &run.period, &records),
        line_count: i64::try_from(lines.len()).unwrap_or(i64::MAX),
        total: lines.iter().map(|l| l.net_amount).sum(),
        ..SifFile::default()
    };
    file.insert(&mut *tx).await?;
    for line in &lines {
        let mut carried = SifLine {
            sif_file_id: file.id,
            line_id: line.id,
            employee_id: line.employee_id,
            net_amount: line.net_amount,
            ..SifLine::default()
        };
        carried.insert(&mut *tx).await?;
    }
    PayrollLine::query()
        .where_(PayrollLine::ID.is_in(lines.iter().map(|l| l.id).collect::<Vec<_>>()))
        .update(
            &mut *tx,
            &[("sif_version", &Some(version)), ("updated_at", &now())],
        )
        .await?;
    PayrollRun::query()
        .where_(PayrollRun::ID.eq(run_id))
        .update(
            &mut *tx,
            &[("sif_version", &version), ("updated_at", &now())],
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_is_named_with_the_establishment_id() {
        assert_eq!(
            file_name(Some("EST-1010"), "2026-11", 2),
            "EST-1010_2026-11_v2.sif"
        );
        assert_eq!(file_name(None, "2026-11", 1), "NOID_2026-11_v1.sif");
    }

    #[test]
    fn the_content_has_a_header_and_a_record_per_line() {
        let text = content(
            Some("EST-1001"),
            "2026-11",
            &[(1, 10, 300_000), (2, 11, 100)],
        );
        let rows: Vec<&str> = text.lines().collect();
        assert_eq!(rows[0], "SCR,EST-1001,2026-11,2,300100");
        assert_eq!(rows[1], "EDR,10,1,300000");
        assert_eq!(rows.len(), 3);
    }
}
