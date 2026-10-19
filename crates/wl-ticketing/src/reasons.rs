//! Reassignment reasons (wl-ticketing §7, #146 A11). They live in code: the
//! §7 defaults, plus the codes a vertical adds with
//! [`WlTicketing::reassign_reason`](crate::WlTicketing::reassign_reason).

/// One reassignment reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReassignReason {
    /// The code stored with the assignment, e.g. `vendor_no_show`.
    pub code: String,
    /// The English name.
    pub name: String,
    /// Whether a free-text explanation is required with it.
    pub text_required: bool,
}

/// The codes a reassignment may use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReassignReasons {
    reasons: Vec<ReassignReason>,
}

impl Default for ReassignReasons {
    /// The five §7 defaults.
    fn default() -> Self {
        let mut reasons = ReassignReasons {
            reasons: Vec::new(),
        };
        reasons.add("vendor_no_show", "Vendor did not turn up", false);
        reasons.add("contract_ended", "Contract ended", false);
        reasons.add("vendor_document_expired", "Vendor document expired", false);
        reasons.add("workload", "Workload", false);
        reasons.add(Self::OTHER, "Other", true);
        reasons
    }
}

impl ReassignReasons {
    /// The `other` code, whose text is required.
    pub const OTHER: &'static str = "other";

    /// Adds `code`, or replaces the reason that has it.
    pub fn add(&mut self, code: &str, name: &str, text_required: bool) -> &mut Self {
        let reason = ReassignReason {
            code: code.to_owned(),
            name: name.to_owned(),
            text_required,
        };
        match self.reasons.iter_mut().find(|r| r.code == code) {
            Some(found) => *found = reason,
            None => self.reasons.push(reason),
        }
        self
    }

    /// The reason `code`, if it is known.
    pub fn get(&self, code: &str) -> Option<&ReassignReason> {
        self.reasons.iter().find(|r| r.code == code)
    }

    /// Every reason, in the order added.
    pub fn all(&self) -> &[ReassignReason] {
        &self.reasons
    }

    /// Checks a reassignment's `code` and `text`: the code must be known and
    /// a reason that requires text needs a non-blank one.
    pub fn check(&self, code: &str, text: &str) -> Result<(), ReasonRefused> {
        let reason = self.get(code).ok_or(ReasonRefused::Unknown)?;
        if reason.text_required && text.trim().is_empty() {
            return Err(ReasonRefused::TextRequired);
        }
        Ok(())
    }
}

/// Why a reassignment reason was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasonRefused {
    /// The code is not a known reason.
    Unknown,
    /// The reason needs an explanation and none was given.
    TextRequired,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_five_codes() {
        let reasons = ReassignReasons::default();
        let codes: Vec<_> = reasons.all().iter().map(|r| r.code.as_str()).collect();
        assert_eq!(
            codes,
            [
                "vendor_no_show",
                "contract_ended",
                "vendor_document_expired",
                "workload",
                "other"
            ]
        );
        assert!(reasons.get("other").is_some_and(|r| r.text_required));
        assert!(!reasons.get("workload").is_some_and(|r| r.text_required));
    }

    #[test]
    fn check_refuses_unknown_codes_and_blank_other_text() {
        let reasons = ReassignReasons::default();
        assert_eq!(reasons.check("vendor_no_show", ""), Ok(()));
        assert_eq!(reasons.check("nope", "x"), Err(ReasonRefused::Unknown));
        assert_eq!(reasons.check("", "x"), Err(ReasonRefused::Unknown));
        assert_eq!(
            reasons.check(ReassignReasons::OTHER, "  "),
            Err(ReasonRefused::TextRequired)
        );
        assert_eq!(reasons.check(ReassignReasons::OTHER, "Moved site"), Ok(()));
    }

    #[test]
    fn a_vertical_can_add_a_code_and_replace_one() {
        let mut reasons = ReassignReasons::default();
        reasons.add("tenant_request", "Tenant asked for someone else", false);
        assert_eq!(reasons.check("tenant_request", ""), Ok(()));
        assert_eq!(reasons.all().len(), 6);
        reasons.add("workload", "Workload", true);
        assert_eq!(reasons.all().len(), 6);
        assert_eq!(
            reasons.check("workload", ""),
            Err(ReasonRefused::TextRequired)
        );
    }
}
