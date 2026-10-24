//! BL-POR-02 (#461): each audience's menu is built from the registered
//! sections and the permissions of the grant's portal roles; a section the
//! roles do not allow, or one of another audience, is a 403 even by URL; a
//! user with grants for several parties switches party; the company admin
//! changes a role's permissions.

mod support;

use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::documents::today;
use wl_portal::permissions::USERS_MANAGE;
use wl_portal::{PortalMenu, PortalRole};

use support::{ACME_ONE, ACME_TWO, SARA_PARTY, boot};

/// "Al Noor Trading LLC", the tenant company.
const AL_NOOR: i64 = ACME_ONE;

async fn menu_keys(t: &support::Booted, user: &User, party: i64) -> Vec<String> {
    let lang = t.state().lang("en");
    PortalMenu::of(t.state(), user.id, PartyId(party), today(), &lang)
        .await
        .unwrap()
        .map(|menu| menu.keys())
        .unwrap_or_default()
}

/// Done when 1: Sara M., `primary_contact` for Al Noor Trading, sees Lease,
/// Invoices, Cheques, Deposit, Documents and Requests in their order, on the
/// portal home too.
#[renox::test]
async fn por02_a_primary_contact_sees_every_tenant_section_in_order() {
    let t = boot().await;
    t.with_tenant_portal().await;
    let (sara, _) = t.tenant_user("Sara", AL_NOOR, &["primary_contact"]).await;
    assert_eq!(
        menu_keys(&t, &sara, AL_NOOR).await,
        ["lease", "invoices", "cheques", "deposit", "documents", "requests"]
    );
    let home = t.get_as(&sara, &format!("/portal/{AL_NOOR}")).await;
    home.assert_ok();
    let page = home.text();
    let at = |label: &str| page.find(label).unwrap_or_else(|| panic!("{label}"));
    let positions = [
        at("Lease"),
        at("Invoices"),
        at("Cheques"),
        at("Deposit"),
        at("Documents"),
        at("Requests"),
    ];
    assert!(positions.windows(2).all(|w| w[0] < w[1]), "{positions:?}");
    for key in ["lease", "invoices", "requests"] {
        t.get_as(&sara, &format!("/portal/{AL_NOOR}/{key}"))
            .await
            .assert_ok();
    }
}

/// Done when 2: Omar, `finance_contact` for the same party, sees only
/// Invoices; the requests page by URL is a 403.
#[renox::test]
async fn por02_a_finance_contact_sees_only_invoices() {
    let t = boot().await;
    t.with_tenant_portal().await;
    let (omar, _) = t.tenant_user("Omar", AL_NOOR, &["finance_contact"]).await;
    assert_eq!(menu_keys(&t, &omar, AL_NOOR).await, ["invoices"]);
    t.get_as(&omar, &format!("/portal/{AL_NOOR}/invoices"))
        .await
        .assert_ok();
    for key in ["requests", "lease", "documents"] {
        t.get_as(&omar, &format!("/portal/{AL_NOOR}/{key}"))
            .await
            .assert_forbidden();
    }
    let home = t.get_as(&omar, &format!("/portal/{AL_NOOR}")).await;
    home.assert_ok();
    assert!(!home.text().contains("Requests"));
}

/// Done when 3: a user with grants for two parties opens the first one,
/// switches to the other, and each page lists that party's rows only; a
/// party without a grant is a 403.
#[renox::test]
async fn por02_a_user_with_two_parties_switches_and_sees_each_partys_rows() {
    let t = boot().await;
    t.with_tenant_portal().await;
    let (nadia, _) = t.tenant_user("Nadia", ACME_ONE, &["finance_contact"]).await;
    t.grant(&nadia, ACME_TWO, &["finance_contact"]).await;

    let start = t.get_as(&nadia, "/portal").await;
    assert_eq!(
        start.header("location"),
        Some(format!("/portal/{ACME_ONE}").as_str())
    );
    let home = t.get_as(&nadia, &format!("/portal/{ACME_ONE}")).await;
    home.assert_ok();
    assert!(home.text().contains(&format!("/portal/{ACME_TWO}")));

    let one = t
        .get_as(&nadia, &format!("/portal/{ACME_ONE}/invoices"))
        .await;
    one.assert_ok();
    assert!(one.text().contains(&format!("Invoices of party {ACME_ONE}")));
    assert!(!one.text().contains(&format!("Invoices of party {ACME_TWO}")));
    let two = t
        .get_as(&nadia, &format!("/portal/{ACME_TWO}/invoices"))
        .await;
    two.assert_ok();
    assert!(two.text().contains(&format!("Invoices of party {ACME_TWO}")));
    assert!(!two.text().contains(&format!("Invoices of party {ACME_ONE}")));

    t.get_as(&nadia, &format!("/portal/{SARA_PARTY}/invoices"))
        .await
        .assert_forbidden();
    t.get_as(&nadia, &format!("/portal/{SARA_PARTY}"))
        .await
        .assert_forbidden();
}

