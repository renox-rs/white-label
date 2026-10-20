fn main() -> renox::Result {
    match host::try_app() {
        Ok(app) => {
            // `.env` is read by now (`try_app`), so APP_ENV is what the app runs with.
            let command = std::env::args().nth(1);
            let env = std::env::var("APP_ENV").ok();
            if let Err(message) = host::refuse_in_production(command.as_deref(), env.as_deref()) {
                eprintln!("{message}");
                std::process::exit(1);
            }
            app.run()
        }
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    }
}
