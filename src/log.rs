use tracing_subscriber::EnvFilter;

pub fn setup_log() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let fmt = tracing_subscriber::fmt().with_env_filter(filter);

    if std::env::var("RUST_LOG_NO_TIMESTAMP").is_ok() {
        fmt.without_time().init();
    } else {
        fmt.init();
    }
}
