// Tenant egress proxy binary. Configuration and decisions live in
// `everruns_server::egress_proxy`.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let config = everruns_server::egress_proxy::ProxyConfig::from_env()?;
    everruns_server::egress_proxy::run(config).await
}
