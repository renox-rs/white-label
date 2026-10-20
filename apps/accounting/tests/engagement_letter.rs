//! Acceptance checks of #326 (BL-ACC-02, `ACC.RUL.11`): an engagement letter
//! sent for acceptance and not signed is chased (the client approver at 7
//! and 14 days, the manager's task at 30 days), a signature stops it, a
//! re-sent version restarts the count, and the manager's decline disables
//! the client's portal users and tells the client with a neutral template.

mod support;

use accounting::client_notices::{ClientNotice, DECLINED_NOTICE, NeutralTemplate, UNABLE_NOTICE};
use accounting::letters::{EngagementLetter, LETTER_REMINDER, run};
use accounting::{Engagement, EngagementStatus};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::history;
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::notify::{self, NotifyChannel};
use wl_kernel::tickets::Assignee;

use support::{Booted, Client, boot, d};

/// The prefix of the manager's chase tasks.
const KEY: &str = "accounting.letter_chase:";

/// An `onboarding` engagement of a fresh client run by `manager`, its
/// letter sent on `sent` through the route, and the client's approver in the
/// portal.
async fn sent_on(t: &Booted, manager: &User, sent: NaiveDate) -> (Client, Engagement, User) {
    let client = t.client().await;
    let engagement = t.engagement(&client).await;
    Engagement::query()
        .where_(Engagement::ID.eq(engagement.id))
        .update(t.db(), &[("manager_user_id", &Some(manager.id))])
        .await
        .unwrap();
    let approver = t.portal_user("Omar", client.ubo, client.company).await;
    send_letter(t, manager, &engagement, sent).await;
    (client, engagement, approver)
}

/// `user` sends the letter of `engagement` through the route on `day`.
async fn send_letter(t: &Booted, user: &User, engagement: &Engagement, day: NaiveDate) {
    t.set_clock_on(day, 10, 0);
    t.act(user, engagement, "letter/send", &[])
        .await
        .assert_ok();
}

/// The rule of `day`.
async fn run_on(t: &Booted, day: NaiveDate) -> accounting::letters::LetterRun {
    t.set_clock_on(day, 9, 0);
    t.run(run(t.state(), day)).await.unwrap()
}

/// DW1: a letter sent 2026-11-01 and not signed: the client approver is
/// reminded on 2026-11-08 and 2026-11-15, the manager gets one task on
/// 2026-12-01, and nothing declines the engagement.
#[renox::test]
async fn an_unsigned_letter_is_chased_at_7_14_and_30_days() {
    let t = boot().await;
    let manager = t.manager().await;
    let (client, engagement, _approver) = sent_on(&t, &manager, d(11, 1)).await;

    assert!(run_on(&t, d(11, 7)).await.reminded.is_empty(), "day 6");
    let day7 = run_on(&t, d(11, 8)).await;
    assert_eq!(day7.reminded.len(), 1);
    assert_eq!(day7.reminded[0].1, 7);
    let again = run_on(&t, d(11, 8)).await;
    assert!(again.reminded.is_empty(), "once per day");
    assert!(run_on(&t, d(11, 14)).await.reminded.is_empty(), "day 13");
    let day14 = run_on(&t, d(11, 15)).await;
    assert_eq!(day14.reminded[0].1, 14);

    let reminders = t.notices.of_type(LETTER_REMINDER);
    assert_eq!(reminders.len(), 2);
    for reminder in &reminders {
        assert!(
            reminder
                .recipients
                .contains(&NotifyRecipient::Party { id: client.ubo }),
            "the client approver is reminded"
        );
    }

    assert!(t.tickets.with_key(KEY).is_empty(), "no task before day 30");
    assert!(run_on(&t, d(11, 30)).await.chased.is_empty(), "day 29");
    let day30 = run_on(&t, d(12, 1)).await;
    assert_eq!(day30.chased.len(), 1);
    assert!(
        run_on(&t, d(12, 2)).await.chased.is_empty(),
        "one task only"
    );
    let tasks = t.tickets.with_key(KEY);
    assert_eq!(tasks.len(), 1, "{tasks:?}");
    assert!(tasks[0].open);
    assert_eq!(tasks[0].assignee, Some(Assignee::user(manager.id)));
    assert_eq!(tasks[0].subject, engagement.subject());
    assert!(tasks[0].title.contains(&engagement.number()));
    assert_eq!(t.notices.of_type(LETTER_REMINDER).len(), 2);
    assert_eq!(
        t.status(&engagement).await,
        EngagementStatus::Onboarding,
        "never declined automatically"
    );
}

