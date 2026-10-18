# wl-kernel

Wl Kernel: a reusable Renox module, made with `rnx make:package wl-kernel`.

Add it to an app (`Cargo.toml`: `wl-kernel = { path = "…" }`, or its version once
published) and register the module:

```
use renox::prelude::*;

# let _ =
App::new().module(wl_kernel::WlKernel)
# ;
```

The page is at `/wl-kernel`. The module owns:

- `migrations/`, run with the app's (`rnx migrate`); their names carry the package
  name, so two packages can't clash.
- `views/wl-kernel/`, compiled in. The app overrides a page by putting a file of the
  same name under its views directory (`resources/views/wl-kernel/index.html`).
- `src/texts.rs`, the English texts (`wl_kernel.*`). The app changes or translates them
  in `resources/lang/{locale}.json`.
