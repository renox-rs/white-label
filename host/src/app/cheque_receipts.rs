//! The cheque handover receipt's document kind and template (BL-PAY-30):
//! wl-payments records the handover and provides its data
//! ([`wl_payments::cheque_handover`]); the kind and the bilingual template
//! are registered with wl-documents here, because the module does not depend
//! on the documents crate. A client changes the text with
//! `DocumentTemplate::revise`.

use wl_documents::{KindSeed, PaperSize, TemplateSeed, register_kind, register_template};
use wl_kernel::generation::LanguageMode;
use wl_payments::cheque_handover::{KIND, SERIES, SIGNED_KIND, SUBJECT_TYPE, TEMPLATE};

/// The English text of the receipt.
const ENGLISH: &str = "Cheque handover receipt {{ number }}
Date: {{ data.date }}
Received from: {{ entity.name_en }}
Received by: {{ data.receiver }}
Cheques of: {{ data.party }}
Reason: {{ data.reason }}

{% for cheque in data.cheques %}{{ cheque.number }} | {{ cheque.bank }} | {{ cheque.date }} | {{ cheque.amount }}
{% endfor %}
{{ data.count }} cheque(s), total {{ data.total }}

Signature of the receiver: ______________________
Name: ______________________   Date: ____________";

/// The Arabic text of the receipt.
const ARABIC: &str = "إيصال تسليم شيكات {{ number }}
التاريخ: {{ data.date }}
المستلم: {{ data.receiver_ar }}
الشيكات باسم: {{ data.party_ar }}

{% for cheque in data.cheques %}{{ cheque.number }} | {{ cheque.bank }} | {{ cheque.date }} | {{ cheque.amount }}
{% endfor %}
عدد الشيكات {{ data.count }}، المجموع {{ data.total }}

توقيع المستلم: ______________________";

/// Registers the receipt's kinds and template (every client with documents;
/// a client without the payments module never generates one).
pub fn register() {
    register_kind(
        KindSeed::new(KIND, "Cheque handover receipt")
            .name_ar("إيصال تسليم شيكات")
            .subject_type(SUBJECT_TYPE)
            .numbered()
            .file_types(&["pdf"])
            .without_review(),
    );
    register_kind(
        KindSeed::new(SIGNED_KIND, "Cheque handover receipt signed by the receiver")
            .name_ar("إيصال تسليم شيكات موقع من المستلم")
            .subject_type(SUBJECT_TYPE),
    );
    register_template(
        TemplateSeed::new(TEMPLATE, KIND)
            .name("Cheque handover receipt")
            .mode(LanguageMode::Bilingual)
            .english(ENGLISH)
            .arabic(ARABIC)
            .paper(PaperSize::A4)
            .numbered(SERIES),
    );
}
