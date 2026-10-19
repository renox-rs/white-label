//! Uploading an official sanctions list by hand when the download fails
//! (wl-parties §5.4, issue #42).
//!
//! This holds the upload log ([`SanctionsListUpload`]), the
//! `manual_upload` setting ([`UploadSettings`]) and the checks an upload
//! passes before it can replace a list: format, source URL, list date and
//! size, then the file itself through the format's
//! [`ListParser`](crate::list_parser::ListParser). The service that applies
//! an accepted upload to the list builds on [`validate`].

use std::fmt;

use renox::chrono::{Duration, NaiveDate};
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::events::SanctionsListUpdated;
use crate::list_parser::{ListParsers, ParsedList};
use crate::permissions;
use crate::sanctions_lists::{ListFormat, ListStatus, SanctionsList, UpdateSource};

/// The largest file an upload may have, in bytes (20 MB).
pub const MAX_UPLOAD_BYTES: usize = 20 * 1024 * 1024;

/// The longest source URL, in characters.
pub const MAX_SOURCE_URL_CHARS: usize = 2000;

/// The offset of Asia/Dubai from UTC, which has no daylight saving time.
const DUBAI_HOURS: i64 = 4;

/// The `manual_upload` setting (default on): when off, every upload is
/// refused and the form is hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UploadSettings {
    /// Whether a reviewer may upload a list file.
    pub enabled: bool,
}

impl Default for UploadSettings {
    fn default() -> Self {
        UploadSettings { enabled: true }
    }
}

impl UploadSettings {
    /// The app's setting, or the default (on).
    pub fn of(state: &AppState) -> UploadSettings {
        state
            .provided::<UploadSettings>()
            .map(|s| *s)
            .unwrap_or_default()
    }
}

/// What became of an upload.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum UploadOutcome {
    /// The file became the list's current version.
    #[default]
    Accepted,
    /// The file was refused; the list is unchanged.
    Refused,
}

/// One upload of a list file, accepted or refused.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_parties_sanctions_list_uploads", index(list_id))]
pub struct SanctionsListUpload {
    /// The primary key.
    pub id: i64,
    /// The list it was uploaded for.
    pub list_id: i64,
    /// The user who uploaded it.
    pub uploaded_by: i64,
    /// When it was uploaded.
    pub uploaded_at: DateTime,
    /// The source URL as given; `None` when it was missing.
    pub source_url: Option<String>,
    /// The list date as given (`YYYY-MM-DD`); `None` when it was missing.
    pub list_date: Option<String>,
    /// The file's name.
    pub file_name: String,
    /// Where the file is on the storage disk; `None` when it was not kept.
    pub file_ref: Option<String>,
    /// The file's size in bytes.
    pub byte_size: i64,
    /// Accepted or refused.
    pub outcome: UploadOutcome,
    /// Why it was refused; `None` when accepted.
    pub refusal_reason: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl SanctionsListUpload {
    /// The uploads of `list_id`, newest first.
    pub async fn for_list<'c>(
        db: impl renox::db::Executor<'c>,
        list_id: i64,
    ) -> Result<Vec<SanctionsListUpload>> {
        SanctionsListUpload::query()
            .where_(SanctionsListUpload::LIST_ID.eq(list_id))
            .order_by_desc(SanctionsListUpload::ID)
            .get(db)
            .await
    }
}

/// Why an upload is refused. The list is left unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadRefusal {
    /// The `manual_upload` setting is off.
    Disabled,
    /// The list's format has no parser.
    NoParser,
    /// The source URL is missing.
    SourceUrlMissing,
    /// The source URL is not an http(s) URL of at most 2000 characters.
    SourceUrlInvalid,
    /// The list date is missing.
    ListDateMissing,
    /// The list date is not an ISO date (`YYYY-MM-DD`).
    ListDateInvalid,
    /// The list date is after today (Asia/Dubai).
    ListDateInFuture,
    /// The list date is before the date of the current version.
    OlderThanCurrent,
    /// The file is empty.
    Empty,
    /// The file is over [`MAX_UPLOAD_BYTES`].
    TooLarge,
    /// The file cannot be read as a list; carries the parser's reason.
    Unreadable(String),
}

