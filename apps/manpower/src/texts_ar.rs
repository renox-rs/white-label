//! The vertical's texts in Arabic, key for key with `texts.rs` (BL-MPW-33).
//!
//! `texts::t` and `texts::tf` read this table when the viewer's language is
//! Arabic (`ar`, `ar-AE`) and the host's `resources/lang/ar.json` has no text
//! of its own for the key (the host file still wins, so an agency changes a
//! word there). A key missing here falls back to English; the test in
//! `texts.rs` lists every key without an Arabic entry.
//!
//! Western digits, `AED 1,250.00` and Gregorian dates stay as the code
//! formats them; names of workers, sites and clients are filled in as
//! entered. Plurals: the code picks between a `_one` and a plural key, so the
//! Arabic plural texts use the form that reads for counts of 11 and more; the
//! dual and the 3 to 10 forms need plural keys of their own (recorded as
//! open in the issue).
//!
//! The keys in [`REVIEW`] carry legal terms (notice period, waiver, deemed
//! approval, absconding, the MoHRE absence report, credit hold) whose Arabic
//! wording the agency must confirm before launch; they are not guessed
//! further than the common UAE usage.

/// Every key of `texts::TEXTS` with its Arabic text.
pub(crate) const TEXTS_AR: &[(&str, &str)] = &[
    (
        "manpower.bad_input",
        "راجع النموذج: هناك قيمة ناقصة أو غير مكتوبة بالشكل الصحيح.",
    ),
    ("manpower.notice.subject", "{{ title }}"),
    ("manpower.notice.body", "{{ body }}"),
    (
        "manpower.problems.client_unknown",
        "العميل ليس طرفاً معروفاً.",
    ),
    ("manpower.problems.site_name", "أدخل اسماً للموقع."),
    (
        "manpower.problems.site_duplicate",
        "لدى هذا العميل موقع بالاسم نفسه.",
    ),
    ("manpower.problems.reference", "أدخل مرجعاً للعقد."),
    (
        "manpower.problems.dates",
        "يجب أن يكون تاريخ الانتهاء بعد تاريخ البدء.",
    ),
    (
        "manpower.problems.payment_terms",
        "لا يمكن أن تكون شروط الدفع أو مدة الإشعار أو أيام الموافقة الضمنية سالبة أو صفراً.",
    ),
    (
        "manpower.problems.deemed_days",
        "أيام الموافقة الضمنية أيام كاملة من 1 فأكثر، أو فارغة إذا لم يكن هناك بند.",
    ),
    (
        "manpower.problems.terms_closed",
        "العقد ملغى أو منتهٍ: لا يمكن تغيير شروط جداول الدوام فيه.",
    ),
    (
        "manpower.problems.signed_in_future",
        "لا يمكن توقيع العقد في يوم لم يأتِ بعد.",
    ),
    (
        "manpower.problems.signed_document",
        "أرفق العقد الموقّع أولاً.",
    ),
    ("manpower.problems.not_draft", "لم يعد العقد مسودة."),
    (
        "manpower.contract.not_signed",
        "لم يوقّع العميل العقد: سجّل العقد الموقّع أولاً.",
    ),
    (
        "manpower.contract.no_rate_card",
        "اقبل إصدار جدول أسعار أولاً: لا يوجد إصدار جدول أسعار قبله العميل ساري المفعول في تاريخ بدء العقد.",
    ),
    (
        "manpower.contract.over",
        "انتهى العقد بالفعل: تاريخ انتهائه في الماضي.",
    ),
    (
        "manpower.contract.history_cancel",
        "أُلغي العقد قبل توقيعه وسريانه",
    ),
    (
        "manpower.hold.reason_required",
        "أدخل سبب التعليق الائتماني: تأخر السداد، أو شيك مرتجع، أو سبب آخر مع نصه.",
    ),
    ("manpower.hold.text_required", "اكتب ما هو السبب الآخر."),
    (
        "manpower.hold.already_held",
        "العقد معلّق بالفعل لهذا السبب.",
    ),
    (
        "manpower.hold.not_holdable",
        "لا يمكن تعليق العقد ائتمانياً إلا إذا كان سارياً أو قيد الإنهاء أو معلّقاً بالفعل.",
    ),
    (
        "manpower.hold.history_overdue",
        "تعليق ائتماني: الفاتورة {{ what }} متأخرة السداد",
    ),
    (
        "manpower.hold.history_bounced_cheque",
        "تعليق ائتماني: الشيك {{ what }} مرتجع",
    ),
    ("manpower.hold.history_other", "تعليق ائتماني: {{ text }}"),
    ("manpower.hold.held_title", "تعليق ائتماني: {{ contract }}"),
    (
        "manpower.hold.held_body",
        "العقد {{ contract }} الخاص بـ {{ client }} معلّق ائتمانياً. لن يُقبل أي طلب أو تفعيل أو تمديد حتى يُرفع التعليق. يبقى العمال في الموقع وتُصدر فواتيرهم؛ ويقرر المدير العام سحبهم من عدمه.\n{{ reasons }}\nالعمال في الموقع ({{ count }}):\n{{ workers }}",
    ),
    (
        "manpower.hold.added_title",
        "سبب آخر للتعليق الائتماني: {{ contract }}",
    ),
    (
        "manpower.hold.added_body",
        "كان العقد {{ contract }} الخاص بـ {{ client }} معلّقاً ائتمانياً بالفعل. أسباب التعليق:\n{{ reasons }}\nالعمال في الموقع ({{ count }}):\n{{ workers }}",
    ),
    (
        "manpower.hold.line_overdue",
        "متأخر السداد: {{ amount }} ({{ invoices }}).",
    ),
    (
        "manpower.hold.line_cheque",
        "شيك مرتجع {{ cheque }}: {{ amount }}.",
    ),
    ("manpower.hold.line_other", "أخرى: {{ text }}."),
    (
        "manpower.terminate.initiated_by",
        "حدّد من أنهى العقد: العميل أم الوكالة.",
    ),
    ("manpower.terminate.reason", "أدخل سبب إنهاء العقد مبكراً."),
    (
        "manpower.terminate.dates",
        "أدخل يوم تقديم الإشعار وتاريخ الانتهاء الجديد.",
    ),
    (
        "manpower.terminate.later",
        "تاريخ الانتهاء الجديد بعد تاريخ انتهاء العقد: هذا تجديد.",
    ),
    (
        "manpower.terminate.before_notice",
        "تاريخ الانتهاء الجديد قبل يوم تقديم الإشعار.",
    ),
    (
        "manpower.terminate.short_notice",
        "تاريخ الانتهاء يقع داخل مدة الإشعار البالغة {{ days }} يوماً: لا يمكن أن ينتهي العقد قبل {{ earliest }}، إلا إذا أُرفق تنازل الطرف الآخر.",
    ),
    ("manpower.terminate.cancel_reason", "إنهاء العقد"),
    (
        "manpower.terminate.history",
        "إنهاء مبكر بدأه {{ by }}: {{ reason }}",
    ),
    (
        "manpower.terminate.approved_title",
        "العقد {{ contract }} ينتهي مبكراً في {{ date }}",
    ),
    (
        "manpower.terminate.approved_body",
        "وافق المدير العام على الإنهاء المبكر للعقد {{ contract }} (بدأه {{ by }}، وقُدّم الإشعار في {{ notice }}: {{ reason }}). ينتهي العقد في {{ date }}. تنتهي عمليات النشر في ذلك اليوم على الأكثر؛ وتُلغى اللاحقة منها، وقد تلقى العميل التأكيد.",
    ),
    (
        "manpower.terminate.letter_title",
        "تأكيد: العقد {{ contract }} ينتهي في {{ date }}",
    ),
    (
        "manpower.terminate.letter_body",
        "نؤكد الإنهاء المبكر للعقد {{ contract }}، وقد قُدّم الإشعار في {{ notice }}. ينتهي العقد في {{ date }}. يبقى العمال في الموقع حتى ذلك اليوم.",
    ),
    (
        "manpower.terminate.withdrawn_title",
        "سُحب طلب إنهاء {{ contract }}",
    ),
    (
        "manpower.terminate.withdrawn_body",
        "سُحب طلب إنهاء العقد {{ contract }} مبكراً: {{ reason }}.",
    ),
    (
        "manpower.renew.later",
        "يجب أن يكون تاريخ الانتهاء الجديد بعد تاريخ الانتهاء الحالي، {{ end }}.",
    ),
    ("manpower.renew.no_date", "أدخل تاريخ الانتهاء الجديد."),
    (
        "manpower.renew.not_signed",
        "سجّل التجديد الذي وقّعه العميل: يوم التوقيع، على ألا يكون في المستقبل، والمستند.",
    ),
    (
        "manpower.renew.rates_date",
        "يجب أن تسري الأسعار الجديدة بعد بدء العقد ({{ start }}) وألا تتجاوز تاريخ انتهائه الجديد ({{ end }}).",
    ),
    (
        "manpower.renew.rate_version",
        "تتطلب الأسعار المعدّلة إصدار جدول أسعار جديداً يقبله العميل ويسري في {{ date }}.",
    ),
    ("manpower.renew.history", "جُدّد حتى {{ end }}"),
    ("manpower.renew.withdraw_reason", "تجديد العقد"),
    (
        "manpower.expiry.title",
        "العقد {{ contract }} ينتهي في {{ end }}",
    ),
    (
        "manpower.expiry.body",
        "ينتهي العقد {{ contract }} في {{ end }}، بعد {{ days }} يوماً من الآن، ولم يُجدَّد. ينتهي معه عمل {{ count }} من العمال في الموقع. جدّد العقد أو خطّط لتغطيتهم؛ عمليات النشر:",
    ),
    (
        "manpower.expiry.line",
        "{{ worker }}، {{ role }}، {{ site }}، حتى {{ until }}",
    ),
    ("manpower.expiry.none", "لا يوجد عامل في الموقع."),
    (
        "manpower.billing.title",
        "مسودة فاتورة لـ {{ client }}: {{ total }}",
    ),
    (
        "manpower.billing.body",
        "أُعدّت الساعات المعتمدة لـ {{ client }} كمسودة فاتورة بقيمة {{ total }} دون ضريبة القيمة المضافة، في {{ count }} بنداً. راجعها وأصدرها.",
    ),
    (
        "manpower.billing.unpriced_title",
        "ساعات لم تُفوتر: لا يوجد سعر لـ {{ worker }}",
    ),
    (
        "manpower.billing.unpriced_body",
        "لم تُفوتر الساعات المعتمدة لـ {{ worker }} ({{ role }}، وردية {{ shift }}) في جدول الدوام {{ timesheet }} بتاريخ {{ days }}: لا يتضمن العقد {{ contract }} إصدار جدول أسعار مقبولاً فيه بند لها في تلك الأيام. اقبل إصدار جدول أسعار يسعّرها، ثم أصدر فاتورتها يدوياً.",
    ),
    (
        "manpower.visa_step.first_title",
        "تأخر خطوة التأشيرة: {{ step }} لـ {{ worker }}",
    ),
    (
        "manpower.visa_step.first_body",
        "بدأت خطوة {{ step }} في ملف تأشيرة {{ case }} الخاص بـ {{ worker }} في {{ started }} واستغرقت {{ days }} يوماً مقابل هدف قدره {{ target }}. هذا الهدف داخلي للوكالة وليس مهلة قانونية. يرجى متابعتها.",
    ),
    (
        "manpower.visa_step.second_title",
        "خطوة التأشيرة ما زالت متأخرة: {{ step }} لـ {{ worker }}",
    ),
    (
        "manpower.visa_step.second_body",
        "خطوة {{ step }} في ملف تأشيرة {{ case }} الخاص بـ {{ worker }} لم تكتمل بعد، بعد {{ days }} يوماً من بدئها في {{ started }} (الهدف {{ target }} يوماً). أُبلغ مندوب العلاقات الحكومية قبل أسبوع.",
    ),
    (
        "manpower.end.not_yet",
        "ينتهي العقد في {{ end }}: يُنهى في اليوم التالي، وليس في {{ day }}.",
    ),
    ("manpower.end.cancel_reason", "انتهاء العقد"),
    ("manpower.end.history", "بلوغ تاريخ الانتهاء {{ end }}"),
    ("manpower.end.withdraw_reason", "انتهاء العقد"),
    ("manpower.end.bench_none", "لا أحد"),
    ("manpower.end.title", "انتهى العقد {{ contract }}"),
    (
        "manpower.end.body",
        "انتهى العقد {{ contract }} في {{ end }}. انتهت {{ ended }} من عمليات النشر وأُلغيت {{ cancelled }} من المخطط لها مع الطلبات المفتوحة. أُغلقت {{ timesheets }} من جداول الدوام الأخيرة وأُرسلت إلى العميل للموافقة، وتحتاج {{ open }} منها إلى متابعة قبل إرسالها. العائدون إلى الاحتياط: {{ bench }}.",
    ),
    (
        "manpower.release.not_on_hold",
        "لا يمكن رفع التعليق إلا عن عقد معلّق ائتمانياً.",
    ),
    (
        "manpower.release.state_before",
        "لا يعود العقد إلا إلى الحالة التي كان عليها قبل التعليق.",
    ),
    ("manpower.release.by_payment", "رُفع التعليق بالسداد"),
    (
        "manpower.release.released_title",
        "رُفع التعليق الائتماني: {{ contract }}",
    ),
    (
        "manpower.release.released_body",
        "رُفع التعليق الائتماني عن العقد {{ contract }}. أصبح {{ note }} مجدداً ويقبل الطلبات وعمليات النشر كما كان.",
    ),
    (
        "manpower.release.rejected_title",
        "رُفض رفع التعليق الائتماني: {{ contract }}",
    ),
    (
        "manpower.release.rejected_body",
        "لم تتم الموافقة على رفع التعليق الائتماني عن العقد {{ contract }}. يبقى معلّقاً. {{ note }}",
    ),
    (
        "manpower.hold.refusal",
        "العقد معلّق ائتمانياً منذ {{ since }}: {{ reason }}",
    ),
    ("manpower.hold.summary_overdue", "تأخر السداد ({{ what }})"),
    ("manpower.hold.summary_overdue_plain", "تأخر السداد"),
    ("manpower.hold.summary_cheque_plain", "شيك مرتجع"),
    (
        "manpower.hold.history_overdue_plain",
        "تعليق ائتماني: تأخر السداد",
    ),
    (
        "manpower.hold.history_bounced_cheque_plain",
        "تعليق ائتماني: شيك مرتجع",
    ),
    ("manpower.hold.summary_cheque", "شيك مرتجع {{ what }}"),
    (
        "manpower.hold.worker",
        "{{ worker }}، {{ role }} في {{ site }}",
    ),
    (
        "manpower.request.positions",
        "لم يعد في الطلب مكان شاغر لهذا العامل.",
    ),
    ("manpower.request.contract_unknown", "عقد العميل غير موجود."),
    ("manpower.request.contract_over", "عقد العميل منتهٍ أو ملغى."),
    ("manpower.request.site", "اختر موقعاً مستخدماً لدى العميل."),
    (
        "manpower.request.fields",
        "أدخل المهنة والوردية، واطلب عاملاً واحداً على الأقل.",
    ),
    ("manpower.request.dates", "يجب ألا ينتهي الطلب قبل أن يبدأ."),
    (
        "manpower.request.contract_not_active",
        "لا يقبل العقد طلبات جديدة حالياً.",
    ),
    (
        "manpower.request.portal_refused",
        "لا يمكن إرسال هذا الطلب الآن. يرجى التواصل مع مدير حسابك.",
    ),
    (
        "manpower.request.outside_contract",
        "يمتد الطلب إلى ما بعد تاريخ انتهاء العقد.",
    ),
    (
        "manpower.request.not_on_rate_card",
        "لا يتضمن جدول الأسعار سعراً لـ {{ role }} في وردية {{ shift }}.",
    ),
    ("manpower.request.submitted_title", "طلب جديد لـ {{ role }}"),
    (
        "manpower.request.submitted_body",
        "يطلب العميل {{ count }} {{ role }} اعتباراً من {{ from }}.",
    ),
    ("manpower.request.cancelled_deployments", "إلغاء الطلب"),
    (
        "manpower.blocked.task_title",
        "العميل موقوف: راجع عمليات النشر الخاصة به",
    ),
    (
        "manpower.blocked.task_body",
        "العميل موقوف. يواصل العمال في الموقع عملهم؛ ولن يُفعّل أو يُمدّد أي نشر، ولن يُجدّد أي عقد أو يُرسل أي طلب حتى يُرفع الإيقاف. يقرر المدير العام أي إنهاء. عمليات النشر الخاصة به:",
    ),
    (
        "manpower.blocked.line",
        "{{ worker }}، {{ role }} في {{ site }}، {{ status }}، من {{ from }} إلى {{ until }}",
    ),
    (
        "manpower.deployment.document_expiring",
        "مستند ينتهي {{ kind }} {{ date }}",
    ),
    (
        "manpower.deployment.document_label",
        "{{ role }} في {{ site }}، من {{ from }} إلى {{ until }}",
    ),
    (
        "manpower.deployment.checked_on",
        "قُيّمت المستندات في {{ date }}: يوم الإجراء مضافاً إليه {{ days }} يوماً، أو تاريخ انتهاء النشر إن كان أبكر.",
    ),
    (
        "manpower.deployment.contract_unknown",
        "عقد العميل غير موجود.",
    ),
    (
        "manpower.deployment.contract_closed",
        "عقد العميل مسودة أو منتهٍ أو ملغى: لا يُخطَّط أي عامل بموجبه.",
    ),
    (
        "manpower.deployment.employee",
        "الموظف غير معروف لدى الموارد البشرية.",
    ),
    (
        "manpower.deployment.dates",
        "يجب أن يبدأ النشر وينتهي ضمن تواريخ العقد، وألا ينتهي قبل أن يبدأ.",
    ),
    (
        "manpower.deployment.request",
        "اختر طلباً من هذا العقد ما زال مفتوحاً للمهنة والموقع نفسيهما.",
    ),
    (
        "manpower.deployment.shift",
        "أدخل أوقات الوردية بصيغة HH:MM وأيام العمل كقائمة مثل mon,tue,wed.",
    ),
    (
        "manpower.deployment.overlap",
        "لدى العامل بالفعل النشر رقم {{ id }} ({{ status }}) بمهنة {{ role }} في {{ site }}، من {{ from }} إلى {{ until }}، {{ shift }}، في ساعات متداخلة.",
    ),
    (
        "manpower.deployment.end_reason",
        "أدخل سبباً يمكنك استخدامه: حلّ تاريخ الانتهاء، أو حلّ عامل آخر محله.",
    ),
    (
        "manpower.check.expired",
        "{{ kind }} الخاص بـ {{ worker }} انتهت صلاحيته في {{ date }}",
    ),
    (
        "manpower.check.missing",
        "{{ kind }} الخاص بـ {{ worker }} غير موجود",
    ),
    (
        "manpower.check.rejected",
        "{{ kind }} الخاص بـ {{ worker }} مرفوض",
    ),
    (
        "manpower.check.pending",
        "{{ kind }} الخاص بـ {{ worker }} لم يُقبل بعد",
    ),
    ("manpower.check.override_allowed", "(يمكن تجاوزه بموافقة)"),
    ("manpower.check.override_never", "(لا يمكن تجاوزه)"),
    ("manpower.check.hold", "العقد معلّق. {{ detail }}"),
    (
        "manpower.deployment.employee_not_active",
        "الموظف غير نشط في الموارد البشرية: لا يبدأ النشر أثناء التعيين أو الإجازة أو فترة الإشعار أو الخروج.",
    ),
    (
        "manpower.deployment.contract_not_active",
        "لا يقبل عقد العميل عمليات نشر حالياً.",
    ),
    ("manpower.deployment.over", "انقضى تاريخ انتهاء النشر."),
    (
        "manpower.deployment.request_closed",
        "الطلب الذي يغطيه هذا النشر لم يعد يقبل عمالاً.",
    ),
    (
        "manpower.deployment.open_ended",
        "ليس للنشر تاريخ انتهاء لتمديده.",
    ),
    (
        "manpower.deployment.extend_later",
        "يجب أن يكون تاريخ الانتهاء الجديد بعد التاريخ الحالي.",
    ),
    (
        "manpower.deployment.beyond_contract",
        "تاريخ الانتهاء الجديد بعد انتهاء العقد.",
    ),
    (
        "manpower.deployment.end_date_not_come",
        "لم يحن تاريخ الانتهاء بعد: أنهِ النشر لسبب آخر.",
    ),
    (
        "manpower.deployment.activated_title",
        "{{ worker }} يبدأ العمل في {{ site }}",
    ),
    (
        "manpower.deployment.activated_body",
        "يبدأ {{ worker }} العمل بمهنة {{ role }} في {{ site }} بتاريخ {{ from }}.",
    ),
    ("manpower.deployment.extended_title", "مُدّد نشر {{ worker }}"),
    (
        "manpower.deployment.extended_body",
        "يبقى {{ worker }} بمهنة {{ role }} في {{ site }} حتى {{ until }}.",
    ),
    (
        "manpower.deployment.ended_title",
        "{{ worker }} يغادر {{ site }}",
    ),
    (
        "manpower.deployment.ended_body",
        "ينتهي نشر {{ worker }} بمهنة {{ role }} في {{ site }}. آخر يوم هو {{ until }}.",
    ),
    (
        "manpower.deployment.unavailable_title",
        "{{ worker }} لم يعد متاحاً",
    ),
    (
        "manpower.deployment.unavailable_body",
        "لم يعد {{ worker }} متاحاً لمهنة {{ role }} في {{ site }}. يمتد النشر من {{ from }} إلى {{ until }}.",
    ),
    (
        "manpower.deployment.notice_withdrawn_title",
        "{{ worker }} يبقى في {{ site }}",
    ),
    (
        "manpower.deployment.notice_withdrawn_body",
        "يستمر نشر {{ worker }} بمهنة {{ role }} في {{ site }} حتى {{ until }}.",
    ),
    ("manpower.exit.reason.absconded", "انقطاع العامل عن العمل"),
    ("manpower.exit.reason.deceased", "وفاة العامل"),
    ("manpower.exit.reason.exited", "ترك العامل الشركة"),
    ("manpower.exit.reason.onboarding_cancelled", "إلغاء التعيين"),
    (
        "manpower.exit.staff_title",
        "{{ worker }}: {{ what }} ({{ contract }})",
    ),
    (
        "manpower.exit.staff_body",
        "{{ worker }}: {{ what }}.\n{{ lines }}",
    ),
    (
        "manpower.exit.line_ended",
        "انتهى النشر في {{ site }} بمهنة {{ role }}، وآخر يوم {{ until }}.",
    ),
    (
        "manpower.exit.line_cancelled",
        "أُلغي النشر المخطط في {{ site }} بمهنة {{ role }} (كان سيبدأ في {{ from }}).",
    ),
    (
        "manpower.exit.line_request",
        "يبقى الطلب رقم {{ request }} دون شغل من هذا العامل ({{ filled }} من {{ headcount }} مشغولة). لا يُعاد فتحه: أنشئ طلب بديل إذا كان العميل ما زال بحاجة إليه.",
    ),
    (
        "manpower.exit.notice_restored_title",
        "سُحب الإشعار: {{ worker }} ({{ contract }})",
    ),
    (
        "manpower.exit.notice_restored_body",
        "{{ worker }}: سُحب الإشعار.\n{{ lines }}",
    ),
    (
        "manpower.exit.line_restored",
        "{{ site }} بمهنة {{ role }}: تاريخ الانتهاء {{ until }} مجدداً (كان الإشعار قد حدّده {{ was }}).",
    ),
    ("manpower.exit.visa_title", "رُفضت تأشيرة تجديد {{ worker }}"),
    (
        "manpower.exit.visa_body",
        "رُفض تجديد تأشيرة الإقامة لـ {{ worker }}. تنتهي التأشيرة الحالية في {{ expiry }}. عمليات النشر التالية تمتد إلى ما بعدها:\n{{ lines }}",
    ),
    (
        "manpower.exit.line_visa",
        "{{ contract }}: {{ site }} بمهنة {{ role }}، {{ status }}، ينتهي {{ until }}.",
    ),
    (
        "manpower.cover.missing_title",
        "لم يُعثر بعد على بديل لـ {{ role }} في {{ site }}",
    ),
    (
        "manpower.cover.missing_body",
        "لم نعثر على بديل بعد: ما زال {{ missing }} من {{ headcount }} {{ role }} مطلوبين في {{ site }} اعتباراً من {{ from }}. ما زلنا نعمل على ذلك.",
    ),
    (
        "manpower.absence.ticket_title",
        "غياب: {{ worker }} في {{ site }}، {{ days }} يوماً",
    ),
    (
        "manpower.absence.ticket_body",
        "لم يحضر {{ worker }} إلى {{ site }} (النشر رقم {{ deployment }}) منذ {{ last }}: لا حضور ولا إجازة معتمدة منذ {{ days }} يوماً. سجّل محاولات التواصل مع العامل (محاولتان فاشلتان في يومين مختلفين) ورقم بلاغ الغياب لدى وزارة الموارد البشرية والتوطين. ثم اطلب report_absconding. إذا عاد العامل، أغلق هذه الحالة مع ذكر السبب.",
    ),
    ("manpower.absence.client_title", "{{ worker }} لم يحضر"),
    (
        "manpower.absence.client_body",
        "لم يحضر {{ worker }} إلى {{ site }} منذ {{ since }}. نحن نتابع الأمر.",
    ),
    (
        "manpower.absence.reminder_title",
        "غياب {{ worker }}: {{ days }} يوماً",
    ),
    (
        "manpower.absence.reminder_body",
        "{{ worker }} غائب منذ {{ days }} يوماً. محاولات التواصل الفاشلة المسجلة في أيام مختلفة: {{ done }} من {{ need }}. رقم بلاغ الغياب لدى وزارة الموارد البشرية والتوطين: {{ number }}. لا تبلّغ عن الانقطاع عن العمل إلا بعد تسجيل الأمرين.",
    ),
    ("manpower.absence.number_missing", "غير مسجل"),
    ("manpower.absence.number_recorded", "مسجل"),
    (
        "manpower.absence.report",
        "أدخل رقم بلاغ الغياب لدى وزارة الموارد البشرية والتوطين وتاريخه (على ألا يكون في المستقبل).",
    ),
    (
        "manpower.absence.report_number",
        "أدخل رقم بلاغ وزارة الموارد البشرية والتوطين.",
    ),
    (
        "manpower.absence.report_date",
        "لا يمكن أن يكون تاريخ البلاغ في المستقبل.",
    ),
    (
        "manpower.absence.report_comment",
        "بلاغ غياب وزارة الموارد البشرية والتوطين رقم {{ number }} بتاريخ {{ date }}.",
    ),
    (
        "manpower.absence.reason",
        "أدخل سبب إغلاق الحالة: عاد العامل، أو دخل المستشفى...",
    ),
    ("manpower.absence.closed", "أُغلقت حالة الغياب."),
    ("manpower.absence.reinstated", "أُعيد العامل إلى العمل"),
    (
        "manpower.end.note_required",
        "اكتب ملاحظة العميل: لماذا رفض العميل العامل.",
    ),
    (
        "manpower.end.before_today",
        "لا يمكن أن يكون آخر يوم في الماضي.",
    ),
    (
        "manpower.end.short_notice",
        "مدة الإشعار في العقد {{ days }} يوماً: لا يمكن أن يكون آخر يوم قبل {{ earliest }} إلا إذا أرفقت تنازلاً.",
    ),
    ("manpower.end.error_reason", "نشر أُدخل بالخطأ"),
    ("manpower.end.adjust_title", "يلزم تعديل لـ {{ worker }}"),
    (
        "manpower.end.adjust_body",
        "أُدخل نشر {{ worker }} بالخطأ. {{ count }} من سجلات الحضور مقبولة أو ضمن جدول دوام مقفل ولم تُرفض: أنشئ جدول دوام تعديلي.",
    ),
    (
        "manpower.end.conduct_title",
        "السلوك: {{ worker }} في {{ site }}",
    ),
    (
        "manpower.end.conduct_body",
        "رفض العميل {{ worker }} في {{ site }}: {{ note }}",
    ),
    (
        "manpower.deployment.withdrawn_title",
        "{{ worker }} سيغادر {{ site }}",
    ),
    (
        "manpower.deployment.withdrawn_body",
        "ينتهي نشر {{ worker }} بمهنة {{ role }} في {{ site }} في {{ until }}، بعد مدة الإشعار المنصوص عليها في العقد.",
    ),
    (
        "manpower.attendance.suspended",
        "نشر هذا العامل في الموقع معلّق: لا يُسجّل الحضور حتى يُستأنف.",
    ),
    (
        "manpower.deployment.suspended_note",
        "معلّق: {{ reasons }}، {{ days }} يوماً من التعليق",
    ),
    (
        "manpower.deployment.suspended_note_one",
        "معلّق: {{ reasons }}، يوم واحد من التعليق",
    ),
    (
        "manpower.deployment.suspend_reason",
        "أدخل السبب: سحب تصريح دخول الموقع، أو تحقيق، أو سبب آخر.",
    ),
    (
        "manpower.deployment.suspend_state",
        "لا يمكن تعليق النشر إلا إذا كان نشطاً أو معلّقاً بالفعل.",
    ),
    (
        "manpower.deployment.suspend_document",
        "انتهت صلاحية {{ kind }}",
    ),
    (
        "manpower.deployment.suspended_title",
        "{{ worker }} غير متاح مؤقتاً",
    ),
    (
        "manpower.deployment.suspended_body",
        "{{ worker }} غير متاح مؤقتاً في {{ site }} اعتباراً من {{ from }}. سنؤكد تاريخ العودة.",
    ),
    (
        "manpower.deployment.suspended_staff_title",
        "عُلّق نشر {{ worker }}",
    ),
    (
        "manpower.deployment.suspended_staff_body",
        "عُلّق نشر {{ worker }} في {{ site }} اعتباراً من {{ from }}: {{ reasons }}. يُرفض تسجيل الحضور ولا تُفوتر الأيام حتى يُستأنف.",
    ),
    (
        "manpower.deployment.resume_contract_ended",
        "انتهى عقد العميل: لا يُستأنف أي نشر بموجبه.",
    ),
    (
        "manpower.deployment.resumed_title",
        "{{ worker }} متاح مجدداً",
    ),
    (
        "manpower.deployment.resumed_body",
        "{{ worker }} متاح مجدداً في {{ site }} اعتباراً من {{ on }}.",
    ),
    (
        "manpower.deployment.resume_failing_title",
        "يبقى نشر {{ worker }} معلّقاً",
    ),
    (
        "manpower.deployment.resume_failing_body",
        "قُبل المستند المجدَّد، لكن لا يمكن لـ {{ worker }} الاستئناف بعد:\n{{ problems }}",
    ),
    ("manpower.cover.history", "فُتح طلب تغطية: {{ cause }}"),
    (
        "manpower.cover.not_covered",
        "لا يمكن تغطية إلا عامل في الموقع، من يوم إلى يوم لا يسبقه.",
    ),
    (
        "manpower.cover.contract_closed",
        "عقد العميل مسودة أو منتهٍ أو ملغى: لا يقبل طلبات تغطية.",
    ),
    ("manpower.cover.opened_title", "مطلوب بديل لـ {{ worker }}"),
    (
        "manpower.cover.opened_body",
        "{{ worker }} ({{ role }} في {{ site }}) غائب من {{ from }} حتى {{ until }}. فُتح طلب تغطية لعامل واحد بمهنة {{ role }}.",
    ),
    ("manpower.screen.status", "الحالة"),
    ("manpower.screen.client", "العميل"),
    ("manpower.screen.none", "لا شيء."),
    ("manpower.screen.contracts.title", "عقود العملاء"),
    (
        "manpower.screen.contracts.subtitle",
        "الاتفاقيات مع الشركات العميلة.",
    ),
    ("manpower.screen.contracts.reference", "العقد"),
    ("manpower.screen.contracts.period", "المدة"),
    ("manpower.screen.contracts.billing", "الفوترة"),
    ("manpower.screen.contracts.cover", "التغطية"),
    ("manpower.screen.contracts.cover_required", "التغطية مطلوبة"),
    ("manpower.screen.contracts.cover_none", "غير مطلوبة"),
    ("manpower.screen.open_ended", "مفتوح المدة"),
    ("manpower.screen.contracts.held_since", "معلّق ائتمانياً منذ"),
    ("manpower.screen.contracts.held_before", "قبل التعليق:"),
    ("manpower.screen.contracts.empty", "لا توجد عقود عملاء بعد"),
    (
        "manpower.screen.contracts.empty_text",
        "يُبرم عقد مع الشركة العميلة قبل طلب عمالها.",
    ),
    ("manpower.screen.requests.title", "طلبات العمالة"),
    (
        "manpower.screen.requests.subtitle",
        "العمال الذين يطلبهم العملاء.",
    ),
    ("manpower.screen.requests.request", "الطلب"),
    ("manpower.screen.requests.workers", "العمال"),
    ("manpower.screen.requests.period", "المدة"),
    ("manpower.screen.requests.cover", "التغطية"),
    ("manpower.screen.requests.empty", "لا توجد طلبات بعد"),
    (
        "manpower.screen.requests.empty_text",
        "يطلب العميل العمال حسب المهنة والعدد والموقع والوردية.",
    ),
    ("manpower.screen.deployments.title", "عمليات النشر"),
    (
        "manpower.screen.deployments.subtitle",
        "العمال المنتشرون في مواقع العملاء.",
    ),
    ("manpower.screen.deployments.worker", "العامل"),
    ("manpower.screen.deployments.shift", "الوردية"),
    ("manpower.screen.deployments.period", "المدة"),
    ("manpower.screen.deployments.cover", "التغطية"),
    ("manpower.screen.deployments.empty", "لا توجد عمليات نشر بعد"),
    (
        "manpower.screen.deployments.empty_text",
        "يُخطَّط للعامل في موقع، ثم يُفعّل النشر عندما تسمح المستندات والعقد.",
    ),
    ("manpower.screen.status.planned", "مخطط"),
    ("manpower.screen.status.suspended", "معلّق"),
    ("manpower.screen.status.draft", "مسودة"),
    ("manpower.screen.status.open", "مفتوح"),
    ("manpower.screen.status.partially_filled", "مشغول جزئياً"),
    ("manpower.screen.status.filled", "مشغول بالكامل"),
    ("manpower.screen.status.active", "نشط"),
    ("manpower.screen.status.on_hold", "معلّق ائتمانياً"),
    ("manpower.screen.status.terminating", "قيد الإنهاء"),
    ("manpower.screen.status.ended", "منتهٍ"),
    ("manpower.screen.status.cancelled", "ملغى"),
    ("manpower.screen.billing.weekly", "أسبوعياً"),
    ("manpower.screen.billing.fortnightly", "كل أسبوعين"),
    ("manpower.screen.billing.monthly", "شهرياً"),
    ("manpower.problems.client_missing", "اختر الشركة العميلة."),
    ("manpower.problems.contract_missing", "اختر عقد العميل."),
    (
        "manpower.problems.dates_missing",
        "أدخل تاريخ البدء وتاريخ الانتهاء.",
    ),
    (
        "manpower.problems.headcount_missing",
        "حدّد عدد العمال المطلوبين.",
    ),
    (
        "manpower.problems.signed_date_missing",
        "أدخل يوم توقيع العميل.",
    ),
    ("manpower.problems.site_missing", "اختر الموقع."),
    ("manpower.problems.start_missing", "أدخل اليوم الأول."),
    ("manpower.screen.not_set", "غير محدد"),
    ("manpower.screen.fields.status", "الحالة"),
    ("manpower.screen.action.sign", "تسجيل التوقيع"),
    (
        "manpower.screen.action.sign_hint",
        "سجّل يوم توقيع العميل والعقد الموقّع. يبقى العقد مسودة حتى يُفعّل.",
    ),
    ("manpower.screen.action.activate", "تفعيل"),
    (
        "manpower.screen.action.activate_hint",
        "يصبح العقد سارياً: يمكن أن تبدأ الطلبات وعمليات النشر بموجبه.",
    ),
    ("manpower.screen.action.cancel", "إلغاء"),
    (
        "manpower.screen.action.cancel_hint",
        "يُلغى العقد الذي لم يُوقّع ولم يسرِ قط. لا يمكن التراجع عن ذلك.",
    ),
    ("manpower.screen.action.hold", "تعليق"),
    (
        "manpower.screen.action.hold_hint",
        "لا يُقبل أي طلب أو تفعيل أو تمديد جديد ما دام العقد معلّقاً ائتمانياً. يبقى العمال في الموقع.",
    ),
    ("manpower.screen.action.release", "رفع التعليق"),
    (
        "manpower.screen.action.release_hint",
        "إنهاء التعليق الائتماني. ما دام أحد أسباب التعليق قائماً، يقرر المدير العام.",
    ),
    ("manpower.screen.action.renew", "تجديد"),
    (
        "manpower.screen.action.renew_hint",
        "تأخير تاريخ الانتهاء بموجب التجديد الموقّع. تتطلب الأسعار المعدّلة إصدار جدول أسعار يقبله العميل.",
    ),
    ("manpower.screen.action.terminate", "إنهاء مبكر"),
    (
        "manpower.screen.action.terminate_hint",
        "طلب إنهاء العقد قبل تاريخ انتهائه. يوافق المدير العام.",
    ),
    ("manpower.screen.action.terms", "شروط جداول الدوام"),
    (
        "manpower.screen.action.terms_hint",
        "عدد الأيام التي يُعدّ بعدها جدول الدوام الذي لم يرد عليه العميل معتمداً، والجهة التي تُرسل إليها التذكيرات.",
    ),
    ("manpower.screen.action.submit", "إرسال"),
    (
        "manpower.screen.action.submit_hint",
        "أرسل الطلب إلى الوكالة: يمكن عندها البحث عن عمال له.",
    ),
    ("manpower.screen.action.plan", "تخطيط نشر"),
    (
        "manpower.screen.action.plan_hint",
        "اختر العامل لهذا الطلب. الطلب والعقد والموقع والوردية محددة مسبقاً.",
    ),
    ("manpower.screen.tab.overview", "نظرة عامة"),
    ("manpower.screen.tab.deployments", "عمليات النشر"),
    ("manpower.screen.tab.rates", "جداول الأسعار"),
    ("manpower.screen.tab.approvals", "الموافقات"),
    ("manpower.screen.tab.activity", "السجل"),
    ("manpower.screen.contract.title", "العقد"),
    ("manpower.screen.contract.period", "المدة"),
    ("manpower.screen.contract.payment_terms", "شروط الدفع"),
    ("manpower.screen.contract.days", "{{ days }} يوماً"),
    ("manpower.screen.contract.notice", "مدة الإشعار"),
    (
        "manpower.screen.contract.deemed",
        "الموافقة الضمنية على جدول الدوام",
    ),
    ("manpower.screen.contract.no_deemed", "لا توجد موافقة ضمنية"),
    ("manpower.screen.contract.escalation", "جهة التصعيد"),
    ("manpower.screen.contract.manager", "مدير الحساب"),
    ("manpower.screen.contract.signed", "التوقيع"),
    ("manpower.screen.contract.signed_on", "وُقّع في {{ date }}"),
    ("manpower.screen.contract.not_signed", "لم يُوقّع بعد"),
    ("manpower.screen.contract.document", "العقد الموقّع"),
    ("manpower.screen.contract.no_document", "غير مرفق"),
    ("manpower.screen.contract.activated", "نشط منذ"),
    ("manpower.screen.contract.cancel_reason", "سبب الإلغاء"),
    ("manpower.screen.contract.termination", "الإنهاء المبكر"),
    (
        "manpower.screen.contract.terminating_line",
        "أنهاه {{ by }}: {{ reason }}. قُدّم الإشعار في {{ notice }}. كان تاريخ الانتهاء {{ was }}.",
    ),
    ("manpower.screen.contract.renewal", "آخر تجديد"),
    (
        "manpower.screen.contract.renewed_line",
        "جُدّد في {{ on }} حتى {{ end }} (كان ينتهي في {{ was }}). الأسعار الجديدة اعتباراً من {{ rates }}.",
    ),
    ("manpower.screen.contract.checks", "قبل أن يمكن تفعيله"),
    (
        "manpower.screen.contract.deployments_empty",
        "يظهر هنا العمال المنتشرون بموجب هذا العقد.",
    ),
    ("manpower.screen.contract.no_licences", "لا يُطلب أي ترخيص"),
    (
        "manpower.screen.hold.banner",
        "هذا العقد معلّق ائتمانياً. لا يُقبل أي طلب أو تفعيل أو تمديد جديد؛ ويبقى العمال في الموقع.",
    ),
    ("manpower.screen.hold.title", "التعليق الائتماني"),
    ("manpower.screen.hold.since", "معلّق منذ"),
    ("manpower.screen.hold.before", "الحالة قبل التعليق"),
    ("manpower.screen.hold.by", "علّقه"),
    ("manpower.screen.hold.by_rule", "الفحص اليومي"),
    ("manpower.screen.hold.reasons", "الأسباب"),
    (
        "manpower.screen.hold.overdue",
        "فاتورة متأخرة السداد {{ what }} {{ amount }}",
    ),
    (
        "manpower.screen.hold.cheque",
        "شيك مرتجع {{ what }} {{ amount }}",
    ),
    ("manpower.screen.hold.other", "{{ text }}"),
    ("manpower.screen.hold.line", "{{ cause }}، منذ {{ since }}"),
    (
        "manpower.screen.hold_reason.overdue",
        "فاتورة متأخرة السداد",
    ),
    ("manpower.screen.hold_reason.bounced_cheque", "شيك مرتجع"),
    ("manpower.screen.hold_reason.other", "سبب آخر"),
    ("manpower.screen.initiator.client", "العميل"),
    ("manpower.screen.initiator.agency", "الوكالة"),
    ("manpower.screen.sites.title", "مواقع العميل"),
    ("manpower.screen.sites.new", "موقع جديد"),
    ("manpower.screen.sites.edit", "تعديل"),
    ("manpower.screen.sites.save", "حفظ الموقع"),
    ("manpower.screen.sites.name", "الاسم"),
    ("manpower.screen.sites.address", "العنوان"),
    ("manpower.screen.sites.licences", "التراخيص المطلوبة"),
    (
        "manpower.screen.sites.licences_hint",
        "بالكلمات، مثلاً: ترخيص SIRA، تصريح دخول الموقع.",
    ),
    ("manpower.screen.sites.actions", "الإجراءات"),
    ("manpower.screen.sites.empty", "لا يوجد موقع بعد"),
    ("manpower.screen.rates.version", "الإصدار"),
    ("manpower.screen.rates.open_card", "فتح جدول الأسعار"),
    ("manpower.screen.rates.start", "إنشاء جدول الأسعار"),
    (
        "manpower.screen.rates.start_hint",
        "ينشئ جدول أسعار فارغاً لهذا العقد. ثم أضف إصداره الأول بتاريخ السريان والبنود.",
    ),
    ("manpower.screen.rates.effective", "الأسعار اعتباراً من"),
    ("manpower.screen.rates.accepted", "مقبول"),
    ("manpower.screen.rates.lines", "المهن المسعّرة"),
    ("manpower.screen.rates.empty", "لا يوجد جدول أسعار بعد"),
    (
        "manpower.screen.rates.empty_text",
        "يظهر هنا جدول أسعار العقد وإصداراته.",
    ),
    ("manpower.screen.rate_status.pending", "بانتظار العميل"),
    (
        "manpower.screen.rate_status.awaiting_approval",
        "بانتظار المدير العام",
    ),
    ("manpower.screen.rate_status.accepted", "مقبول"),
    ("manpower.screen.rate_status.rejected", "مرفوض"),
    ("manpower.screen.approval.request", "الطلب"),
    ("manpower.screen.approval.requester", "مقدّم الطلب"),
    ("manpower.screen.approval.requested", "تاريخ الطلب"),
    ("manpower.screen.approval.decided", "تاريخ القرار"),
    ("manpower.screen.approval.decision", "القرار"),
    ("manpower.screen.approval.approve", "موافقة"),
    ("manpower.screen.approval.reject", "رفض"),
    ("manpower.screen.approval.comment", "تعليق"),
    ("manpower.screen.approval.comment_optional", "اختياري."),
    (
        "manpower.screen.approval.comment_required",
        "اذكر السبب. مطلوب.",
    ),
    ("manpower.screen.approval.waiting", "بانتظار المدير العام"),
    ("manpower.screen.approval.note", "ملاحظة"),
    ("manpower.screen.approval.return", "إعادة للتعديل"),
    (
        "manpower.screen.approval.waiting_for",
        "بانتظار المدير العام: {{ action }}.",
    ),
    ("manpower.screen.approval.empty", "لم تُطلب أي موافقة"),
    (
        "manpower.screen.approval.empty_text",
        "ينتظر هنا رفع التعليق الائتماني أو الإنهاء المبكر قرار المدير العام.",
    ),
    ("manpower.screen.approval.approved", "تمت الموافقة"),
    ("manpower.screen.approval.rejected", "مرفوض"),
    ("manpower.screen.approval.returned", "أُعيد"),
    ("manpower.screen.approval.withdrawn", "مسحوب"),
    ("manpower.screen.approval.expired", "منتهي الصلاحية"),
    ("manpower.screen.approval.failed", "تعذّر التنفيذ"),
    ("manpower.screen.approval.pending", "بانتظار المدير العام"),
    ("manpower.screen.activity.when", "متى"),
    ("manpower.screen.activity.what", "ماذا"),
    ("manpower.screen.activity.by", "بواسطة"),
    ("manpower.screen.activity.reason", "السبب"),
    ("manpower.screen.activity.system", "النظام"),
    ("manpower.screen.activity.empty", "لا شيء بعد"),
    ("manpower.screen.transition.insert", "سُجّل"),
    ("manpower.screen.transition.activate", "فُعّل"),
    ("manpower.screen.transition.cancel", "أُلغي"),
    ("manpower.screen.transition.hold", "عُلّق"),
    (
        "manpower.screen.transition.auto_hold",
        "عُلّق بواسطة الفحص اليومي",
    ),
    ("manpower.screen.transition.release", "رُفع التعليق"),
    (
        "manpower.screen.transition.release_terminating",
        "رُفع التعليق",
    ),
    (
        "manpower.screen.transition.release_approved",
        "رُفع التعليق بعد الموافقة",
    ),
    (
        "manpower.screen.transition.release_terminating_approved",
        "رُفع التعليق بعد الموافقة",
    ),
    (
        "manpower.screen.transition.auto_release",
        "رُفع التعليق بواسطة الفحص اليومي",
    ),
    (
        "manpower.screen.transition.auto_release_terminating",
        "رُفع التعليق بواسطة الفحص اليومي",
    ),
    ("manpower.screen.transition.terminate", "طُلب الإنهاء المبكر"),
    (
        "manpower.screen.transition.terminate_approved",
        "تمت الموافقة على الإنهاء المبكر",
    ),
    ("manpower.screen.transition.renew", "جُدّد"),
    (
        "manpower.screen.transition.renew_on_hold",
        "جُدّد أثناء التعليق",
    ),
    ("manpower.screen.transition.end", "انتهى"),
    ("manpower.screen.transition.submit", "أُرسل"),
    ("manpower.screen.transition.fill", "شُغل بالكامل"),
    ("manpower.screen.transition.fill_partly", "شُغل جزئياً"),
    ("manpower.screen.transition.auto_cancel", "ألغاه النظام"),
    ("manpower.screen.done.created", "أُنشئ العقد."),
    ("manpower.screen.done.signed", "سُجّل التوقيع."),
    ("manpower.screen.done.activated", "العقد نشط."),
    ("manpower.screen.done.cancelled", "أُلغي العقد."),
    ("manpower.screen.done.held", "العقد معلّق."),
    (
        "manpower.screen.done.release_asked",
        "بانتظار موافقة المدير العام على رفع التعليق.",
    ),
    ("manpower.screen.done.released", "رُفع التعليق عن العقد."),
    (
        "manpower.screen.done.terminate_asked",
        "بانتظار موافقة المدير العام على الإنهاء المبكر.",
    ),
    (
        "manpower.screen.done.terminated",
        "العقد قيد الإنهاء المبكر.",
    ),
    ("manpower.screen.done.renewed", "جُدّد العقد."),
    ("manpower.screen.done.terms", "حُفظت شروط جداول الدوام."),
    ("manpower.screen.done.site_created", "أُضيف الموقع."),
    ("manpower.screen.done.site_updated", "حُفظ الموقع."),
    ("manpower.screen.done.request_created", "حُفظ الطلب كمسودة."),
    ("manpower.screen.done.request_submitted", "أُرسل الطلب."),
    ("manpower.screen.done.request_cancelled", "أُلغي الطلب."),
    ("manpower.screen.done.cover_asked", "فُتح طلب التغطية."),
    ("manpower.screen.form.client", "الشركة العميلة"),
    (
        "manpower.screen.form.client_placeholder",
        "ابحث عن العميل بالاسم",
    ),
    ("manpower.screen.form.reference", "المرجع"),
    (
        "manpower.screen.form.reference_hint",
        "الرقم الذي تعطيه الوكالة للعقد، مثلاً CC-2026-004.",
    ),
    ("manpower.screen.form.start_date", "اليوم الأول"),
    ("manpower.screen.form.end_date", "اليوم الأخير"),
    (
        "manpower.screen.form.end_date_hint",
        "اتركه فارغاً إذا لم يكن هناك تاريخ انتهاء.",
    ),
    ("manpower.screen.form.billing", "دورة الفوترة"),
    ("manpower.screen.form.payment_terms", "أيام السداد"),
    ("manpower.screen.form.notice_days", "أيام الإشعار"),
    (
        "manpower.screen.form.cover_required",
        "يجب على الوكالة تغطية العمال الغائبين",
    ),
    ("manpower.screen.form.deemed_days", "أيام الموافقة الضمنية"),
    (
        "manpower.screen.form.deemed_days_hint",
        "يُعدّ جدول الدوام الذي لا يرد عليه العميل خلال هذا العدد من الأيام معتمداً. اتركه فارغاً إذا لم يوجد.",
    ),
    ("manpower.screen.form.escalation", "جهة التصعيد"),
    (
        "manpower.screen.form.escalation_placeholder",
        "ابحث عن جهة الاتصال بالاسم",
    ),
    ("manpower.screen.form.signed_on", "تاريخ التوقيع"),
    ("manpower.screen.form.document", "العقد الموقّع"),
    (
        "manpower.screen.form.document_hint",
        "اسم ملف العقد الموقّع أو رقمه.",
    ),
    ("manpower.screen.form.hold_reason", "السبب"),
    ("manpower.screen.form.hold_text", "التفاصيل"),
    (
        "manpower.screen.form.hold_text_hint",
        "رقم الفاتورة أو الشيك، أو ما هو السبب الآخر.",
    ),
    ("manpower.screen.form.new_end", "تاريخ الانتهاء الجديد"),
    (
        "manpower.screen.form.renewal_signed_on",
        "تاريخ توقيع التجديد",
    ),
    ("manpower.screen.form.renewal_document", "التجديد الموقّع"),
    (
        "manpower.screen.form.rates_from",
        "الأسعار الجديدة اعتباراً من",
    ),
    (
        "manpower.screen.form.rates_from_hint",
        "فقط عند تغيّر الأسعار: يوم سريان إصدار جدول الأسعار الجديد.",
    ),
    ("manpower.screen.form.initiated_by", "أنهاه"),
    ("manpower.screen.form.terminate_reason", "السبب"),
    ("manpower.screen.form.notice_on", "تاريخ تقديم الإشعار"),
    ("manpower.screen.form.last_day", "آخر يوم في العقد"),
    ("manpower.screen.form.waiver", "التنازل"),
    (
        "manpower.screen.form.waiver_hint",
        "التنازل الموقّع عندما يكون الإشعار أقصر من مدة الإشعار.",
    ),
    ("manpower.screen.form.cancel_reason", "السبب"),
    ("manpower.screen.form.contract", "العقد"),
    ("manpower.screen.form.site", "الموقع"),
    ("manpower.screen.form.role", "المهنة"),
    ("manpower.screen.form.positions", "عدد الوظائف"),
    ("manpower.screen.form.shift", "الوردية"),
    ("manpower.screen.form.shift_start", "بداية الوردية"),
    ("manpower.screen.form.shift_end", "نهاية الوردية"),
    ("manpower.screen.form.worker", "العامل"),
    ("manpower.screen.form.worker_placeholder", "اختر العامل"),
    ("manpower.screen.shift.day", "وردية نهارية"),
    ("manpower.screen.shift.night", "وردية ليلية"),
    ("manpower.screen.contracts.new", "عقد جديد"),
    (
        "manpower.screen.contracts.new_hint",
        "مسودة عقد لعميل. يصبح سارياً بعد توقيعه ووجود إصدار جدول أسعار مقبول.",
    ),
    ("manpower.screen.contracts.new_save", "إنشاء العقد"),
    ("manpower.screen.requests.new", "طلب جديد"),
    (
        "manpower.screen.requests.new_hint",
        "طلب عميل للعمال. يبقى مسودة حتى يُرسل.",
    ),
    ("manpower.screen.requests.new_save", "حفظ الطلب"),
    (
        "manpower.screen.requests.contract_option",
        "{{ reference }}، {{ client }}",
    ),
    ("manpower.screen.deployments.role", "المهنة"),
    ("manpower.screen.deployments.site", "الموقع"),
    ("manpower.screen.contracts.hold", "التعليق الائتماني"),
    ("manpower.screen.contracts.signed_on", "تاريخ التوقيع"),
    ("manpower.screen.requests.kind", "النوع"),
    ("manpower.screen.requests.standard", "عادي"),
    ("manpower.screen.requests.filled", "المشغول"),
    ("manpower.screen.deployments.kind", "النوع"),
    ("manpower.screen.deployments.standard", "عادي"),
    ("manpower.screen.deployments.flags", "تنبيهات"),
    ("manpower.screen.request.title", "الطلب"),
    ("manpower.screen.request.contract", "العقد"),
    ("manpower.screen.request.cause", "سبب وجود هذا الطلب"),
    (
        "manpower.screen.request.cause_link",
        "فتح النشر الذي يحل محله",
    ),
    ("manpower.screen.request.deployments", "عمليات النشر"),
    ("manpower.screen.request.no_deployments", "لا يوجد عامل بعد"),
    (
        "manpower.screen.request.no_deployments_text",
        "يظهر هنا العمال المخطط لهم لهذا الطلب.",
    ),
    ("manpower.screen.requests.site", "الموقع"),
    ("manpower.screen.requests.shift", "الوردية"),
    (
        "manpower.screen.requests.shift_hours",
        "{{ shift }}، من {{ start }} إلى {{ end }}",
    ),
    ("manpower.screen.requests.submitted", "تاريخ الإرسال"),
    ("manpower.screen.requests.cancel_reason", "سبب الإلغاء"),
    (
        "manpower.screen.requests.positions",
        "{{ filled }} من {{ headcount }} مشغولة، {{ missing }} ناقصة",
    ),
    ("manpower.screen.requests.suspended", "{{ count }} معلّق"),
    (
        "manpower.screen.requests.active_one",
        "ما زال {{ count }} نشر نشطاً.",
    ),
    (
        "manpower.screen.requests.active_many",
        "ما زالت {{ count }} من عمليات النشر نشطة.",
    ),
    (
        "manpower.screen.requests.on_hold_blocked",
        "العقد معلّق. تنتظر الطلبات الجديدة رفع التعليق.",
    ),
    (
        "manpower.screen.requests.contract_blocked",
        "لا يقبل العقد طلبات جديدة حالياً.",
    ),
    ("manpower.screen.requests.plan_blocked", "أرسل الطلب أولاً."),
    ("manpower.screen.requests.an_absent_worker", "عامل غائب"),
    (
        "manpower.screen.requests.cause_rejection",
        "بديل عن {{ worker }}، أُنهي نشره برفض العميل في {{ date }}",
    ),
    (
        "manpower.screen.requests.cause_leave",
        "تغطية لـ {{ worker }} أثناء إجازته",
    ),
    (
        "manpower.screen.requests.cause_suspension",
        "تغطية لـ {{ worker }} الذي عُلّق نشره",
    ),
    (
        "manpower.screen.requests.cause_absence",
        "تغطية لعامل غائب، {{ worker }}",
    ),
    (
        "manpower.screen.requests.cause_manual",
        "تغطية طُلبت يدوياً لـ {{ worker }}",
    ),
    (
        "manpower.screen.requests.client_note",
        "كتب العميل: {{ note }}",
    ),
    ("manpower.screen.status.voided", "مُبطل"),
    ("manpower.screen.deployment.title", "النشر"),
    ("manpower.screen.deployment.worker", "العامل"),
    ("manpower.screen.deployment.client", "العميل"),
    ("manpower.screen.deployment.is_cover", "يحل محل عامل آخر"),
    ("manpower.screen.deployment.activated", "تاريخ التفعيل"),
    ("manpower.screen.deployment.suspension", "التعليق"),
    ("manpower.screen.deployment.cancel_reason", "سبب الإلغاء"),
    (
        "manpower.screen.deployment.days_suspended_one",
        "يوم واحد من التعليق",
    ),
    (
        "manpower.screen.deployment.days_suspended",
        "{{ days }} يوماً من التعليق",
    ),
    (
        "manpower.screen.deployment.suspended_because",
        "معلّق: {{ reasons }}. {{ days }}.",
    ),
    (
        "manpower.screen.deployment.withdrawal_planned",
        "يُسحب العامل بعد {{ date }} لأن العميل لم يسدد.",
    ),
    ("manpower.screen.deployment.ended", "كيف انتهى"),
    (
        "manpower.screen.deployment.ended_because",
        "انتهى في {{ date }}: {{ reason }}.",
    ),
    (
        "manpower.screen.deployment.client_note",
        "ملاحظة العميل: {{ note }}",
    ),
    (
        "manpower.screen.deployment.waiver_note",
        "التنازل عن مدة الإشعار: {{ waiver }}",
    ),
    (
        "manpower.screen.deployment.replacement_link",
        "فتح طلب البديل",
    ),
    (
        "manpower.screen.deployment.covers",
        "هذا النشر يحل محل {{ worker }}",
    ),
    (
        "manpower.screen.deployment.cover_request",
        "طلب تغطية، {{ period }} ({{ status }})",
    ),
    ("manpower.screen.deployment.cover_title", "التغطية"),
    (
        "manpower.screen.deployment.checks",
        "لماذا لا يمكن أن يبدأ بعد",
    ),
    (
        "manpower.screen.deployment.request_override",
        "طلب تجاوز (موافقة المدير العام)",
    ),
    ("manpower.screen.deployment.documents", "المستندات"),
    ("manpower.screen.deployment.document", "المستند"),
    ("manpower.screen.deployment.validity", "الصلاحية"),
    (
        "manpower.screen.deployment.documents_empty",
        "لا يلزم أي مستند لهذه المهنة.",
    ),
    ("manpower.screen.deployment.doc_no_expiry", "لا تنتهي صلاحيته"),
    (
        "manpower.screen.deployment.doc_valid_until",
        "صالح حتى {{ date }}",
    ),
    (
        "manpower.screen.deployment.doc_expired_on",
        "انتهت صلاحيته في {{ date }}",
    ),
    ("manpower.screen.deployment.timesheets", "جداول الدوام"),
    (
        "manpower.screen.deployment.timesheets_empty",
        "لا يوجد جدول دوام بعد.",
    ),
    ("manpower.screen.deployment.sheet_kind", "النوع"),
    (
        "manpower.screen.deployment.activate_hint",
        "يبدأ النشر اليوم. يُعرض كل فحص يفشل، ولا يبدأ شيء حتى تنجح جميعها.",
    ),
    ("manpower.screen.deployment.extend", "تمديد"),
    (
        "manpower.screen.deployment.extend_hint",
        "يؤخّر اليوم الأخير. يجب أن تبقى المستندات صالحة حتى التاريخ الجديد.",
    ),
    ("manpower.screen.deployment.suspend", "تعليق"),
    (
        "manpower.screen.deployment.suspend_hint",
        "يتوقف العامل عن العمل في الموقع ولا تُفوتر الأيام.",
    ),
    ("manpower.screen.deployment.suspend_reason", "السبب"),
    ("manpower.screen.deployment.resume", "استئناف"),
    (
        "manpower.screen.deployment.resume_hint",
        "يعود العامل إلى الموقع. يجب أن تكون المستندات صالحة مجدداً.",
    ),
    ("manpower.screen.deployment.cover", "طلب تغطية"),
    (
        "manpower.screen.deployment.cover_hint",
        "يفتح طلباً لعامل يحل محله في الأيام التي يغيب فيها.",
    ),
    ("manpower.screen.deployment.cover_from", "من"),
    ("manpower.screen.deployment.cover_to", "إلى"),
    (
        "manpower.screen.deployment.cover_to_hint",
        "اتركه فارغاً ما دام تاريخ العودة غير معروف.",
    ),
    ("manpower.screen.deployment.end", "إنهاء"),
    (
        "manpower.screen.deployment.end_hint",
        "ينهي النشر. يحدد السبب ما يُطلب بعد ذلك.",
    ),
    ("manpower.screen.deployment.end_reason", "السبب"),
    ("manpower.screen.deployment.end_note", "ملاحظة العميل"),
    (
        "manpower.screen.deployment.end_note_hint",
        "اكتب ما قاله العميل. يُفتح طلب بديل عندما يتطلب العقد التغطية.",
    ),
    (
        "manpower.screen.deployment.end_conduct",
        "الأمر يتعلق بالسلوك: افتح تذكرة شكوى",
    ),
    (
        "manpower.screen.deployment.end_last_day",
        "آخر يوم في الموقع",
    ),
    (
        "manpower.screen.deployment.end_last_day_hint",
        "بعد مدة الإشعار في العقد على الأقل من اليوم، إلا إذا قُدّم تنازل.",
    ),
    (
        "manpower.screen.deployment.cancel_hint",
        "لن يبدأ النشر أبداً.",
    ),
    ("manpower.screen.end_reason.end_date", "حلّ تاريخ الانتهاء"),
    ("manpower.screen.end_reason.replaced", "حلّ محله عامل آخر"),
    (
        "manpower.screen.end_reason.client_rejected",
        "رفض العميل العامل",
    ),
    (
        "manpower.screen.end_reason.client_non_payment",
        "العميل لا يسدد",
    ),
    ("manpower.screen.end_reason.entered_in_error", "أُدخل بالخطأ"),
    ("manpower.screen.end_reason.contract_end", "انتهى العقد"),
    ("manpower.screen.end_reason.exit", "ترك العامل الوكالة"),
    (
        "manpower.screen.end_reason.absconded",
        "انقطع العامل عن العمل",
    ),
    ("manpower.screen.end_reason.deceased", "توفي العامل"),
    ("manpower.screen.doc_state.valid", "صالح"),
    ("manpower.screen.doc_state.expired", "منتهي الصلاحية"),
    ("manpower.screen.doc_state.missing", "غير موجود"),
    ("manpower.screen.doc_state.rejected", "مرفوض"),
    ("manpower.screen.doc_state.pending", "بانتظار المراجعة"),
    ("manpower.screen.sheet_kind.regular", "عادي"),
    ("manpower.screen.sheet_kind.adjustment", "تعديلي"),
    ("manpower.screen.sheet_status.open", "مفتوح"),
    ("manpower.screen.sheet_status.submitted", "مُرسل"),
    ("manpower.screen.sheet_status.approved", "معتمد"),
    ("manpower.screen.sheet_status.locked", "مقفل"),
    ("manpower.screen.deployments.request", "الطلب"),
    (
        "manpower.screen.deployments.request_option",
        "{{ role }} في {{ site }}، {{ client }}، {{ period }}",
    ),
    (
        "manpower.screen.deployments.plan_hint_list",
        "اختر الطلب المراد شغله والعامل. يُخطَّط النشر؛ وزر التفعيل يبدؤه.",
    ),
    ("manpower.screen.tab.timesheets", "جداول الدوام"),
    ("manpower.screen.done.deployment_planned", "خُطّط النشر."),
    ("manpower.screen.done.deployment_activated", "النشر نشط."),
    ("manpower.screen.done.deployment_extended", "مُدّد النشر."),
    ("manpower.screen.done.deployment_suspended", "النشر معلّق."),
    (
        "manpower.screen.done.deployment_resumed",
        "النشر نشط مجدداً.",
    ),
    ("manpower.screen.done.deployment_ended", "انتهى النشر."),
    ("manpower.screen.done.deployment_cancelled", "أُلغي النشر."),
    (
        "manpower.screen.deployment.end_reason_unknown",
        "اختر أحد الأسباب من القائمة.",
    ),
    ("manpower.screen.done.absence_reported", "سُجّل البلاغ."),
    ("manpower.screen.done.absence_closed", "أُغلقت الحالة."),
    ("manpower.screen.absence_status.open", "مفتوحة"),
    ("manpower.screen.absence_status.closed", "مغلقة"),
    ("manpower.screen.ticket_status.open", "مفتوحة"),
    ("manpower.screen.ticket_status.assigned", "مُسندة"),
    ("manpower.screen.ticket_status.in_progress", "قيد المعالجة"),
    ("manpower.screen.ticket_status.on_hold", "معلّقة"),
    ("manpower.screen.ticket_status.resolved", "تم الحل"),
    ("manpower.screen.ticket_status.closed", "مغلقة"),
    ("manpower.screen.ticket_status.cancelled", "ملغاة"),
    ("manpower.screen.absence.title", "حالات الغياب"),
    (
        "manpower.screen.absence.subtitle",
        "العمال الذين لم يحضروا. سجّل محاولات التواصل معهم وبلاغ وزارة الموارد البشرية والتوطين، أو أغلق الحالة عند عودة العامل.",
    ),
    ("manpower.screen.absence.case", "حالة غياب"),
    ("manpower.screen.absence.worker", "العامل"),
    ("manpower.screen.absence.client", "العميل"),
    ("manpower.screen.absence.site", "الموقع"),
    ("manpower.screen.absence.last_present", "آخر يوم حضور"),
    ("manpower.screen.absence.opened", "تاريخ الفتح"),
    ("manpower.screen.absence.days", "أيام الغياب"),
    ("manpower.screen.absence.days_column", "أيام الغياب"),
    ("manpower.screen.absence.days_absent", "{{ days }} يوماً"),
    ("manpower.screen.absence.contacts", "محاولات التواصل"),
    (
        "manpower.screen.absence.contacts_of",
        "{{ done }} من {{ need }} أيام تواصل فاشلة",
    ),
    (
        "manpower.screen.absence.report",
        "بلاغ وزارة الموارد البشرية والتوطين",
    ),
    (
        "manpower.screen.absence.report_line",
        "{{ number }}، {{ date }}",
    ),
    ("manpower.screen.absence.report_none", "غير مسجل"),
    ("manpower.screen.absence.status", "الحالة"),
    ("manpower.screen.absence.empty", "لا توجد حالات غياب"),
    (
        "manpower.screen.absence.empty_text",
        "تُفتح حالة عندما لا يحضر عامل في الموقع لأكثر من أسبوع.",
    ),
    ("manpower.screen.absence.failed", "تعذّر الوصول إليه"),
    (
        "manpower.screen.absence.contact_days",
        "أيام التواصل الفاشلة",
    ),
    ("manpower.screen.absence.contact_day", "اليوم"),
    ("manpower.screen.absence.contact_outcome", "النتيجة"),
    (
        "manpower.screen.absence.contacts_empty",
        "لم تُسجّل أي محاولة تواصل فاشلة بعد.",
    ),
    ("manpower.screen.absence.tickets", "التذاكر"),
    ("manpower.screen.absence.ticket", "التذكرة"),
    ("manpower.screen.absence.ticket_title", "العنوان"),
    (
        "manpower.screen.absence.tickets_empty",
        "لا توجد تذكرة لهذا العامل.",
    ),
    ("manpower.screen.absence.record_report", "تسجيل البلاغ"),
    (
        "manpower.screen.absence.record_report_hint",
        "رقم بلاغ الغياب المقدّم إلى وزارة الموارد البشرية والتوطين وتاريخه.",
    ),
    (
        "manpower.screen.absence.report_number",
        "رقم بلاغ وزارة الموارد البشرية والتوطين",
    ),
    ("manpower.screen.absence.report_date", "تاريخ البلاغ"),
    ("manpower.screen.absence.close", "إغلاق الحالة"),
    (
        "manpower.screen.absence.close_hint",
        "عاد العامل. يُصرف الراتب المحتجز ويُقترح في مسير رواتب خارج الدورة.",
    ),
    ("manpower.screen.absence.close_reason", "السبب"),
    (
        "manpower.screen.absence.abscond",
        "الإبلاغ عن انقطاع عن العمل",
    ),
    (
        "manpower.screen.absence.abscond_blocked",
        "يلزم يوما تواصل فاشلان ورقم بلاغ وزارة الموارد البشرية والتوطين.",
    ),
    ("manpower.screen.absence.employee_link", "فتح ملف العامل"),
    ("manpower.screen.absence.deployment_link", "فتح النشر"),
    (
        "manpower.screen.absence.closed_line",
        "أُغلقت في {{ date }} بواسطة {{ by }}: {{ reason }}.",
    ),
    (
        "manpower.screen.absence.pay_released",
        "صُرف الراتب المحتجز.",
    ),
    (
        "manpower.screen.absence.pay_proposed",
        "صُرف الراتب المحتجز واقتُرح في مسير رواتب خارج الدورة.",
    ),
    ("manpower.screen.absence.run_link", "فتح مسير الرواتب"),
    ("manpower.screen.absence.closed_title", "كيف انتهت"),
    ("manpower.screen.request_status.draft", "مسودة"),
    ("manpower.screen.request_status.open", "مُرسل"),
    (
        "manpower.screen.request_status.partially_filled",
        "مشغول جزئياً",
    ),
    ("manpower.screen.request_status.filled", "مشغول بالكامل"),
    ("manpower.screen.request_status.cancelled", "ملغى"),
    ("manpower.screen.request_status.closed", "مغلق"),
    ("manpower.portal.requests_title", "الطلبات"),
    ("manpower.portal.deployments_title", "العمال في الموقع"),
    ("manpower.portal.timesheets_title", "جداول الدوام"),
    ("manpower.screen.portal.requests.title", "الطلبات"),
    (
        "manpower.screen.portal.requests.subtitle",
        "العمال الذين طلبتهم وحالة كل طلب.",
    ),
    ("manpower.screen.portal.requests.new", "طلب جديد"),
    (
        "manpower.screen.portal.requests.new_hint",
        "اطلب عمالاً. يبقى الطلب مسودة حتى ترسله.",
    ),
    ("manpower.screen.portal.requests.request", "الطلب"),
    ("manpower.screen.portal.requests.positions", "عدد الوظائف"),
    ("manpower.screen.portal.requests.empty", "لا توجد طلبات بعد"),
    (
        "manpower.screen.portal.requests.empty_text",
        "استخدم زر طلب جديد لطلب عمال.",
    ),
    ("manpower.screen.portal.requests.submit", "إرسال"),
    (
        "manpower.screen.portal.requests.submit_hint",
        "أرسل الطلب إلى مدير حسابك.",
    ),
    ("manpower.screen.portal.requests.cancel", "إلغاء الطلب"),
    (
        "manpower.screen.portal.requests.cancel_hint",
        "يُغلق الطلب ويُحرَّر العمال المخطط لهم.",
    ),
    ("manpower.screen.portal.requests.reason", "لماذا تلغيه؟"),
    (
        "manpower.screen.portal.requests.on_hold",
        "العقد معلّق. تنتظر الطلبات الجديدة.",
    ),
    ("manpower.screen.portal.contracts", "عقودك"),
    ("manpower.screen.portal.contract", "العقد"),
    ("manpower.screen.portal.company", "الشركة"),
    ("manpower.screen.portal.period", "المدة"),
    ("manpower.screen.portal.status", "الحالة"),
    (
        "manpower.screen.portal.deployments.title",
        "العمال في الموقع",
    ),
    (
        "manpower.screen.portal.deployments.subtitle",
        "العمال المنتشرون لدى شركتك، مع مهنهم ومواقعهم وتواريخهم.",
    ),
    ("manpower.screen.portal.deployments.worker", "العامل"),
    ("manpower.screen.portal.deployments.role", "المهنة"),
    ("manpower.screen.portal.deployments.site", "الموقع"),
    ("manpower.screen.portal.deployments.shift", "الوردية"),
    (
        "manpower.screen.portal.deployments.empty",
        "لا يوجد عمال في الموقع بعد",
    ),
    (
        "manpower.screen.portal.deployments.empty_text",
        "يظهرون هنا عندما يبدأ عامل العمل في أحد مواقعك.",
    ),
    ("manpower.screen.portal.timesheets.title", "جداول الدوام"),
    (
        "manpower.screen.portal.timesheets.subtitle",
        "ساعات العمال في مواقعك، كما أُرسلت إليك للموافقة.",
    ),
    (
        "manpower.screen.portal.timesheets.waiting_one",
        "جدول دوام واحد بانتظارك",
    ),
    (
        "manpower.screen.portal.timesheets.waiting_many",
        "{{ count }} من جداول الدوام بانتظارك",
    ),
    ("manpower.screen.portal.timesheets.worker", "العامل"),
    ("manpower.screen.portal.timesheets.month", "الشهر"),
    ("manpower.screen.portal.timesheets.hours", "الساعات"),
    ("manpower.screen.portal.timesheets.work", "العمل"),
    ("manpower.screen.portal.timesheets.adjustment", "تعديل"),
    ("manpower.screen.portal.timesheets.waiting", "بانتظارك"),
    (
        "manpower.screen.portal.timesheets.empty",
        "لا توجد جداول دوام بعد",
    ),
    (
        "manpower.screen.portal.timesheets.empty_text",
        "تظهر هنا عندما ترسل إليك الوكالة ساعات شهر ما.",
    ),
    ("manpower.screen.portal.sheet.submitted", "مُرسل"),
    ("manpower.screen.portal.sheet.escalated", "مُصعّد"),
    ("manpower.screen.portal.sheet.approved", "معتمد"),
    ("manpower.screen.portal.sheet.deemed", "معتمد ضمنياً"),
];

