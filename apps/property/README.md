# property

Property: a reusable Renox module, made with `rnx make:package property`.

Add it to an app (`Cargo.toml`: `property = { path = "…" }`, or its version once
published) and register the module:

```
use renox::prelude::*;

# let _ =
App::new().module(property::Property)
# ;
```

The page is at `/property`. The module owns:

- `migrations/`, run with the app's (`rnx migrate`); their names carry the package
  name, so two packages can't clash.
- `views/property/`, compiled in. The app overrides a page by putting a file of the
  same name under its views directory (`resources/views/property/index.html`).
- `src/texts.rs`, the English texts (`property.*`). The app changes or translates them
  in `resources/lang/{locale}.json`.
