#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

mod model;
mod texts;

use renox::db::Migration;
use renox::prelude::*;
use serde::Serialize;

pub use model::Entry;

/// The templates, compiled in. An app replaces one with a file of the same
/// name under its views directory (`resources/views/property/index.html`).
const VIEWS: &[(&str, &str)] = &[(
    "property/index.html",
    include_str!("../views/property/index.html"),
)];

/// The Property module. Add it to the app with `.module(property::Property)`.
pub struct Property;

/// What the page needs; `view:check` compares it with the template.
#[derive(Serialize, ViewData)]
#[view(template = "property/index.html")]
struct IndexPage {
    heading: String,
    empty: String,
    entries: Vec<Entry>,
}

impl Module for Property {
    fn name(&self) -> &'static str {
        "property"
    }

    // Public on purpose: add `.require_auth()` to protect the page.
    fn routes(&self) -> Routes {
        Routes::new().get("/property", index).name("property.index")
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.model::<Entry>().view_data::<IndexPage>();
        app.templates(|env| {
            for (name, source) in VIEWS {
                // The app's own file of that name wins.
                if env.get_template(name).is_err() {
                    renox::view::add_template(env, *name, *source)
                        .expect("a built-in template compiles");
                }
            }
        });
    }
}

async fn index(State(db): State<Db>, lang: Lang) -> Result<View> {
    let entries = Entry::query().latest().get(&db).await?;
    Ok(View::typed(IndexPage {
        heading: texts::t(&lang, "property.title"),
        empty: texts::t(&lang, "property.empty"),
        entries,
    }))
}