/// The keys whose legal wording the agency reviews before launch.
/// Read by the test of `texts.rs` and by whoever prepares the review.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const REVIEW: &[&str] = &[
    "manpower.problems.deemed_days",
    "manpower.hold.held_body",
    "manpower.terminate.short_notice",
    "manpower.exit.reason.absconded",
    "manpower.absence.ticket_body",
    "manpower.absence.reminder_body",
    "manpower.end.short_notice",
    "manpower.screen.action.hold",
    "manpower.screen.contract.notice",
    "manpower.screen.contract.deemed",
    "manpower.screen.hold.title",
    "manpower.screen.form.deemed_days",
    "manpower.screen.form.deemed_days_hint",
    "manpower.screen.form.waiver",
    "manpower.screen.form.waiver_hint",
    "manpower.screen.deployment.waiver_note",
    "manpower.screen.end_reason.absconded",
    "manpower.screen.absence.report",
    "manpower.screen.absence.abscond",
    "manpower.screen.portal.sheet.deemed",
];

/// The Arabic text of `key`, if it has one.
pub(crate) fn arabic(key: &str) -> Option<&'static str> {
    TEXTS_AR
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, text)| *text)
}

/// Whether `locale` is Arabic (`ar`, `ar-AE`, `ar_AE`).
pub(crate) fn is_arabic(locale: &str) -> bool {
    let language = locale.split(['-', '_']).next().unwrap_or_default();
    language.eq_ignore_ascii_case("ar")
}
