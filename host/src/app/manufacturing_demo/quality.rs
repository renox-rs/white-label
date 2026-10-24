//! Quality and compliance in the demo company: the conformity flags and
//! certificates, the NCRs, and the supplier recall traced to the work orders
//! and customers that got the paper.

use manufacturing::conformity::{ECAS, EQM, FOOD_CONTACT};
use manufacturing::{Ncr, WorkOrder};
use renox::prelude::*;
use wl_documents::{Document, DocumentKind, DocumentStatus};
use wl_kernel::Subject;

use super::{MANAGER_EMAIL, OWNER_EMAIL, QC_EMAIL, Seed, day};

impl Seed {
    async fn document(
        &self,
        subject: Subject,
        kind: &str,
        title: &str,
        number: &str,
        expires: i64,
    ) -> Result {
        let kind = DocumentKind::by_key(self.db(), kind)
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the document kind {kind}"))?;
        Document::create(
            self.db(),
            Document {
                subject_type: subject.subject_type,
                subject_id: subject.subject_id,
                kind_id: kind.id,
                title: title.to_owned(),
                number: Some(number.to_owned()),
                issue_date: Some(day(-300)),
                expiry_date: Some(day(expires)),
                issuing_authority: Some("Certification body".to_owned()),
                current_version: 1,
                status: DocumentStatus::Accepted,
                ..Default::default()
            },
        )
        .await?;
        Ok(())
    }

    /// The flags of the items (production manager) and the certificates and
    /// licences on file: the food-contact certificate of the food box is valid,
    /// the quality mark licence of the retail box expires in 20 days (the look-ahead
    /// lists the orders promised after it), the ICV certificate in 24.
    pub(crate) async fn conformity(&mut self) -> Result {
        for (code, form) in [
            ("BOX-FOOD", vec![("regulated", "1"), ("kind", FOOD_CONTACT)]),
            ("BOX-RTL", vec![("regulated", "1"), ("kind", EQM)]),
            ("SLV-EPH", vec![("customer_specific", "1")]),
            ("BOX-EXP", vec![("export_only", "1")]),
        ] {
            let mut fields = vec![("code", code)];
            fields.extend(form);
            self.driver
                .post(MANAGER_EMAIL, "/manufacturing/conformity", &fields)
                .await?;
        }
        let item = |code: &str| Subject::new("inventory.item", self.items[code]);
        self.document(
            item("BOX-FOOD"),
            FOOD_CONTACT,
            "Food-contact certificate, food boxes",
            "FC-2025-0118",
            200,
        )
        .await?;
        self.document(
            item("BOX-RTL"),
            EQM,
            "Emirates Quality Mark licence, retail boxes",
            "EQM-55021",
            20,
        )
        .await?;
        let _ = ECAS;
        let company = Subject::new("kernel.legal_entity", self.entity);
        self.document(
            company.clone(),
            "industrial_licence",
            "Industrial licence",
            "IL-2019-4471",
            250,
        )
        .await?;
        self.document(
            company.clone(),
            "icv_certificate",
            "ICV certificate",
            "ICV-88-2025",
            24,
        )
        .await?;
        self.document(
            company.clone(),
            "iso_certificate",
            "ISO 9001:2015",
            "ISO-AE-9001-3321",
            150,
        )
        .await?;
        self.document(
            company,
            "trademark_registration",
            "Emirates Pack mark",
            "TM-UAE-312200",
            75,
        )
        .await?;
        Ok(())
    }

    /// The NCRs: liner paper that failed incoming inspection (supplier), a batch of
    /// boxes with delaminated liner (internal process) already scrapped and
    /// closed; and the supplier recall of one liner lot.
    pub(crate) async fn quality(&mut self) -> Result {
        let plant = self.plant.to_string();
        self.driver
            .post(
                QC_EMAIL,
                "/manufacturing/ncrs",
                &[
                    ("plant_id", &plant),
                    ("lot_id", &self.lots["KL-2318"].to_string()),
                    ("quantity", "800"),
                    (
                        "defect",
                        "Moisture above specification at incoming inspection",
                    ),
                    ("cause", "supplier"),
                ],
            )
            .await?;
        let output = WorkOrder::find_or_404(self.db(), self.work_orders["noor_box"])
            .await?
            .output_lot_id;
        if let Some(lot) = output {
            self.driver
                .post(
                    QC_EMAIL,
                    "/manufacturing/ncrs",
                    &[
                        ("plant_id", &plant),
                        ("lot_id", &lot.to_string()),
                        ("quantity", "120"),
                        ("defect", "Delamination of the liner on the box edge"),
                        ("cause", "internal_process"),
                    ],
                )
                .await?;
            let ncr = Ncr::query()
                .order_by_desc(Ncr::ID)
                .first(self.db())
                .await?
                .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the NCR"))?;
            self.driver
                .post(
                    QC_EMAIL,
                    &format!("/manufacturing/ncrs/{}/disposition", ncr.id),
                    &[("disposition", "scrap"), ("note", "Pulped and recycled")],
                )
                .await?;
        }
        // The supplier recalls liner lot KL-2291.
        self.driver
            .post(
                OWNER_EMAIL,
                "/recalls",
                &[
                    ("source", "supplier"),
                    (
                        "reason",
                        "The paper mill found contamination in lot 77412 of kraft liner.",
                    ),
                    ("items", &self.items["KRAFT-LINER"].to_string()),
                    ("lot_numbers", "KL-2291"),
                ],
            )
            .await?;
        let recall = wl_inventory::Recall::query()
            .order_by_desc(wl_inventory::Recall::ID)
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the recall was not opened"))?;
        self.driver
            .post(OWNER_EMAIL, &format!("/recalls/{}/trace", recall.id), &[])
            .await?;
        Ok(())
    }
}
