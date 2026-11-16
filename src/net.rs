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
    read_body(url, &mut resp)
}

fn read_body(url: &str, resp: &mut ureq::http::Response<ureq::Body>) -> Result<Vec<u8>> {
    resp.body_mut()
        .with_config()
        .limit(200 * 1024 * 1024)
        .read_to_vec()
        .with_context(|| format!("reading body of {url}"))
}

pub enum Fetched {
    NotModified,
    Body {
        bytes: Vec<u8>,
        etag: Option<String>,
    },
}

/// GET with `If-None-Match` when an ETag from an earlier response is known.
pub fn get_conditional(url: &str, etag: Option<&str>) -> Result<Fetched> {
    let mut req = agent()?.get(url);
    if let Some(etag) = etag {
        req = req.header("If-None-Match", etag);
    }
    let mut resp = req.call().with_context(|| format!("GET {url}"))?;
    if resp.status() == ureq::http::StatusCode::NOT_MODIFIED {
        return Ok(Fetched::NotModified);
    }
    let etag = resp
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let bytes = read_body(url, &mut resp)?;
    Ok(Fetched::Body { bytes, etag })
}