/// DW2: the client approver signs in the portal on 2026-11-10: no reminder
/// on 2026-11-15, no task on 2026-12-01; the signature keeps the signer and
/// the document hash.
#[renox::test]
async fn a_signed_letter_stops_the_reminders_and_the_task() {
    let t = boot().await;
    let manager = t.manager().await;
    let (_client, engagement, approver) = sent_on(&t, &manager, d(11, 1)).await;
    assert_eq!(run_on(&t, d(11, 8)).await.reminded.len(), 1);

    // Staff cannot sign for the client; the client approver can.
    t.post(
        &manager,
        &format!(
            "/accounting/portal/engagements/{}/letter/sign",
            engagement.id
        ),
        &[],
    )
    .await
    .assert_forbidden();
    t.set_clock_on(d(11, 10), 11, 0);
    t.get(
        &approver,
        &format!("/accounting/portal/engagements/{}/letter", engagement.id),
    )
    .await
    .assert_ok()
    .assert_see("Waiting for a signature");
    t.post(
        &approver,
        &format!(
            "/accounting/portal/engagements/{}/letter/sign",
            engagement.id
        ),
        &[],
    )
    .await
    .assert_ok();

    let signed = t.reload(&engagement).await;
    assert!(signed.letter_signed);
    assert_eq!(signed.letter_signed_on, Some(d(11, 10)));
    let letters = EngagementLetter::of_engagement(t.db(), engagement.id)
        .await
        .unwrap();
    assert_eq!(letters.len(), 1);
    assert!(letters[0].is_signed());
    assert_eq!(letters[0].signer_user_id, Some(approver.id));
    assert_eq!(letters[0].signer_name.as_deref(), Some("Omar"));
    assert_eq!(letters[0].document_hash.len(), 16);

    assert!(run_on(&t, d(11, 15)).await.reminded.is_empty());
    assert!(run_on(&t, d(12, 1)).await.chased.is_empty());
    assert_eq!(t.notices.of_type(LETTER_REMINDER).len(), 1);
    assert!(t.tickets.with_key(KEY).is_empty());
    // A second signature has nothing to sign.
    t.post(
        &approver,
        &format!(
            "/accounting/portal/engagements/{}/letter/sign",
            engagement.id
        ),
        &[],
    )
    .await
    .assert_status(422);
}

