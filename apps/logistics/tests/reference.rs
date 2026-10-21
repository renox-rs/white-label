//! BL-LOG-01 (#299), 03 §4.1 *Duplicate booking*, §6 *Shipment* (client
//! reference): a client reference is unique per client across all states; a
//! CSV row with a used reference is refused in the dry run and in the import,
//! and the unique index refuses a second insert.

mod support;

use logistics::Shipment;
use renox::prelude::*;

use support::{OTHER_CLIENT, SOUQ, boot};

const HEADER: &str =
    "client_reference,recipient_name,recipient_phone,address,area,parcels,weight_grams,cod";

fn csv(rows: &[&str]) -> String {
    let mut text = HEADER.to_owned();
    for row in rows {
        text.push('\n');
        text.push_str(row);
    }
    text
}

async fn shipments(w: &support::World) -> Vec<Shipment> {
    Shipment::query()
        .order_by(Shipment::ID)
        .get(w.db())
        .await
        .unwrap()
}

/// AC1: given client Souq Express has shipment SHP-xxxx with reference
/// "ORD-77", a CSV row with that reference is refused with "client reference
/// already used (SHP-xxxx)" and the other rows import; two rows with one
/// reference in a file: the first imports, the second is refused.
#[renox::test]
async fn a_csv_row_with_a_used_reference_is_refused_and_the_other_rows_import() {
    let w = boot().await;
    let existing = w.book_as(&w.noor, SOUQ, "ORD-77").await;
    let file = csv(&[
        "ORD-77,Ali Hassan,+971500000001,Villa 1,Al Barsha,1,1000,0",
        "ORD-78,Sara Nasser,+971500000002,Villa 2,Jumeirah,2,800,150.50",
        "ORD-78,Sara Nasser,+971500000002,Villa 2,Jumeirah,1,800,0",
        ",Omar Khalid,+971500000003,Villa 3,Deira,1,500,0",
    ]);
    let client = SOUQ.0.to_string();

    // The dry run lists every row and books nothing.
    let dry = w
        .post(
            &w.noor,
            "/shipments/import",
            &[
                ("client_party_id", client.as_str()),
                ("csv", file.as_str()),
                ("dry_run", "1"),
            ],
        )
        .await;
    dry.assert_ok();
    dry.assert_json_path("accepted", 2);
    dry.assert_json_path("refused", 2);
    dry.assert_json_path(
        "rows.0.refused",
        format!("client reference already used ({})", existing.number),
    );
    dry.assert_json_path("rows.1.refused", renox::serde_json::Value::Null);
    dry.assert_json_path("rows.2.refused", "client reference already used (row 3)");
    dry.assert_json_path("rows.3.refused", renox::serde_json::Value::Null);
    assert_eq!(shipments(&w).await.len(), 1);

    // The import books the other rows.
    w.post(
        &w.noor,
        "/shipments/import",
        &[("client_party_id", client.as_str()), ("csv", file.as_str())],
    )
    .await
    .assert_status(303);
    let all = shipments(&w).await;
    assert_eq!(all.len(), 3);
    let second = &all[1];
    assert_eq!(second.client_reference.as_deref(), Some("ORD-78"));
    assert_eq!(second.parcel_count, 2);
    assert_eq!(second.cod_amount, 15_050);
    assert_eq!(all[2].client_reference, None);
    // The row of the used reference booked nothing new.
    assert_eq!(
        all.iter()
            .filter(|s| s.client_reference.as_deref() == Some("ORD-77"))
            .count(),
        1
    );
}

/// AC2: given another client books with reference "ORD-77", it is accepted;
/// the same client, any user of it, is refused; a booking without a
/// reference is not checked; the database refuses a second insert.
#[renox::test]
async fn the_reference_is_unique_per_client_and_the_index_backs_it() {
    let w = boot().await;
    let first = w.book_as(&w.noor, SOUQ, "ORD-77").await;
    let other = w.book_as(&w.farah, OTHER_CLIENT, "ORD-77").await;
    assert_ne!(first.id, other.id);

    let again = w.try_book(&w.nadia, SOUQ, "ORD-77", &[]).await;
    again.assert_status(422);
    again.assert_see(&format!("client reference already used ({})", first.number));

    w.book_as(&w.noor, SOUQ, "").await;
    w.book_as(&w.noor, SOUQ, "").await;

    // Two uploads at once: the second insert fails on the unique index.
    let mut raced = Shipment {
        number: "SHP-9999".to_owned(),
        client_party_id: SOUQ.0,
        client_reference: Some("ORD-77".to_owned()),
        ..Shipment::default()
    };
    let error = raced.insert(w.db()).await.unwrap_err();
    assert!(error.is_unique_violation());
}
