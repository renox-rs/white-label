//! `client.toml`: what one client's deployment is ([ARCHITECTURE.md] §7).
//!
//! The host reads it at boot. Its values seed the settings store and the
//! legal entities on first run; later changes in the admin win over it.
//!
//! [ARCHITECTURE.md]: https://github.com/renox-rs/white-label/blob/main/docs/ARCHITECTURE.md

use std::collections::BTreeSet;
use std::path::Path;

use renox::chrono::NaiveDate;
use serde::Deserialize;

use crate::calendar::BusinessCalendar;
use crate::manifest::Manifest;
use crate::scopes::COMPANY_ADMIN_ROLE;

/// The whole file. Unknown top-level tables are module sections; unknown
/// keys inside the kernel's own tables are refused.
#[derive(Debug, Clone, Deserialize)]
pub struct ClientConfig {
    /// `[client]`: who the client is and which vertical app they run.
    pub client: Client,
    /// `[branding]`: logo, colours and font.
    #[serde(default)]
    pub branding: Branding,
    /// `[modules]`: the modules this deployment runs.
    #[serde(default)]
    pub modules: Modules,
    /// `[[legal_entities]]`: the companies of the client's group that issue
    /// documents ([decision 0003]). Empty means one entity named after the
    /// client.
    ///
    /// [decision 0003]: https://github.com/renox-rs/white-label/blob/main/docs/decisions/0003-legal-entities.md
    #[serde(default)]
    pub legal_entities: Vec<LegalEntityConfig>,
    /// `[calendar]`: the client's public holidays (wl-kernel §6
    /// `BusinessCalendar`).
    #[serde(default)]
    pub calendar: CalendarConfig,
    /// `[roles]`: which role plays which part in the deployment.
    #[serde(default)]
    pub roles: RolesConfig,
    /// The modules' own sections (`[invoicing]`, `[ticketing]`, …), read by
    /// each module.
    #[serde(flatten)]
    pub sections: toml::Table,
}

/// `[client]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Client {
    /// The client's trading name, e.g. `Acme Properties LLC`.
    pub name: String,
    /// The vertical app: `property`, `manpower`, …
    pub app: String,
    /// The domain the deployment answers on.
    #[serde(default)]
    pub domain: Option<String>,
    /// The languages, the first one the default. Only `en` and `ar`.
    #[serde(default = "default_locales")]
    pub locales: Vec<String>,
}

/// `[branding]`. Defaults are the platform's own look (wl-kernel §7).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Branding {
    /// A path to the logo, relative to the client's directory.
    #[serde(default)]
    pub logo: Option<String>,
    /// The main colour, `#rrggbb`.
    #[serde(default = "default_accent")]
    pub accent: String,
    /// The second colour, `#rrggbb`.
    #[serde(default = "default_second")]
    pub second: String,
    /// The display font family.
    #[serde(default = "default_font")]
    pub display_font: String,
    /// `classic` gives the UI kit's older look.
    #[serde(default)]
    pub theme: Option<String>,
}

impl Default for Branding {
    fn default() -> Self {
        Branding {
            logo: None,
            accent: default_accent(),
            second: default_second(),
            display_font: default_font(),
            theme: None,
        }
    }
}

/// `[modules]`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Modules {
    /// Module names without the `wl-` prefix: `parties`, `invoicing`, …
    /// The kernel is always on and is not listed.
    #[serde(default)]
    pub enabled: Vec<String>,
}

/// `[roles]`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RolesConfig {
    /// The key of the business owner's role, told about compliance-critical
    /// events. Defaults to [`COMPANY_ADMIN_ROLE`].
    #[serde(default)]
    pub owner: Option<String>,
}

/// The business owner role key of this deployment: `[roles] owner`, else
/// [`COMPANY_ADMIN_ROLE`].
pub fn owner_role(config: &ClientConfig) -> &str {
    config.owner_role()
}