impl fmt::Display for UploadRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UploadRefusal::Disabled => f.write_str("manual upload is turned off"),
            UploadRefusal::NoParser => f.write_str(
                "this list format cannot be uploaded; use Record manual check or ask for a parser",
            ),
            UploadRefusal::SourceUrlMissing => f.write_str("the source URL is required"),
            UploadRefusal::SourceUrlInvalid => f.write_str(
                "the source URL must be an http or https address of at most 2000 characters",
            ),
            UploadRefusal::ListDateMissing => f.write_str("the list date is required"),
            UploadRefusal::ListDateInvalid => {
                f.write_str("the list date must be a date written YYYY-MM-DD")
            }
            UploadRefusal::ListDateInFuture => f.write_str("the list date is in the future"),
            UploadRefusal::OlderThanCurrent => f.write_str("older than the current version"),
            UploadRefusal::Empty => f.write_str("the file is empty"),
            UploadRefusal::TooLarge => f.write_str("the file is larger than 20 MB"),
            UploadRefusal::Unreadable(why) => write!(f, "the file cannot be read: {why}"),
        }
    }
}

impl std::error::Error for UploadRefusal {}

/// An upload as given, to be checked.
#[derive(Debug, Clone, Copy)]
pub struct UploadInput<'a> {
    /// The format of the list it is for.
    pub format: ListFormat,
    /// The version of the list now (its date, when it has one).
    pub current_version: Option<&'a str>,
    /// The source URL as given.
    pub source_url: &'a str,
    /// The list date as given.
    pub list_date: &'a str,
    /// The file.
    pub bytes: &'a [u8],
}

/// An upload that passed every check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidUpload {
    /// The trimmed source URL.
    pub source_url: String,
    /// The list date, which becomes the version (`YYYY-MM-DD`).
    pub list_date: NaiveDate,
    /// Whether the list date equals the current version: a successful update
    /// that is not a new version (no event, no re-screening).
    pub same_version: bool,
    /// What the parser found.
    pub parsed: ParsedList,
}

/// Today's date in Asia/Dubai at `now`.
pub fn today_dubai(now: DateTime) -> NaiveDate {
    (now + Duration::hours(DUBAI_HOURS)).date_naive()
}

/// The date of a list version: its first ten characters as an ISO date;
/// `None` for a version that is not dated.
pub fn version_date(version: &str) -> Option<NaiveDate> {
    parse_iso_date(version.get(..10)?)
}

fn parse_iso_date(text: &str) -> Option<NaiveDate> {
    if text.len() != 10 {
        return None;
    }
    NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()
}

/// Checks an upload against `today` (Asia/Dubai): the source URL, the list
/// date (ISO, not in the future, not older than the current version), the
/// size and then the file through the parser of its format. The first
/// failure is returned. The `manual_upload` setting and the list being stale
/// are checked by the caller.
pub fn validate(
    input: UploadInput<'_>,
    parsers: &ListParsers,
    today: NaiveDate,
) -> Result<ValidUpload, UploadRefusal> {
    let parser = parsers.get(input.format).ok_or(UploadRefusal::NoParser)?;
    let url = input.source_url.trim();
    if url.is_empty() {
        return Err(UploadRefusal::SourceUrlMissing);
    }
    let has_host = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .is_some_and(|rest| !rest.is_empty() && !rest.starts_with('/'));
    if !has_host
        || url.chars().count() > MAX_SOURCE_URL_CHARS
        || url.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(UploadRefusal::SourceUrlInvalid);
    }
    let date = input.list_date.trim();
    if date.is_empty() {
        return Err(UploadRefusal::ListDateMissing);
    }
    let list_date = parse_iso_date(date).ok_or(UploadRefusal::ListDateInvalid)?;
    if list_date > today {
        return Err(UploadRefusal::ListDateInFuture);
    }
    let mut same_version = false;
    if let Some(current) = input.current_version.and_then(version_date) {
        if list_date < current {
            return Err(UploadRefusal::OlderThanCurrent);
        }
        same_version = list_date == current;
    }
    if input.bytes.is_empty() {
        return Err(UploadRefusal::Empty);
    }
    if input.bytes.len() > MAX_UPLOAD_BYTES {
        return Err(UploadRefusal::TooLarge);
    }
    let parsed = parser
        .parse(input.bytes)
        .map_err(|e| UploadRefusal::Unreadable(e.0))?;
    Ok(ValidUpload {
        source_url: url.to_owned(),
        list_date,
        same_version,
        parsed,
    })
}

