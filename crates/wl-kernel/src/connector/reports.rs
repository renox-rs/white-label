//! The reports an assistant may run with `list_reports` and `run_report`
//! (BL-AGT-07): a module registers a report with its key, its permission
//! and its filters, and answers with rows limited to what the user may see
//! (the [`Access`] it is given: the user's scopes).
//!
//! The report library itself (wl-kernel §5.10) is not built yet: this is the
//! registry the connector reads, which the library will fill. A report
//! with more than [`MAX_ROWS`] rows returns the first ones and says how to
//! narrow it. Amounts are in the minor unit of their currency (fils for
//! AED), as the report's `amounts` note says.

use std::sync::{Arc, PoisonError, RwLock};

use renox::prelude::*;
use renox::serde_json::{Map, Value};
use serde::Serialize;

use crate::machines::BoxFuture;
use crate::search::Access;

/// The most rows a report returns to an assistant.
pub const MAX_ROWS: usize = 500;

/// The note on amounts every report result carries.
pub const AMOUNTS_NOTE: &str =
    "Amounts are integers in the currency's minor unit: fils for AED (100 fils = 1 AED).";

/// A filter a report takes.
#[derive(Debug, Clone, Serialize)]
pub struct ReportFilter {
    /// Its name in the `filters` object, e.g. `building`.
    pub name: String,
    /// What it does, for the assistant.
    pub description: String,
}

/// What a report answers.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ReportRows {
    /// The column names.
    pub columns: Vec<String>,
    /// The rows, each with one value per column.
    pub rows: Vec<Vec<Value>>,
    /// The totals, by column name.
    pub totals: Map<String, Value>,
}

type RunFn = Arc<
    dyn for<'a> Fn(&'a Db, &'a Access, Map<String, Value>) -> BoxFuture<'a, Result<ReportRows>>
        + Send
        + Sync,
>;

/// A report an assistant may run.
#[derive(Clone)]
pub struct AgentReport {
    key: String,
    title: String,
    description: String,
    permission: String,
    filters: Vec<ReportFilter>,
    run: RunFn,
}

impl AgentReport {
    /// The report `key` (`invoicing.ageing`), seen by holders of
    /// `permission`, answered by `run`.
    pub fn new<F>(key: &str, title: &str, permission: &str, run: F) -> Self
    where
        F: for<'a> Fn(&'a Db, &'a Access, Map<String, Value>) -> BoxFuture<'a, Result<ReportRows>>
            + Send
            + Sync
            + 'static,
    {
        AgentReport {
            key: key.to_owned(),
            title: title.to_owned(),
            description: String::new(),
            permission: permission.to_owned(),
            filters: Vec::new(),
            run: Arc::new(run),
        }
    }

    /// What it shows, for the assistant.
    pub fn description(mut self, description: &str) -> Self {
        self.description = description.to_owned();
        self
    }

    /// A filter it takes.
    pub fn filter(mut self, name: &str, description: &str) -> Self {
        self.filters.push(ReportFilter {
            name: name.to_owned(),
            description: description.to_owned(),
        });
        self
    }

    /// The key.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The permission it needs.
    pub fn permission(&self) -> &str {
        &self.permission
    }

    /// What `list_reports` says of it.
    pub fn info(&self) -> Value {
        json!({
            "key": self.key,
            "title": self.title,
            "description": self.description,
            "permission": self.permission,
            "filters": self.filters,
        })
    }

    /// Runs it for `access` with `filters`: at most [`MAX_ROWS`] rows, with
    /// `truncated` and how to narrow it when there were more.
    pub async fn run(&self, db: &Db, access: &Access, filters: Map<String, Value>) -> Result<Value> {
        let mut answer = (self.run)(db, access, filters).await?;
        let total_rows = answer.rows.len();
        let truncated = total_rows > MAX_ROWS;
        answer.rows.truncate(MAX_ROWS);
        let mut result = json!({
            "key": self.key,
            "title": self.title,
            "permission": self.permission,
            "amounts": AMOUNTS_NOTE,
            "columns": answer.columns,
            "rows": answer.rows,
            "totals": answer.totals,
            "row_count": total_rows,
            "truncated": truncated,
        });
        if truncated {
            let names: Vec<&str> = self.filters.iter().map(|f| f.name.as_str()).collect();
            result["narrow"] = Value::String(format!(
                "Only the first {MAX_ROWS} of {total_rows} rows are returned. Narrow the report \
                 with its filters ({}) to see the rest.",
                if names.is_empty() {
                    "none declared".to_owned()
                } else {
                    names.join(", ")
                }
            ));
        }
        Ok(result)
    }
}

static REPORTS: RwLock<Vec<AgentReport>> = RwLock::new(Vec::new());

/// Adds `report`, replacing one of the same key.
pub fn register_report(report: AgentReport) {
    let mut reports = REPORTS.write().unwrap_or_else(PoisonError::into_inner);
    match reports.iter_mut().find(|r| r.key == report.key) {
        Some(existing) => *existing = report,
        None => reports.push(report),
    }
}

/// Every registered report, in the order registered.
pub fn reports() -> Vec<AgentReport> {
    REPORTS.read().unwrap_or_else(PoisonError::into_inner).clone()
}

/// The report `key`, if registered.
pub fn report(key: &str) -> Option<AgentReport> {
    REPORTS
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .find(|r| r.key == key)
        .cloned()
}