/// `[calendar]`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalendarConfig {
    /// Public holidays as `YYYY-MM-DD` strings, e.g. `["2026-12-02"]`:
    /// not working days.
    #[serde(default)]
    pub holidays: Vec<NaiveDate>,
    /// The daily normal hours of a working day. Defaults to 8.
    #[serde(default)]
    pub normal_hours: Option<u32>,
    /// The periods with shorter normal hours (Ramadan), `[[calendar.reduced_hours]]`.
    #[serde(default)]
    pub reduced_hours: Vec<ReducedHoursConfig>,
}

/// One `[[calendar.reduced_hours]]` entry: the normal hours are `hours_less`
/// shorter from `from` to `to` (included).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReducedHoursConfig {
    /// The first day, `YYYY-MM-DD`.
    pub from: NaiveDate,
    /// The last day, `YYYY-MM-DD`.
    pub to: NaiveDate,
    /// Hours taken off each day of the period, e.g. 2 for Ramadan.
    pub hours_less: u32,
}

/// One `[[legal_entities]]` entry.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegalEntityConfig {
    /// A short code used in document numbers, e.g. `ACME`: capital
    /// letters and digits, 2 to 10 of them.
    pub code: String,
    /// The legal name in English, as on the trade licence.
    pub name_en: String,
    /// The legal name in Arabic.
    #[serde(default)]
    pub name_ar: Option<String>,
    /// The trade licence number.
    #[serde(default)]
    pub trade_licence_no: Option<String>,
    /// Who issued the licence: a mainland emirate's department or a free zone.
    #[serde(default)]
    pub licensing_authority: Option<String>,
    /// The emirate, e.g. `dubai`.
    #[serde(default)]
    pub emirate: Option<String>,
    /// The tax registration number: 15 digits. None when not VAT registered.
    #[serde(default)]
    pub trn: Option<String>,
    /// The address in English.
    #[serde(default)]
    pub address_en: Option<String>,
    /// The address in Arabic.
    #[serde(default)]
    pub address_ar: Option<String>,
    /// The footer on documents, English.
    #[serde(default)]
    pub footer_en: Option<String>,
    /// The footer on documents, Arabic.
    #[serde(default)]
    pub footer_ar: Option<String>,
    /// The entity documents use when their scope names none. Exactly one
    /// entity is the default; with one entity it is that one.
    #[serde(default)]
    pub default: bool,
}

fn default_locales() -> Vec<String> {
    vec!["en".into(), "ar".into()]
}

fn default_accent() -> String {
    "#0b6e66".into()
}

fn default_second() -> String {
    "#b8532e".into()
}

fn default_font() -> String {
    "Poppins".into()
}

/// Why a `client.toml` was refused. Boot stops with this message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "client.toml: {}", self.0)
    }
}

impl std::error::Error for ConfigError {}

