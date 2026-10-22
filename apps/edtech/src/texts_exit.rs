//! The texts of the money documents of a withdrawal and of the batch `cancel`
//! and `terminate` (BL-EDU-03, BL-EDU-13, BL-EDU-12), in English. They join the
//! app's other texts in [`crate::texts`].

/// Every key of these parts, with its English text.
pub(crate) const TEXTS: &[(&str, &str)] = &[
    // The withdrawal.
    (
        "edtech.enrolments.withdrawal_sent",
        "The withdrawal waits for another user to approve the refund.",
    ),
    (
        "edtech.enrolments.pending_approval",
        "A withdrawal of this enrolment waits for approval. The enrolment stays as it is until another user decides the refund.",
    ),
    ("edtech.enrolments.settlement", "Money"),
    ("edtech.enrolments.settled", "Settled"),
    ("edtech.enrolments.settling", "Being settled"),
    ("edtech.enrolments.not_settled", "Not finished:"),
    ("edtech.enrolments.credit_notes", "Credit notes"),
    ("edtech.enrolments.refunds_asked", "Refunds asked"),
    (
        "edtech.enrolments.owed_invoice",
        "Invoice for the amount owed",
    ),
    ("edtech.enrolments.settle", "Finish the money steps"),
    (
        "edtech.enrolments.settle_message",
        "Repeat the steps of the withdrawal that did not finish: the credit note, the refund request and the invoice for the amount owed.",
    ),
    (
        "edtech.enrolments.settled_now",
        "The money of the withdrawal is settled.",
    ),
    (
        "edtech.fees.owed",
        "Sessions used and fees owed for :enrolment",
    ),
    (
        "edtech.settle.credit_reason",
        "Withdrawal of :enrolment: :reason",
    ),
    (
        "edtech.settle.unpaid_reason",
        "Withdrawal of :enrolment: nothing was paid on this invoice",
    ),
    // Cancelling and terminating a batch.
    (
        "edtech.batches.problem.cancel_status",
        "Only a planned or an open batch can be cancelled; a batch that is running is terminated instead.",
    ),
    (
        "edtech.batches.problem.cancel_reason",
        "Give the reason the batch is cancelled.",
    ),
    (
        "edtech.batches.problem.terminate_status",
        "Only a running batch can be terminated; a planned or an open batch is cancelled instead.",
    ),
    (
        "edtech.batches.problem.terminate_reason",
        "Pick the reason the batch is terminated: the tutor is unavailable, the approval expired or was revoked, the venue, or too few students.",
    ),
    (
        "edtech.batches.termination.tutor_unavailable",
        "The tutor is unavailable",
    ),
    (
        "edtech.batches.termination.approval_expired",
        "The approval expired",
    ),
    (
        "edtech.batches.termination.approval_revoked",
        "The approval was revoked",
    ),
    ("edtech.batches.termination.venue", "The venue is lost"),
    (
        "edtech.batches.termination.too_few_students",
        "Too few students",
    ),
    (
        "edtech.exit.problem.choices",
        "Say for every enrolment whether the student moves to another batch or is withdrawn before the batch ends.",
    ),
    (
        "edtech.exit.problem.approval",
        "Terminating a running batch needs the approval of the centre manager, and no approval is set up to ask for it.",
    ),
    (
        "edtech.exit.problem.not_in_batch",
        "Enrolment :enrolment does not hold a seat in this batch.",
    ),
    (
        "edtech.exit.problem.twice",
        "Enrolment :enrolment is named twice; give one choice for it.",
    ),
    (
        "edtech.exit.problem.unpaid_transfer",
        "Enrolment :enrolment has not paid yet and cannot be moved; withdraw it.",
    ),
    (
        "edtech.exit.problem.target",
        "Enrolment :enrolment cannot move to batch :batch: it must be another open or running batch of the same course.",
    ),
    ("edtech.exit.ended_cancel", "cancelled"),
    ("edtech.exit.ended_terminate", "stopped"),
    ("edtech.exit.family_title", "Batch :batch was :ended"),
    (
        "edtech.exit.family_body",
        "The centre :ended batch :batch: :reason. :what",
    ),
    (
        "edtech.exit.family_moved",
        "You were moved to batch :target and keep what you paid for the sessions not yet used.",
    ),
    (
        "edtech.exit.family_refund",
        "Your enrolment is withdrawn and a refund of :amount is on its way once it is approved.",
    ),
    (
        "edtech.exit.family_nothing",
        "Your enrolment is withdrawn; nothing is left to refund.",
    ),
    (
        "edtech.exit.fallback_title",
        "Students of :batch could not be moved",
    ),
    (
        "edtech.exit.fallback_body",
        "The seat in the other batch was gone, so these enrolments were withdrawn instead and refunded: :enrolments.",
    ),
    (
        "edtech.exit.regulatory_title",
        "Batch :batch stopped for a regulatory reason",
    ),
    (
        "edtech.exit.regulatory_body",
        "Batch :batch was terminated: :reason. The learners are being settled: moved to another batch or refunded for the sessions not taught.",
    ),
    ("edtech.batches.cancel", "Cancel the batch"),
    (
        "edtech.batches.cancel_description",
        "The batch will not run. Say for each student whether they move to another batch or are withdrawn with everything paid refunded, the registration fee included. Each refund waits for its own approval.",
    ),
    ("edtech.batches.terminate", "Terminate the batch"),
    (
        "edtech.batches.terminate_description",
        "Stop a running batch. The centre manager approves it; then the remaining sessions are cancelled and each student moves to another batch or is withdrawn with the unused sessions refunded pro rata, without admin fee.",
    ),
    ("edtech.batches.reason", "Reason"),
    ("edtech.batches.termination_note", "Note (optional)"),
    (
        "edtech.batches.cancelled",
        "Batch cancelled; the students were moved or withdrawn.",
    ),
    (
        "edtech.batches.termination_sent",
        "The termination waits for the centre manager's approval.",
    ),
    ("edtech.batches.terminated", "Batch terminated."),
    (
        "edtech.batches.pending_approval",
        "The termination of this batch waits for the centre manager. The sessions go on until it is approved.",
    ),
    ("edtech.exit.withdraw", "Withdraw with a refund"),
    ("edtech.exit.transfer_to", "Move to"),
];