/// An upload as the back office gives it.
#[derive(Debug)]
pub struct UploadRequest {
    /// The official file.
    pub file: Upload,
    /// The page of the authority the file was taken from.
    pub source_url: String,
    /// The date of the list (`YYYY-MM-DD`).
    pub list_date: String,
}

/// What an accepted upload did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadResult {
    /// The file is a new version; `SanctionsListUpdated` was emitted.
    NewVersion(String),
    /// The list date is the version already held: the list is refreshed and
    /// current again, with no event and no re-screening.
    SameVersion,
}

/// Why `upload_list_file` did not take the file.
#[derive(Debug)]
pub enum UploadError {
    /// The user lacks `screening.review`; nothing is logged.
    Forbidden,
    /// The list does not exist; nothing is logged.
    NotFound,
    /// The list is current, so no upload is needed (logged as refused).
    NotStale,
    /// The file or its details were refused (logged as refused).
    Refused(UploadRefusal),
    /// The database or the storage failed.
    Failed(renox::Error),
}

impl fmt::Display for UploadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UploadError::Forbidden => f.write_str("not allowed"),
            UploadError::NotFound => f.write_str("no such list"),
            UploadError::NotStale => f.write_str("the list is current; no upload needed"),
            UploadError::Refused(why) => why.fmt(f),
            UploadError::Failed(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for UploadError {}

impl From<renox::Error> for UploadError {
    fn from(e: renox::Error) -> Self {
        UploadError::Failed(e)
    }
}

/// Where the files of list `list_id` are kept (private).
pub fn storage_dir(list_id: i64) -> String {
    format!("parties/lists/{list_id}/uploads")
}

/// `user` uploads the official file of list `list_id` by hand. Needs
/// `screening.review`. Only a `stale` list takes an upload, and only while
/// the `manual_upload` setting is on. In one transaction it locks the list,
/// checks the details and the file ([`validate`]), stores the file privately,
/// makes the list current with source *manual upload* (version = the list
/// date, uploader, source URL, `stale_since` and `last_error` cleared) and
/// logs an accepted upload. A list date equal to the version held refreshes
/// the list without a new version. After the commit a new version emits
/// `SanctionsListUpdated`, which starts the same re-run as a download.
///
/// A refused upload (setting off, list current, bad details or file) changes
/// nothing on the list and is logged with its reason, so the audit shows who
/// tried what.
///
/// # Errors
///
/// See [`UploadError`].
pub async fn upload_list_file(
    state: &AppState,
    user: &AuthUser,
    list_id: i64,
    request: UploadRequest,
) -> Result<UploadResult, UploadError> {
    if !user.has_permission(permissions::SCREENING_REVIEW) {
        return Err(UploadError::Forbidden);
    }
    let mut tx = state
        .db
        .begin_immediate()
        .await
        .map_err(renox::Error::from)?;
    let Some(mut list) = SanctionsList::query()
        .where_eq("id", list_id)
        .lock_for_update()
        .first(&mut tx)
        .await?
    else {
        return Err(UploadError::NotFound);
    };
    let now = renox::db::now();
    let valid = match check(state, &list, &request, now) {
        Ok(valid) => valid,
        Err(error) => {
            drop(tx);
            log_refusal(state, user, list_id, &request, &error.to_string(), now).await?;
            return Err(error);
        }
    };
    let key = request
        .file
        .store(&state.storage, &storage_dir(list.id))
        .await?;
    let applied = apply(&mut tx, user, &mut list, &request, &valid, &key, now).await;
    let committed = match applied {
        Ok(()) => tx.commit().await.map_err(renox::Error::from),
        Err(e) => Err(e),
    };
    if let Err(e) = committed {
        // Do not leave the file of an upload that was not recorded.
        let _ = state.storage.delete(&key).await;
        return Err(e.into());
    }
    if valid.same_version {
        return Ok(UploadResult::SameVersion);
    }
    let version = list.version.clone().unwrap_or_default();
    state
        .emit(SanctionsListUpdated {
            list: list.id,
            name: list.name.clone(),
            version: version.clone(),
        })
        .await?;
    Ok(UploadResult::NewVersion(version))
}

/// The checks that need the list: the setting, staleness, then [`validate`].
fn check(
    state: &AppState,
    list: &SanctionsList,
    request: &UploadRequest,
    now: DateTime,
) -> Result<ValidUpload, UploadError> {
    if !UploadSettings::of(state).enabled {
        return Err(UploadError::Refused(UploadRefusal::Disabled));
    }
    if list.status != ListStatus::Stale {
        return Err(UploadError::NotStale);
    }
    let parsers = state
        .provided::<ListParsers>()
        .map(|p| (*p).clone())
        .unwrap_or_default();
    validate(
        UploadInput {
            format: list.format,
            current_version: list.version.as_deref(),
            source_url: &request.source_url,
            list_date: &request.list_date,
            bytes: request.file.bytes(),
        },
        &parsers,
        today_dubai(now),
    )
    .map_err(UploadError::Refused)
}

/// Writes the list and the accepted upload row in `tx`.
async fn apply(
    tx: &mut renox::db::Transaction,
    user: &AuthUser,
    list: &mut SanctionsList,
    request: &UploadRequest,
    valid: &ValidUpload,
    key: &str,
    now: DateTime,
) -> Result<()> {
    let date = valid.list_date.format("%Y-%m-%d").to_string();
    if !valid.same_version {
        list.version = Some(date.clone());
    }
    list.last_updated_at = Some(now);
    list.update_source = Some(UpdateSource::Manual);
    list.updated_by = Some(user.id);
    list.source_url = Some(valid.source_url.clone());
    list.last_error = None;
    list.stale_since = None;
    list.status = ListStatus::Current;
    list.updated_at = Some(now);
    list.save(&mut *tx).await?;
    let mut row = SanctionsListUpload {
        list_id: list.id,
        uploaded_by: user.id,
        uploaded_at: now,
        source_url: Some(valid.source_url.clone()),
        list_date: Some(date),
        file_name: request.file.file_name().trim().to_owned(),
        file_ref: Some(key.to_owned()),
        byte_size: i64::try_from(request.file.size()).unwrap_or(i64::MAX),
        outcome: UploadOutcome::Accepted,
        ..SanctionsListUpload::default()
    };
    row.insert(&mut *tx).await?;
    Ok(())
}

/// Logs a refused upload; the file is not kept.
async fn log_refusal(
    state: &AppState,
    user: &AuthUser,
    list_id: i64,
    request: &UploadRequest,
    reason: &str,
    now: DateTime,
) -> Result<()> {
    let given = |text: &str| Some(text.trim().to_owned()).filter(|t| !t.is_empty());
    let mut row = SanctionsListUpload {
        list_id,
        uploaded_by: user.id,
        uploaded_at: now,
        source_url: given(&request.source_url),
        list_date: given(&request.list_date),
        file_name: request.file.file_name().trim().to_owned(),
        file_ref: None,
        byte_size: i64::try_from(request.file.size()).unwrap_or(i64::MAX),
        outcome: UploadOutcome::Refused,
        refusal_reason: Some(reason.to_owned()),
        ..SanctionsListUpload::default()
    };
    row.insert(&state.db).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &[u8] = b"<list><entry><name>A</name></entry></list>";

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn input<'a>(date: &'a str, version: Option<&'a str>) -> UploadInput<'a> {
        UploadInput {
            format: ListFormat::Xml,
            current_version: version,
            source_url: "https://example.org/list",
            list_date: date,
            bytes: XML,
        }
    }

    fn check(i: UploadInput<'_>) -> Result<ValidUpload, UploadRefusal> {
        validate(i, &ListParsers::default(), day(2026, 11, 5))
    }

    #[test]
    fn a_valid_upload_passes_with_its_entries() {
        let ok = check(input("2026-11-03", Some("2026-11-02"))).unwrap();
        assert_eq!(ok.list_date, day(2026, 11, 3));
        assert!(!ok.same_version);
        assert_eq!(ok.parsed.entry_count, 1);
        assert_eq!(ok.source_url, "https://example.org/list");
    }

    #[test]
    fn a_list_without_a_dated_version_takes_any_past_date() {
        assert!(check(input("2026-01-01", None)).is_ok());
        assert!(check(input("2026-01-01", Some("v17"))).is_ok());
    }

    #[test]
    fn an_older_date_is_refused_and_the_same_date_is_the_same_version() {
        let err = check(input("2026-11-01", Some("2026-11-02"))).unwrap_err();
        assert_eq!(err, UploadRefusal::OlderThanCurrent);
        assert_eq!(err.to_string(), "older than the current version");
        let same = check(input("2026-11-02", Some("2026-11-02"))).unwrap();
        assert!(same.same_version);
    }

    #[test]
    fn a_future_date_is_refused_but_today_is_not() {
        assert_eq!(
            check(input("2026-11-06", None)).unwrap_err(),
            UploadRefusal::ListDateInFuture
        );
        assert!(check(input("2026-11-05", None)).is_ok());
    }

    #[test]
    fn a_missing_or_malformed_date_is_refused() {
        assert_eq!(
            check(input("  ", None)).unwrap_err(),
            UploadRefusal::ListDateMissing
        );
        for bad in ["03/11/2026", "2026-1-3", "2026-13-01", "2026-02-30", "x"] {
            assert_eq!(
                check(input(bad, None)).unwrap_err(),
                UploadRefusal::ListDateInvalid,
                "{bad}"
            );
        }
    }

    #[test]
    fn the_source_url_is_required_and_http() {
        let mut i = input("2026-11-03", None);
        i.source_url = " ";
        assert_eq!(check(i).unwrap_err(), UploadRefusal::SourceUrlMissing);
        for bad in [
            "ftp://x.org/a",
            "example.org",
            "https://",
            "https:///a",
            "https://a b",
        ] {
            i.source_url = bad;
            assert_eq!(
                check(i).unwrap_err(),
                UploadRefusal::SourceUrlInvalid,
                "{bad}"
            );
        }
        let long = format!("https://example.org/{}", "a".repeat(MAX_SOURCE_URL_CHARS));
        i.source_url = &long;
        assert_eq!(check(i).unwrap_err(), UploadRefusal::SourceUrlInvalid);
        let edge = format!("https://{}", "a".repeat(MAX_SOURCE_URL_CHARS - 8));
        i.source_url = &edge;
        assert!(check(i).is_ok());
        i.source_url = "  http://example.org/x  ";
        assert_eq!(check(i).unwrap().source_url, "http://example.org/x");
    }

    #[test]
    fn a_format_without_a_parser_is_refused() {
        for format in [ListFormat::Pdf, ListFormat::Excel] {
            let mut i = input("2026-11-03", None);
            i.format = format;
            let err = check(i).unwrap_err();
            assert_eq!(err, UploadRefusal::NoParser);
            assert!(err.to_string().contains("Record manual check"));
        }
    }

    #[test]
    fn the_size_is_limited_to_20_mb() {
        let mut i = input("2026-11-03", None);
        i.bytes = b"";
        assert_eq!(check(i).unwrap_err(), UploadRefusal::Empty);
        let big = vec![b' '; MAX_UPLOAD_BYTES + 1];
        i.bytes = &big;
        assert_eq!(check(i).unwrap_err(), UploadRefusal::TooLarge);
        let edge = vec![b' '; MAX_UPLOAD_BYTES];
        i.bytes = &edge;
        // At the limit it is read, and is not a list.
        assert!(matches!(
            check(i).unwrap_err(),
            UploadRefusal::Unreadable(_)
        ));
    }

    #[test]
    fn an_unreadable_file_is_refused_with_the_reason() {
        let mut i = input("2026-11-03", None);
        i.bytes = b"<list><entry></list>";
        let err = check(i).unwrap_err();
        assert!(err.to_string().starts_with("the file cannot be read: "));
        i.bytes = b"<list/>";
        assert_eq!(
            check(i).unwrap_err(),
            UploadRefusal::Unreadable("the file has no entries".into())
        );
    }

    #[test]
    fn today_is_the_date_in_dubai() {
        let at = |h: u32| {
            renox::chrono::NaiveDate::from_ymd_opt(2026, 11, 5)
                .unwrap()
                .and_hms_opt(h, 0, 0)
                .unwrap()
                .and_utc()
        };
        assert_eq!(today_dubai(at(19)), day(2026, 11, 5));
        assert_eq!(today_dubai(at(20)), day(2026, 11, 6));
    }

    #[test]
    fn the_version_date_is_read_from_a_dated_version() {
        assert_eq!(version_date("2026-11-03"), Some(day(2026, 11, 3)));
        assert_eq!(version_date("2026-11-03T10:00:00Z"), Some(day(2026, 11, 3)));
        assert_eq!(version_date("v2026"), None);
        assert_eq!(version_date(""), None);
    }

    #[test]
    fn upload_is_on_unless_the_app_turns_it_off() {
        assert!(UploadSettings::default().enabled);
    }
}