impl ClientConfig {
    /// Reads and checks the file at `path`.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .map_err(|e| ConfigError(format!("cannot read {}: {e}", path.display())))?;
        Self::parse(&text)
    }

    /// Reads and checks a file's text.
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let mut config: ClientConfig =
            toml::from_str(text).map_err(|e| ConfigError(e.to_string()))?;
        config.check()?;
        if config.legal_entities.is_empty() {
            config
                .legal_entities
                .push(LegalEntityConfig::named_after(&config.client.name));
        }
        if config.legal_entities.len() == 1 {
            config.legal_entities[0].default = true;
        }
        Ok(config)
    }

    /// The enabled modules with their dependencies checked.
    pub fn manifest(&self) -> Result<Manifest, ConfigError> {
        Manifest::new(&self.modules.enabled)
    }

    /// The business calendar: the kernel's working days and hours with the
    /// `[calendar]` holidays.
    pub fn business_calendar(&self) -> BusinessCalendar {
        let mut calendar = BusinessCalendar::new(self.calendar.holidays.iter().copied());
        if let Some(hours) = self.calendar.normal_hours {
            calendar = calendar.with_normal_hours(hours);
        }
        for period in &self.calendar.reduced_hours {
            calendar = calendar.with_reduced_hours(period.from, period.to, period.hours_less);
        }
        calendar
    }

    /// A module's own section, e.g. `section("invoicing")` for `[invoicing]`.
    pub fn section(&self, name: &str) -> Option<&toml::Table> {
        self.sections.get(name).and_then(toml::Value::as_table)
    }

    /// The business owner role key: `[roles] owner`, else
    /// [`COMPANY_ADMIN_ROLE`].
    pub fn owner_role(&self) -> &str {
        self.roles
            .owner
            .as_deref()
            .filter(|role| !role.trim().is_empty())
            .unwrap_or(COMPANY_ADMIN_ROLE)
    }

    /// The default language.
    pub fn default_locale(&self) -> &str {
        self.client.locales.first().map_or("en", String::as_str)
    }

    fn check(&self) -> Result<(), ConfigError> {
        let fail = |message: String| Err(ConfigError(message));
        if self.client.name.trim().is_empty() {
            return fail("[client] name is empty".into());
        }
        if self.client.locales.is_empty() {
            return fail("[client] locales is empty".into());
        }
        for locale in &self.client.locales {
            if locale != "en" && locale != "ar" {
                return fail(format!(
                    "[client] locale `{locale}` is not supported (en, ar)"
                ));
            }
        }
        self.manifest()?;
        let mut codes = BTreeSet::new();
        for entity in &self.legal_entities {
            let code = &entity.code;
            let well_formed = (2..=10).contains(&code.len())
                && code
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
            if !well_formed {
                return fail(format!(
                    "legal entity code `{code}`: use 2 to 10 capital letters or digits"
                ));
            }
            if !codes.insert(code.as_str()) {
                return fail(format!("legal entity code `{code}` is listed twice"));
            }
            if entity.name_en.trim().is_empty() {
                return fail(format!("legal entity `{code}`: name_en is empty"));
            }
            if let Some(trn) = &entity.trn
                && !crate::entities::is_trn(trn)
            {
                return fail(format!(
                    "legal entity `{code}`: TRN `{trn}` is not 15 digits"
                ));
            }
        }
        let defaults = self.legal_entities.iter().filter(|e| e.default).count();
        if self.legal_entities.len() > 1 && defaults != 1 {
            return fail(format!(
                "{} legal entities are marked default; mark exactly one",
                defaults
            ));
        }
        Ok(())
    }
}

