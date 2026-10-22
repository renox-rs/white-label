//! The texts of the outlet outage, the channels and the paper orders
//! (BL-FNB-14): keys `fnb.outage.*` and `fnb.paper.*` (toasts, refusals, the
//! notices and the task) and `fnb.screen.channels.*` / `fnb.screen.ordering.*`
//! (what a template reads as `tx['channels...']`).

/// Every text of the outages and the paper orders.
pub(crate) const OUTAGE_TEXTS: &[(&str, &str)] = &[
    // The manual fallback of the kitchen devices connection.
    (
        "fnb.outage.fallback",
        "Work from the aggregators' printouts and paper tickets, ring counter sales on paper, and pause the aggregators on their tablets. Enter the paper orders when the outlet is back.",
    ),
    // Toasts.
    ("fnb.outage.paused", "The outlet's channels are paused."),
    ("fnb.outage.resumed", "The channels are open again."),
    ("fnb.outage.channel_paused", "The channel is marked paused."),
    ("fnb.outage.channel_resumed", "The channel is open again."),
    ("fnb.paper.recorded", "Paper order recorded."),
    (
        "fnb.paper.recorded_late",
        "Paper order recorded with the late-entry flag and its business time.",
    ),
    (
        "fnb.paper.pending",
        "The paper order waits for the area manager: it goes into a closing that was already submitted.",
    ),
    // History.
    (
        "fnb.outage.history.devices_down",
        "Kitchen devices went silent",
    ),
    (
        "fnb.outage.history.paused_by_hand",
        "Channels paused by a manager",
    ),
    (
        "fnb.outage.history.devices_back",
        "Kitchen devices answer again",
    ),
    (
        "fnb.outage.history.silent_again",
        "Kitchen devices went silent again",
    ),
    (
        "fnb.outage.history.resumed",
        "Channels resumed by a manager",
    ),
    // Refusals.
    (
        "fnb.outage.problem.reason",
        "Give the reason for pausing the channels.",
    ),
    (
        "fnb.outage.problem.already_paused",
        "This outlet's channels are paused already.",
    ),
    ("fnb.paper.problem.day", "Give the day the order happened."),
    (
        "fnb.paper.problem.time",
        "Give the time the order happened, like 20:20.",
    ),
    (
        "fnb.paper.problem.identity",
        "Give the channel and the number on the paper ticket.",
    ),
    (
        "fnb.paper.problem.amount",
        "The amount of an order cannot be negative.",
    ),
    (
        "fnb.paper.problem.method",
        "Choose cash, card or aggregator.",
    ),
    (
        "fnb.paper.problem.locked",
        "The closing of {{ date }} was approved by the area manager and is not reopened. The area manager is asked to record the order in the next day's closing.",
    ),
    (
        "fnb.paper.outage_reference",
        "Outage {{ id }} of {{ outlet }}",
    ),
    // The area manager's task for an entry into an approved closing.
    (
        "fnb.paper.task_title",
        "Record a late order of {{ outlet }} ({{ date }}) in the closing of {{ next }}",
    ),
    (
        "fnb.paper.task_body",
        "A paper order {{ reference }} of {{ amount }} with business time on {{ date }} was entered at {{ outlet }}, but the closing of {{ date }} is approved and cannot take it. Record it as an adjustment in the closing of {{ next }}. {{ link }}",
    ),
    // The notice to the managers when the channels are paused.
    (
        "fnb.outage.paused_title",
        "{{ outlet }}: kitchen devices are silent, channels paused",
    ),
    (
        "fnb.outage.paused_body",
        "The kitchen devices of {{ outlet }} stopped answering. {{ brands }} show temporarily closed on the ordering site. Pause the aggregators on their tablets; the kitchen works on paper. The channels stay paused until you resume them.",
    ),
    // The task for the kitchen manager.
    (
        "fnb.outage.task_title",
        "{{ outlet }}: pause the aggregators, then resume the channels when the outlet is back",
    ),
    (
        "fnb.outage.task_body",
        "{{ outlet }} lost its devices. {{ brands }} are paused on the ordering site. Pause the aggregators on their tablets, work on paper, and when the devices answer enter the paper orders as late entries and resume the channels. {{ link }}",
    ),
    ("fnb.outage.task_done", "The channels were resumed."),
    // The prompt to resume.
    (
        "fnb.outage.back_title",
        "{{ outlet }} is back: resume the channels",
    ),
    (
        "fnb.outage.back_body",
        "The kitchen devices of {{ outlet }} answer again. Enter the paper orders as late entries with their business time, then resume the channels. They stay paused until you do; you are asked again every 15 minutes.",
    ),
    // The screens: the outlet channels page.
    ("fnb.screen.channels.title", "Outlet channels"),
    (
        "fnb.screen.channels.subtitle",
        "The brands an outlet takes orders for on the ordering site and the aggregators' tablets, and what an outage paused.",
    ),
    ("fnb.screen.channels.outlet", "Outlet"),
    ("fnb.screen.channels.channel", "Channel"),
    ("fnb.screen.channels.brand", "Brand"),
    ("fnb.screen.channels.kind", "Kind"),
    ("fnb.screen.channels.kind.d2c", "Ordering site"),
    ("fnb.screen.channels.kind.aggregator", "Aggregator tablet"),
    ("fnb.screen.channels.open", "Taking orders"),
    ("fnb.screen.channels.paused", "Temporarily closed"),
    ("fnb.screen.channels.since", "Since"),
    ("fnb.screen.channels.empty", "No channels yet"),
    (
        "fnb.screen.channels.empty_text",
        "The channels of an outlet are the brands it cooks and the aggregators it is on.",
    ),
    ("fnb.screen.channels.pause", "Pause"),
    ("fnb.screen.channels.pause_title", "Mark the channel paused"),
    (
        "fnb.screen.channels.pause_message",
        "Mark this channel paused. For an aggregator, pause it on its tablet as well.",
    ),
    ("fnb.screen.channels.resume", "Resume"),
    ("fnb.screen.channels.resume_title", "Open the channel again"),
    (
        "fnb.screen.channels.resume_message",
        "Open this channel again?",
    ),
    ("fnb.screen.channels.outage", "Outage"),
    ("fnb.screen.channels.outage_since", "Silent since"),
    (
        "fnb.screen.channels.devices_back_at",
        "Devices answer again at",
    ),
    ("fnb.screen.channels.prompts", "Times asked to resume"),
    ("fnb.screen.channels.pause_all", "Pause all channels"),
    (
        "fnb.screen.channels.pause_all_title",
        "Pause the outlet's channels",
    ),
    (
        "fnb.screen.channels.pause_all_description",
        "The brands show temporarily closed on the ordering site and take no orders here. Nothing resumes them but a manager.",
    ),
    ("fnb.screen.channels.reason", "Reason"),
    ("fnb.screen.channels.resume_all", "Resume the channels"),
    (
        "fnb.screen.channels.resume_all_title",
        "Resume the outlet's channels",
    ),
    (
        "fnb.screen.channels.resume_all_message",
        "Open every channel the outage paused? Enter the paper orders first.",
    ),
    (
        "fnb.screen.channels.banner_down",
        "The kitchen devices are silent: the brands of this outlet show temporarily closed on the ordering site. Pause the aggregators on their tablets and work on paper. The channels stay paused until you resume them.",
    ),
    (
        "fnb.screen.channels.banner_back",
        "The kitchen devices answer again. Enter the paper orders as late entries, then resume the channels. They stay paused until you do.",
    ),
    (
        "fnb.screen.channels.banner_manual",
        "The channels were paused by a manager.",
    ),
    (
        "fnb.screen.channels.no_outage",
        "No outage: the channels are open.",
    ),
    (
        "fnb.screen.channels.fallback",
        "While the devices are silent",
    ),
    ("fnb.screen.channels.history", "Outages"),
    ("fnb.screen.channels.source", "How"),
    ("fnb.screen.channels.source.devices", "Devices silent"),
    ("fnb.screen.channels.source.manual", "Paused by a manager"),
    ("fnb.screen.channels.resumed_at", "Resumed at"),
    ("fnb.screen.channels.status.paused", "Paused"),
    ("fnb.screen.channels.status.devices_back", "Devices back"),
    ("fnb.screen.channels.status.resumed", "Resumed"),
    // The paper orders.
    ("fnb.screen.channels.paper", "Paper orders"),
    ("fnb.screen.channels.paper_add", "Enter a paper order"),
    ("fnb.screen.channels.paper_title", "Enter a paper order"),
    (
        "fnb.screen.channels.paper_description",
        "Enter an order written on paper during the outage with the time it really happened. An entry more than 30 minutes late carries the late-entry flag and needs a reason; into a submitted closing it waits for the area manager.",
    ),
    (
        "fnb.screen.channels.paper_reference",
        "Number on the paper ticket",
    ),
    ("fnb.screen.channels.paper_channel", "Channel"),
    ("fnb.screen.channels.paper_amount", "Amount with VAT (AED)"),
    ("fnb.screen.channels.paper_method", "Paid by"),
    ("fnb.screen.channels.paper_day", "Day it happened"),
    ("fnb.screen.channels.paper_time", "Time it happened"),
    (
        "fnb.screen.channels.paper_time_hint",
        "Like 20:20, Dubai time.",
    ),
    ("fnb.screen.channels.paper_reason", "Why it is late"),
    ("fnb.screen.channels.paper_outage", "Outage reference"),
    ("fnb.screen.channels.paper_flag", "Late entry"),
    ("fnb.screen.channels.paper_business_time", "Business time"),
    ("fnb.screen.channels.paper_entered", "Entered"),
    ("fnb.screen.channels.paper_empty", "No paper orders"),
    ("fnb.screen.channels.paper_method.cash", "Cash"),
    ("fnb.screen.channels.paper_method.card", "Card"),
    ("fnb.screen.channels.paper_method.aggregator", "Aggregator"),
    ("fnb.screen.channels.paper_channel.counter", "Counter"),
    ("fnb.screen.channels.paper_channel.phone", "Phone"),
    ("fnb.screen.channels.paper_channel.website", "Ordering site"),
    (
        "fnb.screen.channels.paper_channel.aggregator_a",
        "Aggregator A",
    ),
    (
        "fnb.screen.channels.paper_channel.aggregator_b",
        "Aggregator B",
    ),
    (
        "fnb.screen.channels.paper_channel.aggregator_c",
        "Aggregator C",
    ),
    ("fnb.screen.channels.yes", "Late"),
    // The ordering page of a brand.
    ("fnb.screen.ordering.title", "Order from"),
    (
        "fnb.screen.ordering.subtitle",
        "Choose the kitchen that cooks for you. A kitchen that is temporarily closed takes no orders.",
    ),
    ("fnb.screen.ordering.open", "Taking orders"),
    ("fnb.screen.ordering.closed", "Temporarily closed"),
    (
        "fnb.screen.ordering.closed_text",
        "This kitchen cannot take orders right now. Please try again later.",
    ),
    (
        "fnb.screen.ordering.empty",
        "No kitchen cooks this brand yet",
    ),
    ("fnb.screen.ordering.kitchen", "Kitchen"),
    ("fnb.screen.ordering.state", "Now"),
];