/// An open task closes when the letter is signed, and a re-sent version
/// restarts the count: the old version's task closes, its reminders stop,
/// and the new version is chased from its own sent date.
#[renox::test]
async fn a_signature_closes_the_task_and_a_new_version_restarts_the_count() {
    let t = boot().await;
    let manager = t.manager().await;
    let (_client, engagement, approver) = sent_on(&t, &manager, d(11, 1)).await;

    run_on(&t, d(12, 1)).await;
    assert!(t.tickets.with_key(KEY)[0].open);

    // A new version on 2026-12-02: the first task closes, the count restarts.
    send_letter(&t, &manager, &engagement, d(12, 2)).await;
    let tasks = t.tickets.with_key(KEY);
    assert!(!tasks[0].open, "the task of version 1 is closed");
    let letters = EngagementLetter::of_engagement(t.db(), engagement.id)
        .await
        .unwrap();
    assert_eq!(
        letters.iter().map(|l| l.version).collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(letters[0].superseded_on, Some(d(12, 2)));
    assert_ne!(letters[0].document_hash, letters[1].document_hash);

    let before = t.notices.of_type(LETTER_REMINDER).len();
    assert!(
        run_on(&t, d(12, 8)).await.reminded.is_empty(),
        "day 6 of version 2"
    );
    assert_eq!(
        run_on(&t, d(12, 9)).await.reminded.len(),
        1,
        "day 7 of version 2"
    );
    assert_eq!(t.notices.of_type(LETTER_REMINDER).len(), before + 1);
    assert!(
        run_on(&t, d(12, 31)).await.chased.is_empty(),
        "day 29 of version 2"
    );
    let chased = run_on(&t, NaiveDate::from_ymd_opt(2027, 1, 1).unwrap()).await;
    assert_eq!(chased.chased.len(), 1, "day 30 of version 2");
    assert_eq!(t.tickets.with_key(KEY).len(), 2);
    assert!(t.tickets.with_key(KEY)[1].open);

    // The signature closes the open task.
    t.set_clock_on(NaiveDate::from_ymd_opt(2027, 1, 2).unwrap(), 10, 0);
    t.post(
        &approver,
        &format!(
            "/accounting/portal/engagements/{}/letter/sign",
            engagement.id
        ),
        &[],
    )
    .await
    .assert_ok();
    assert!(!t.tickets.with_key(KEY)[1].open);
}

/// DW3: the manager declines with *letter not signed* and the neutral
/// template: the engagement is `declined`, the client company's portal users
/// lose their grants for it (a grant for another company stays), and the
/// client receives the chosen template, which says nothing of the reason.
#[renox::test]
async fn declining_for_an_unsigned_letter_disables_the_users_and_tells_the_client() {
    let t = boot().await;
    let manager = t.manager().await;
    let (client, engagement, approver) = sent_on(&t, &manager, d(11, 1)).await;
    // The same user also acts for another company of the group.
    let other = t.client_named(" Group").await;
    wl_portal::PortalAccess::register(
        t.state(),
        approver.id,
        wl_portal::NewGrant::new(client.ubo, other.company, "client", &["owner"]),
    )
    .await
    .unwrap();
    run_on(&t, d(12, 1)).await;

    t.set_clock_on(d(12, 1), 11, 0);
    // The wrong template for the reason is refused; no template tells nothing.
    t.act(
        &manager,
        &engagement,
        "decline",
        &[("reason", "letter_not_signed"), ("template", "unable")],
    )
    .await
    .assert_status(422);
    t.act(
        &manager,
        &engagement,
        "decline",
        &[("reason", "letter_not_signed"), ("template", "declined")],
    )
    .await
    .assert_ok();

    let declined = t.reload(&engagement).await;
    assert_eq!(declined.status, EngagementStatus::Declined);
    assert_eq!(declined.reason_code.as_deref(), Some("letter_not_signed"));

    // The portal users of the client company lost their access.
    t.get(
        &approver,
        &format!("/accounting/portal/engagements/{}/letter", engagement.id),
    )
    .await
    .assert_not_found();
    let today = d(12, 1);
    assert!(
        wl_portal::PortalAccess::active_grant(t.db(), approver.id, client.company, today)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        wl_portal::PortalAccess::active_grant(t.db(), approver.id, other.company, today)
            .await
            .unwrap()
            .is_some(),
        "the grant for the other company stays"
    );

    // The chosen template, recorded and sent to the client company.
    let notices = ClientNotice::of_engagement(t.db(), engagement.id)
        .await
        .unwrap();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].template, NeutralTemplate::Declined);
    let sent = t.notices.of_type(DECLINED_NOTICE);
    assert_eq!(sent.len(), 1);
    assert_eq!(t.notices.to_party(client.company).len(), 1);
    assert!(t.notices.of_type(UNABLE_NOTICE).is_empty());
    // The chase task is the manager's to close; the letter shows replaced
    // or waiting, never signed.
    assert!(!t.reload(&engagement).await.letter_signed);
}

