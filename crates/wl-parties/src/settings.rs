//! What a confirmed sanctions match does (wl-parties §7): the settings of
//! `confirm_match`, set with the builders on [`crate::WlParties`] and read
//! with [`ConfirmedMatchSettings::of`].

use renox::prelude::*;

/// The default hours, after the flag, until the freeze and report tasks are
/// due.
pub const DEFAULT_TASK_DUE_HOURS: i64 = 24;

/// The settings of a confirmed match. A change affects only the tasks
/// created afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfirmedMatchSettings {
    /// Whether the companies the matched person is a UBO, director or
    /// signatory of are blocked too (default on).
    pub related_company_block: bool,
    /// Hours after the flag until the freeze task is due (default 24).
    pub freeze_task_due_hours: i64,
    /// Hours after the flag until the report task is due (default 24; the
    /// open L-30 parameter, so nothing about the deadline is hard-coded).
    pub report_task_due_hours: i64,
}

impl ConfirmedMatchSettings {
    /// The defaults.
    pub const DEFAULT: ConfirmedMatchSettings = ConfirmedMatchSettings {
        related_company_block: true,
        freeze_task_due_hours: DEFAULT_TASK_DUE_HOURS,
        report_task_due_hours: DEFAULT_TASK_DUE_HOURS,
    };

    /// The app's settings, or the defaults.
    pub fn of(state: &AppState) -> ConfirmedMatchSettings {
        state
            .provided::<ConfirmedMatchSettings>()
            .map(|s| *s)
            .unwrap_or_default()
    }
}

impl Default for ConfirmedMatchSettings {
    fn default() -> Self {
        ConfirmedMatchSettings::DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_block_related_companies_and_give_24_hours() {
        let settings = ConfirmedMatchSettings::default();
        assert!(settings.related_company_block);
        assert_eq!(settings.freeze_task_due_hours, 24);
        assert_eq!(settings.report_task_due_hours, 24);
    }
}
