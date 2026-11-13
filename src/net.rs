use std::sync::{Arc, OnceLock};

use anyhow::{Context, Result};
use ureq::tls::{Certificate, PemItem, RootCerts, TlsConfig};

const USER_AGENT: &str = concat!(
    "cargo-project-tools/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/osseco/cargo-project-tools)"
);

/// Shared HTTP agent. Follows `HTTPS_PROXY`/`NO_PROXY` and, when `SSL_CERT_FILE` is set
/// (as sfw does), trusts exactly the certificates in that bundle.
pub fn agent() -> Result<&'static ureq::Agent> {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    if let Some(agent) = AGENT.get() {
        return Ok(agent);
    }
    let mut tls = TlsConfig::builder();
    if let Some(path) = std::env::var_os("SSL_CERT_FILE") {
        let pem = std::fs::read(&path)
            .with_context(|| format!("reading SSL_CERT_FILE {}", path.to_string_lossy()))?;
        let certs: Vec<Certificate<'static>> = ureq::tls::parse_pem(&pem)
            .filter_map(|item| match item {
                Ok(PemItem::Certificate(c)) => Some(c.to_owned()),
                _ => None,
            })
            .collect();
        anyhow::ensure!(
            !certs.is_empty(),
            "no certificates found in SSL_CERT_FILE {}",
            path.to_string_lossy()
        );
        tls = tls.root_certs(RootCerts::Specific(Arc::new(certs)));
    }
    let agent = ureq::Agent::config_builder()
        .user_agent(USER_AGENT)
        .proxy(ureq::Proxy::try_from_env())
        .tls_config(tls.build())
        .build()
        .new_agent();
    Ok(AGENT.get_or_init(|| agent))
}

pub fn get_bytes(url: &str) -> Result<Vec<u8>> {
    let mut resp = agent()?
        .get(url)
        .call()
        .with_context(|| format!("GET {url}"))?;
    resp.body_mut()
        .with_config()
        .limit(200 * 1024 * 1024)
        .read_to_vec()
        .with_context(|| format!("reading body of {url}"))
}
