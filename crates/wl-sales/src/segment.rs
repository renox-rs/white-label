//! The customer segment (wl-sales §3 *Customer segment*, `SAL.LOY.01`;
//! BL-SAL-25): a saved set of rules that picks parties, combined with *all*
//! or *any*. A segment keeps its rules, never its members: they are worked
//! out each time the segment is used ([`crate::segments`]), so the preview
//! count is an estimate and the list at send time may differ.
//!
//! Decisions:
//! - The rule kinds built here are the ones the module's own data answers:
//!   days since the last sale, spend in a period, number of sales, an item
//!   bought, loyalty balance, package credits left and enrolled in loyalty.
//!   The vertical adds kinds through [`SegmentRule`](crate::segments::SegmentRule)
//!   and the [`Rule::Vertical`] rule (05 no booking in N days, 08 service
//!   due, 10 course completed). Birthday month, party group and branch need
//!   party and catalogue data the directory does not give yet.
//! - A rule on a clinical item (05, the codes in
//!   [`SalesConfig::clinical_items`]) is refused while the setting
//!   `segment_clinical_rules` is off (the default), and the editor does not
//!   offer such items.

use renox::db::DateTime;
use renox::prelude::*;
use renox::serde_json::Value;
use serde::{Deserialize, Serialize};

use crate::config::SalesConfig;
use crate::texts::english;

/// How the rules of a segment combine.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MatchMode {
    /// A party must meet every rule.
    #[default]
    All,
    /// A party must meet at least one rule.
    Any,
}

/// One rule of a segment. It is stored as JSON in the segment, tagged by
/// `kind`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Rule {
    /// The last sale was at least `days` days ago (a party with no sale does
    /// not meet it).
    NoSaleForDays {
        /// The days since the last sale.
        days: i64,
    },
    /// The sales of the last `days` days add up to more than `amount` fils
    /// before VAT.
    SpendOver {
        /// The amount in fils, exclusive.
        amount: i64,
        /// The period in days.
        days: i64,
    },
    /// At least `count` sales in the last `days` days.
    SalesAtLeast {
        /// The number of sales.
        count: i64,
        /// The period in days.
        days: i64,
    },
    /// The item `item` was bought, within the last `days` days when given.
    BoughtItem {
        /// The item code.
        item: String,
        /// The period in days; ever when empty.
        days: Option<i64>,
    },
    /// The member has at least `points` points.
    LoyaltyBalanceAtLeast {
        /// The points.
        points: i64,
    },
    /// The party holds at least `credits` credits in active packages.
    PackageCreditsAtLeast {
        /// The credits.
        credits: i64,
    },
    /// The party has (or has not) an open loyalty account that is not
    /// paused.
    EnrolledInLoyalty {
        /// Whether the party must be enrolled.
        enrolled: bool,
    },
    /// A rule kind the vertical added.
    Vertical {
        /// The kind the vertical registered.
        name: String,
        /// Its parameters, as the vertical reads them.
        params: Value,
    },
}

/// A saved set of rules that selects parties.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_sales_segments", unique(name))]
pub struct Segment {
    /// The primary key.
    pub id: i64,
    /// The name.
    pub name: String,
    /// How the rules combine.
    pub match_mode: MatchMode,
    /// The rules as a JSON list.
    pub rules: String,
    /// The member count of the last preview; an estimate.
    pub last_member_count: Option<i64>,
    /// When it was counted.
    pub last_counted_at: Option<DateTime>,
    /// Whether it is archived: kept for the campaigns that name it.
    pub archived: bool,
    /// The user who made it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// Why a segment could not be saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SegmentRefused {
    /// The name is blank.
    NameRequired,
    /// Another segment has the name.
    NameTaken,
    /// A segment with no rule would pick everybody.
    NoRules,
    /// A rule has a number out of range or a blank item; the text key says
    /// which.
    BadRule(&'static str),
    /// The rule is on a clinical item and the setting is off (05).
    ClinicalItem(String),
    /// There is no such segment.
    NotFound,
}

impl SegmentRefused {
    /// The English explanation.
    pub fn message(&self) -> String {
        match self {
            SegmentRefused::NameRequired => english("wl_sales.segments.problem.name", &[]),
            SegmentRefused::NameTaken => english("wl_sales.segments.problem.name_taken", &[]),
            SegmentRefused::NoRules => english("wl_sales.segments.problem.no_rules", &[]),
            SegmentRefused::BadRule(key) => english(key, &[]),
            SegmentRefused::ClinicalItem(item) => {
                english("wl_sales.segments.problem.clinical", &[("item", item)])
            }
            SegmentRefused::NotFound => english("wl_sales.segments.problem.not_found", &[]),
        }
    }