impl LegalEntityConfig {
    /// The one entity of a client that lists none: named after the client.
    fn named_after(client: &str) -> Self {
        let code: String = client
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|c| c.to_ascii_uppercase())
            .take(4)
            .collect();
        LegalEntityConfig {
            code: if code.len() < 2 { "MAIN".into() } else { code },
            name_en: client.to_owned(),
            name_ar: None,
            trade_licence_no: None,
            licensing_authority: None,
            emirate: None,
            trn: None,
            address_en: None,
            address_ar: None,
            footer_en: None,
            footer_ar: None,
            default: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
        [client]
        name = "Acme Properties LLC"
        app = "property"
    "#;

    #[test]
    fn the_owner_role_defaults_to_the_company_admin() {
        let config = ClientConfig::parse(MINIMAL).unwrap();
        assert_eq!(config.owner_role(), "admin");
        assert_eq!(owner_role(&config), "admin");
        let text = format!("{MINIMAL}\n[roles]\nowner = \"md\"\n");
        let config = ClientConfig::parse(&text).unwrap();
        assert_eq!(owner_role(&config), "md");
        let text = format!("{MINIMAL}\n[roles]\nboss = \"md\"\n");
        assert!(ClientConfig::parse(&text).is_err());
    }

    #[test]
    fn a_minimal_file_gets_the_defaults() {
        let config = ClientConfig::parse(MINIMAL).unwrap();
        assert_eq!(config.client.locales, ["en", "ar"]);
        assert_eq!(config.branding.accent, "#0b6e66");
        assert_eq!(config.legal_entities.len(), 1);
        let entity = &config.legal_entities[0];
        assert_eq!(entity.code, "ACME");
        assert_eq!(entity.name_en, "Acme Properties LLC");
        assert!(entity.default);
    }

    #[test]
    fn module_sections_are_kept_for_their_modules() {
        let text = format!("{MINIMAL}\n[ticketing]\nstatuses = [\"open\", \"done\"]\n");
        let config = ClientConfig::parse(&text).unwrap();
        let statuses = config.section("ticketing").unwrap()["statuses"]
            .as_array()
            .unwrap();
        assert_eq!(statuses.len(), 2);
        assert!(config.section("invoicing").is_none());
    }

    #[test]
    fn calendar_holidays_are_read_and_not_a_module_section() {
        let text = format!("{MINIMAL}\n[calendar]\nholidays = [\"2026-12-02\", \"2026-12-03\"]\n");
        let config = ClientConfig::parse(&text).unwrap();
        assert!(config.section("calendar").is_none());
        let calendar = config.business_calendar();
        let holidays: Vec<String> = calendar.holidays().map(|d| d.to_string()).collect();
        assert_eq!(holidays, ["2026-12-02", "2026-12-03"]);
        assert!(!calendar.is_working_day("2026-12-02".parse().unwrap()));
        let none = ClientConfig::parse(MINIMAL).unwrap().business_calendar();
        assert_eq!(none.holidays().count(), 0);
    }

    #[test]
    fn normal_hours_and_reduced_periods_are_read() {
        let text = format!(
            "{MINIMAL}\n[calendar]\nnormal_hours = 9\n\n[[calendar.reduced_hours]]\nfrom = \"2026-11-09\"\nto = \"2026-11-13\"\nhours_less = 2\n"
        );
        let calendar = ClientConfig::parse(&text).unwrap().business_calendar();
        let day = |text: &str| -> NaiveDate { text.parse().unwrap() };
        assert_eq!(calendar.normal_minutes_on(day("2026-11-03")), 9 * 60);
        assert_eq!(calendar.normal_minutes_on(day("2026-11-10")), 7 * 60);
        let plain = ClientConfig::parse(MINIMAL).unwrap().business_calendar();
        assert_eq!(plain.normal_minutes_on(day("2026-11-03")), 8 * 60);
        let unknown = format!(
            "{MINIMAL}\n[[calendar.reduced_hours]]\nfrom = \"2026-11-09\"\nto = \"2026-11-13\"\nhours_less = 2\nname = \"x\"\n"
        );
        assert!(ClientConfig::parse(&unknown).is_err());
    }

    #[test]
    fn a_bad_holiday_or_calendar_key_is_refused() {
        let bad_date = format!("{MINIMAL}\n[calendar]\nholidays = [\"2026-13-02\"]\n");
        assert!(ClientConfig::parse(&bad_date).is_err());
        let bad_key = format!("{MINIMAL}\n[calendar]\nweekend = [\"fri\"]\n");
        assert!(ClientConfig::parse(&bad_key).is_err());
    }

    #[test]
    fn several_entities_need_exactly_one_default() {
        let text = format!(
            "{MINIMAL}
            [[legal_entities]]
            code = \"ACME\"
            name_en = \"Acme Properties LLC\"
            [[legal_entities]]
            code = \"ACFZ\"
            name_en = \"Acme FZ-LLC\"
            "
        );
        let err = ClientConfig::parse(&text).unwrap_err();
        assert!(
            err.0.contains("0 legal entities are marked default"),
            "{err}"
        );
    }

    #[test]
    fn a_bad_trn_or_code_is_refused() {
        let bad_trn = format!(
            "{MINIMAL}\n[[legal_entities]]\ncode = \"ACME\"\nname_en = \"Acme\"\ntrn = \"12345\"\n"
        );
        assert!(
            ClientConfig::parse(&bad_trn)
                .unwrap_err()
                .0
                .contains("not 15 digits")
        );
        let bad_code =
            format!("{MINIMAL}\n[[legal_entities]]\ncode = \"acme\"\nname_en = \"Acme\"\n");
        assert!(
            ClientConfig::parse(&bad_code)
                .unwrap_err()
                .0
                .contains("capital letters")
        );
    }

    #[test]
    fn an_unknown_locale_or_key_is_refused() {
        let text = MINIMAL.replace(
            "app = \"property\"",
            "app = \"property\"\nlocales = [\"fr\"]",
        );
        assert!(ClientConfig::parse(&text).unwrap_err().0.contains("`fr`"));
        let text = MINIMAL.replace("app = \"property\"", "app = \"property\"\ncolour = \"red\"");
        assert!(ClientConfig::parse(&text).is_err());
    }
}
