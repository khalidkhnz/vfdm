use crate::types::DownloadRequest;
use reqwest::header::{ACCEPT_ENCODING, COOKIE, REFERER, USER_AGENT};
use reqwest::{redirect::Policy, Client, RequestBuilder};
use std::time::Duration;

pub const DEFAULT_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36 vfdm/0.1";

/// HTTP/1.1 only on purpose: h2 would multiplex every segment onto one TCP
/// connection, which defeats the point of segmenting.
pub fn build_client() -> Client {
    Client::builder()
        .redirect(Policy::limited(10))
        .connect_timeout(Duration::from_secs(15))
        .pool_max_idle_per_host(32)
        .tcp_keepalive(Duration::from_secs(30))
        .user_agent(DEFAULT_UA)
        .build()
        .expect("reqwest client")
}

pub fn apply_headers(mut rb: RequestBuilder, req: &DownloadRequest) -> RequestBuilder {
    rb = rb.header(ACCEPT_ENCODING, "identity");
    if let Some(r) = &req.referrer {
        rb = rb.header(REFERER, r);
    }
    if let Some(ua) = &req.user_agent {
        rb = rb.header(USER_AGENT, ua);
    }
    if let Some(c) = &req.cookies {
        if !c.is_empty() {
            rb = rb.header(COOKIE, c);
        }
    }
    for (k, v) in &req.headers {
        rb = rb.header(k, v);
    }
    rb
}
