//! Rendering a message's content from its type's template (wl-notify §3
//! *Template*): the text for the channel and language, falling back to
//! English, with `{{ name }}` filled from the request data by the kernel's
//! [`fill`].

use renox::serde_json::Value;
use wl_kernel::notify::{NotificationTypeDef, NotifyChannel, fill};

/// A message's rendered content.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Rendered {
    /// The subject (e-mail) or title, if the template has one.
    pub subject: Option<String>,
    /// The body.
    pub body: String,
}

impl Rendered {
    /// The title to show where one is needed (the bell): the subject, else
    /// the type key.
    pub fn title_or<'a>(&'a self, key: &'a str) -> &'a str {
        self.subject.as_deref().unwrap_or(key)
    }
}

/// The content of a `def` message on `channel` in `language` with `data`.
/// With no template at all, there is no subject and the body is empty.
pub fn render(
    def: &NotificationTypeDef,
    channel: NotifyChannel,
    language: &str,
    data: &Value,
) -> Rendered {
    match def.template_for(channel, language) {
        Some(template) => Rendered {
            subject: template.subject.as_deref().map(|s| fill(s, data)),
            body: fill(&template.body, data),
        },
        None => Rendered::default(),
    }
}

#[cfg(test)]
mod tests {
    use renox::serde_json::json;

    use super::*;

    fn reminder() -> NotificationTypeDef {
        NotificationTypeDef::new("invoice.reminder", "invoicing")
            .channels(&[NotifyChannel::Email, NotifyChannel::Sms])
            .template(
                NotifyChannel::Email,
                "en",
                Some("Invoice {{ number }} is overdue"),
                "Please pay invoice {{ number }}.",
            )
            .template(
                NotifyChannel::Email,
                "ar",
                Some("الفاتورة {{ number }} متأخرة"),
                "يرجى دفع الفاتورة {{ number }}.",
            )
            .template(
                NotifyChannel::Sms,
                "en",
                None,
                "Invoice {{ number }} overdue",
            )
    }

    #[test]
    fn the_template_of_the_channel_and_language_is_filled() {
        let data = json!({ "number": "INV-0001" });
        let email = render(&reminder(), NotifyChannel::Email, "ar", &data);
        assert_eq!(email.subject.as_deref(), Some("الفاتورة INV-0001 متأخرة"));
        let sms = render(&reminder(), NotifyChannel::Sms, "ar", &data);
        assert_eq!(sms.subject, None);
        assert_eq!(sms.body, "Invoice INV-0001 overdue");
        assert_eq!(sms.title_or("invoice.reminder"), "invoice.reminder");
    }

    #[test]
    fn a_type_without_templates_renders_empty() {
        let bare = NotificationTypeDef::new("lease.signed", "property");
        let rendered = render(&bare, NotifyChannel::Email, "en", &json!({}));
        assert_eq!(rendered, Rendered::default());
    }
}