    /// The refusal as an HTTP 422.
    pub fn into_error(self) -> Error {
        Error::Status(StatusCode::UNPROCESSABLE_ENTITY, self.message())
    }
}

/// Checks one rule against the settings.
fn check_rule(rule: &Rule, config: &SalesConfig) -> Result<(), SegmentRefused> {
    let bad = |key: &'static str| Err(SegmentRefused::BadRule(key));
    match rule {
        Rule::NoSaleForDays { days } if *days < 1 => bad("wl_sales.segments.problem.days"),
        Rule::SpendOver { amount, days } if *amount < 0 || *days < 1 => {
            bad("wl_sales.segments.problem.spend")
        }
        Rule::SalesAtLeast { count, days } if *count < 1 || *days < 1 => {
            bad("wl_sales.segments.problem.count")
        }
        Rule::BoughtItem { item, days } => {
            if item.trim().is_empty() || days.is_some_and(|d| d < 1) {
                return bad("wl_sales.segments.problem.item");
            }
            if config.is_blocked_clinical(item) {
                return Err(SegmentRefused::ClinicalItem(item.trim().to_owned()));
            }
            Ok(())
        }
        Rule::LoyaltyBalanceAtLeast { points } if *points < 0 => {
            bad("wl_sales.segments.problem.points")
        }
        Rule::PackageCreditsAtLeast { credits } if *credits < 1 => {
            bad("wl_sales.segments.problem.credits")
        }
        Rule::Vertical { name, .. } if name.trim().is_empty() => {
            bad("wl_sales.segments.problem.vertical")
        }
        _ => Ok(()),
    }
}

impl Segment {
    /// The rules of the segment; none when the stored text is not a list of
    /// rules.
    pub fn rules(&self) -> Vec<Rule> {
        renox::serde_json::from_str(&self.rules).unwrap_or_default()
    }

    /// The segment named `name` with these rules, not saved.
    pub fn of(name: &str, match_mode: MatchMode, rules: &[Rule]) -> Segment {
        Segment {
            name: name.to_owned(),
            match_mode,
            rules: renox::serde_json::to_string(rules).unwrap_or_else(|_| "[]".to_owned()),
            ..Segment::default()
        }
    }

    /// Checks the fields, then creates the segment (`id` 0) or updates it.
    /// The count of the last preview is never taken from `self`.
    pub async fn store(
        mut self,
        db: &Db,
        config: &SalesConfig,
    ) -> Result<std::result::Result<Segment, SegmentRefused>> {
        self.name = self.name.trim().to_owned();
        if self.name.is_empty() {
            return Ok(Err(SegmentRefused::NameRequired));
        }
        let rules = self.rules();
        if rules.is_empty() {
            return Ok(Err(SegmentRefused::NoRules));
        }
        for rule in &rules {
            if let Err(refused) = check_rule(rule, config) {
                return Ok(Err(refused));
            }
        }
        let taken = Segment::query()
            .where_(Segment::NAME.eq(self.name.clone()))
            .where_(Segment::ID.ne(self.id))
            .exists(db)
            .await?;
        if taken {
            return Ok(Err(SegmentRefused::NameTaken));
        }
        if self.id == 0 {
            return Ok(Ok(Segment::create(db, self).await?));
        }
        let Some(current) = Segment::find(db, self.id).await? else {
            return Ok(Err(SegmentRefused::NotFound));
        };
        self.last_member_count = current.last_member_count;
        self.last_counted_at = current.last_counted_at;
        self.created_by = current.created_by;
        self.created_at = current.created_at;
        self.save(db).await?;
        Ok(Ok(self))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_are_stored_as_tagged_json() {
        let segment = Segment::of(
            "Lapsed",
            MatchMode::All,
            &[
                Rule::NoSaleForDays { days: 60 },
                Rule::SpendOver {
                    amount: 50_000,
                    days: 365,
                },
            ],
        );
        assert!(segment.rules.contains("\"kind\":\"no_sale_for_days\""));
        assert_eq!(segment.rules().len(), 2);
        assert_eq!(segment.rules()[0], Rule::NoSaleForDays { days: 60 });
    }

    #[test]
    fn an_unreadable_rule_text_has_no_rules() {
        let segment = Segment {
            rules: "not json".into(),
            ..Segment::default()
        };
        assert!(segment.rules().is_empty());
    }

    #[test]
    fn a_rule_with_a_number_out_of_range_is_refused() {
        let config = SalesConfig::default();
        assert!(check_rule(&Rule::NoSaleForDays { days: 0 }, &config).is_err());
        assert!(check_rule(&Rule::NoSaleForDays { days: 60 }, &config).is_ok());
        assert!(
            check_rule(
                &Rule::BoughtItem {
                    item: " ".into(),
                    days: None
                },
                &config
            )
            .is_err()
        );
        assert!(
            check_rule(
                &Rule::SalesAtLeast { count: 0, days: 30 },
                &config
            )
            .is_err()
        );
    }

    #[test]
    fn a_clinical_item_is_refused_unless_the_setting_is_on() {
        let mut config = SalesConfig {
            clinical_items: vec!["XRAY".into()],
            ..SalesConfig::default()
        };
        let rule = Rule::BoughtItem {
            item: "xray".into(),
            days: None,
        };
        assert_eq!(
            check_rule(&rule, &config),
            Err(SegmentRefused::ClinicalItem("xray".into()))
        );
        config.segment_clinical_rules = true;
        assert!(check_rule(&rule, &config).is_ok());
    }
}