/// Done when 4: a section registered for the audience `owner` never appears
/// for a tenant, even for one whose roles hold every tenant permission, and
/// its URL is a 403.
#[renox::test]
async fn por02_an_owner_section_never_appears_for_a_tenant() {
    let t = boot().await;
    t.with_tenant_portal().await;
    let (sara, _) = t
        .tenant_user("Sara", AL_NOOR, &["primary_contact", "finance_contact"])
        .await;
    assert!(
        !menu_keys(&t, &sara, AL_NOOR)
            .await
            .contains(&"statements".to_owned())
    );
    t.get_as(&sara, &format!("/portal/{AL_NOOR}/statements"))
        .await
        .assert_forbidden();
}

/// Done when 5: the company admin adds `portal.documents.view` to
/// `finance_contact` in the portal settings; Omar then sees Documents. Only
/// portal permissions are kept, and a user without `portal_users.manage`
/// cannot change a role.
#[renox::test]
async fn por02_the_company_admin_adds_documents_to_the_finance_contact() {
    let t = boot().await;
    t.with_tenant_portal().await;
    let (omar, _) = t.tenant_user("Omar", AL_NOOR, &["finance_contact"]).await;
    let admin = t.user("Aisha", &[USERS_MANAGE]).await;
    let clerk = t.user("Karim", &[]).await;

    let page = t.get_as(&admin, "/portal/roles").await;
    page.assert_ok();
    assert!(page.text().contains("finance_contact"));
    t.get_as(&clerk, "/portal/roles").await.assert_forbidden();

    let finance = PortalRole::of_audience(t.db(), "tenant")
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.key == "finance_contact")
        .unwrap();
    let path = format!("/portal/roles/{}", finance.id);
    let form = [(
        "permissions",
        "portal.invoices.view, portal.invoices.pay, portal.documents.view, invoices.view",
    )];
    t.post_as(&clerk, &path, &form).await.assert_forbidden();
    t.post_as(&admin, &path, &form).await.assert_ok();
    let changed = PortalRole::find_or_404(t.db(), finance.id).await.unwrap();
    assert_eq!(
        changed.permission_keys(),
        [
            "portal.invoices.view",
            "portal.invoices.pay",
            "portal.documents.view"
        ]
    );

    assert_eq!(
        menu_keys(&t, &omar, AL_NOOR).await,
        ["invoices", "documents"]
    );
    t.get_as(&omar, &format!("/portal/{AL_NOOR}/documents"))
        .await
        .assert_ok();
    // A re-run of the seed keeps the admin's change.
    t.with_tenant_portal().await;
    assert_eq!(
        menu_keys(&t, &omar, AL_NOOR).await,
        ["invoices", "documents"]
    );
}

/// Two grants for the same party: the union of the roles' permissions; a
/// role key with no row gives nothing.
#[renox::test]
async fn por02_two_grants_for_one_party_add_up() {
    let t = boot().await;
    t.with_tenant_portal().await;
    let (lina, _) = t.tenant_user("Lina", AL_NOOR, &["finance_contact"]).await;
    let (ghost, _) = t.tenant_user("Ghost", AL_NOOR, &["unknown_role"]).await;
    assert!(menu_keys(&t, &ghost, AL_NOOR).await.is_empty());
    t.grant(&lina, AL_NOOR, &["primary_contact"]).await;
    assert_eq!(
        menu_keys(&t, &lina, AL_NOOR).await,
        ["lease", "invoices", "cheques", "deposit", "documents", "requests"]
    );
}