/// *Other* needs a text; the text is kept.
#[renox::test]
async fn declining_for_another_reason_needs_a_text() {
    let t = boot().await;
    let manager = t.manager().await;
    let client = t.client().await;
    let engagement = t.engagement(&client).await;
    t.act(&manager, &engagement, "decline", &[("reason", "other")])
        .await
        .assert_status(422);
    t.act(
        &manager,
        &engagement,
        "decline",
        &[("reason", "other"), ("note", "Moved abroad")],
    )
    .await
    .assert_ok();
    let declined = t.reload(&engagement).await;
    assert_eq!(declined.status, EngagementStatus::Declined);
    assert_eq!(declined.decline_note.as_deref(), Some("Moved abroad"));
}

/// DW4: a decline with *CDD rejected*: a partner without the MLRO
/// permission sees "Declined" and no reason, on the page and in the list;
/// the template the client gets never mentions due diligence.
#[renox::test]
async fn a_cdd_rejected_decline_shows_only_declined_to_the_partner() {
    let t = boot().await;
    let manager = t.manager().await;
    let partner = t.partner().await;
    let client = t.client().await;
    let engagement = t.engagement(&client).await;

    t.act(
        &manager,
        &engagement,
        "decline",
        &[("reason", "not_accepted"), ("template", "unable")],
    )
    .await
    .assert_ok();
    let declined = t.reload(&engagement).await;
    assert_eq!(declined.status, EngagementStatus::Declined);
    assert!(!declined.may_message_client(), "the reason is restricted");

    for path in [
        format!("/accounting/engagements/{}", engagement.id),
        "/accounting/engagements".to_owned(),
        "/accounting/engagements/export".to_owned(),
    ] {
        let page = t.get(&partner, &path).await;
        page.assert_ok().assert_see("Declined");
        let text = page.text();
        for hidden in [
            "CDD rejected",
            "Not accepted",
            "not_accepted",
            "due diligence",
        ] {
            assert!(!text.contains(hidden), "{path} shows `{hidden}`");
        }
    }

    let sent = t.notices.of_type(UNABLE_NOTICE);
    assert_eq!(sent.len(), 1);
    let body = accounting::client_notices::NeutralTemplate::Unable.as_str();
    assert_eq!(body, "unable");
    assert!(t.notices.of_type(DECLINED_NOTICE).is_empty());

    // The template the client reads, rendered in both languages, never says
    // why: no due diligence, CDD, rejection or screening.
    let def = notify::notification_type(UNABLE_NOTICE).expect("registered");
    for language in ["en", "ar"] {
        let template = def
            .template_for(NotifyChannel::Email, language)
            .unwrap_or_else(|| panic!("a {language} template"));
        let rendered = format!(
            "{} {}",
            notify::fill(
                template.subject.as_deref().unwrap_or_default(),
                &sent[0].data
            ),
            notify::fill(&template.body, &sent[0].data)
        );
        assert!(!rendered.trim().is_empty(), "{language}");
        for hidden in ["due diligence", "CDD", "reject", "KYC", "screen", "AML"] {
            assert!(
                !rendered.to_lowercase().contains(&hidden.to_lowercase()),
                "the {language} template says `{hidden}`: {rendered}"
            );
        }
        if language == "en" {
            assert!(rendered.contains("unable to proceed"), "{rendered}");
        }
    }

    // The history of the engagement carries no more than the partner saw.
    let rows = history(t.db(), Engagement::TABLE, engagement.id)
        .await
        .unwrap();
    let shown = format!("{rows:?}");
    assert!(!shown.contains("due diligence") && !shown.contains("CDD"));

    // The MLRO and the deputy read the reason; the partner and manager do not.
    let (mlro, deputy) = (t.mlro().await, t.deputy().await);
    let page = format!("/accounting/engagements/{}", engagement.id);
    for viewer in [&mlro, &deputy] {
        t.get(viewer, &page)
            .await
            .assert_ok()
            .assert_see("CDD rejected");
    }
    for viewer in [&partner, &manager] {
        let text = t.get(viewer, &page).await.text();
        assert!(!text.contains("CDD rejected"), "{text}");
    }
}
