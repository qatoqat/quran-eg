//! Blocking HTTP GET for the API and the font files.
//!
//! IPv4 is tried first, then any address family. `ureq` connects to a host's
//! addresses in the order the resolver gives them, which is IPv6 first — and on
//! a network that announces IPv6 without routing it, every request would wait
//! out the connect timeout once per IPv6 address before reaching IPv4.

use std::time::Duration;

use ureq::config::IpFamily;

/// Largest response body accepted.
const MAX_BODY: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Http {
    v4: ureq::Agent,
    any: ureq::Agent,
}

impl Http {
    /// Every request has deadlines, so a hung connection turns into a visible
    /// error (with retry) instead of an endless "Loading…".
    pub fn new(connect: Duration, total: Duration) -> Self {
        let agent = |family| -> ureq::Agent {
            ureq::Agent::config_builder()
                .timeout_connect(Some(connect))
                .timeout_global(Some(total))
                // A non-2xx status comes back as a response, so its body can be reported.
                .http_status_as_error(false)
                .ip_family(family)
                .build()
                .into()
        };
        Self { v4: agent(IpFamily::Ipv4Only), any: agent(IpFamily::Any) }
    }

    /// GET `url`; returns the status and the body.
    pub fn get(&self, url: &str, accept: &str) -> Result<(u16, Vec<u8>), Box<ureq::Error>> {
        let fetch = |agent: &ureq::Agent| -> Result<(u16, Vec<u8>), Box<ureq::Error>> {
            let mut response = agent.get(url).header("Accept", accept).call().map_err(Box::new)?;
            let body = response.body_mut().with_config().limit(MAX_BODY).read_to_vec().map_err(Box::new)?;
            Ok((response.status().as_u16(), body))
        };
        fetch(&self.v4).or_else(|_| fetch(&self.any))
    }
}
