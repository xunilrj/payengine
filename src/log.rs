use tracing_subscriber::EnvFilter;

pub fn setup_log() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    // Logs go to stderr: stdout is reserved for the output CSV.
    let fmt = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr);

    if std::env::var("RUST_LOG_NO_TIMESTAMP").is_ok() {
        fmt.without_time().init();
    } else {
        fmt.init();
    }
}
