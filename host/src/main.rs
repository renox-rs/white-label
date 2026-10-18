fn main() -> renox::Result {
    match host::try_app() {
        Ok(app) => app.run(),
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    }
}
