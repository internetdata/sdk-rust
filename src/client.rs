use std::sync::Arc;
use std::time::Duration;

use crate::error::Error;
use crate::transport::Transport;

/// The production API. Override it with [`ClientBuilder::base_url`].
pub const DEFAULT_BASE_URL: &str = "https://internetdata.io";

const DEFAULT_RETRIES: u32 = 2;
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(30);
const RETRY_BASE_DELAY: Duration = Duration::from_millis(250);

/// A client for the InternetData API.
///
/// Cloning is cheap and shares the connection pool, so a clone per task is the
/// intended way to use it across one.
///
/// There is no answer cache here, deliberately. This API serves whole database
/// files, and the one thing worth not fetching twice is a multi-gigabyte
/// download that is already on your disk; a per-process cache in front of it
/// would hold the file in memory and still not survive a restart. Poll
/// [`Client::metadata`] for `updated` and skip the transfer yourself.
#[derive(Debug, Clone)]
pub struct Client(Arc<Inner>);

#[derive(Debug)]
struct Inner {
    transport: Transport,
    retries: u32,
}

impl Client {
    /// A client against production. Every endpoint needs a key, so this is only
    /// useful with [`ClientBuilder::api_key`]; it exists so a caller reading a
    /// key out of the environment can build in one line.
    pub fn new() -> Result<Self, Error> {
        Self::builder().build()
    }

    pub fn builder() -> ClientBuilder {
        ClientBuilder::default()
    }

    pub(crate) fn transport(&self) -> &Transport {
        &self.0.transport
    }

    pub(crate) fn retries(&self) -> u32 {
        self.0.retries
    }
}

/// Builds a [`Client`]. With nothing set it talks to production with no key.
#[derive(Debug, Default)]
pub struct ClientBuilder {
    api_key: Option<String>,
    base_url: Option<String>,
    retries: Option<u32>,
    http_client: Option<reqwest::Client>,
}

impl ClientBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Authenticates as the key's organization, which is what decides both the
    /// catalog you can see and the databases you may download. The key needs the
    /// `db.download` scope; keys are default-deny, so an existing key does not
    /// gain database access until that scope is added to it.
    pub fn api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }

    /// Points the client at a different deployment of the API.
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = Some(url.into());
        self
    }

    /// How many further attempts a transient failure gets. Default 2.
    pub fn retries(mut self, n: u32) -> Self {
        self.retries = Some(n);
        self
    }

    /// The HTTP client to send with, for a custom transport, proxy or timeout.
    ///
    /// **Build it with [`reqwest::redirect::Policy::none`].** reqwest follows
    /// redirects by default and its policy is a client-level setting with no
    /// per-request override, so a following client would chase the download
    /// endpoint's 302 into object storage instead of handing back the link.
    /// [`Client::download_url`] refuses rather than downloading, but only after
    /// the request has been spent.
    ///
    /// **And do not put a total [`reqwest::ClientBuilder::timeout`] on it.**
    /// That deadline covers the response BODY, so it caps how large a database
    /// this client can fetch: at 30 seconds a 5 GiB transfer cannot finish
    /// however healthy the connection is. The default client sets a connect
    /// timeout and a read (inactivity) timeout instead, which fail a stalled
    /// transfer without failing a slow one.
    pub fn http_client(mut self, client: reqwest::Client) -> Self {
        self.http_client = Some(client);
        self
    }

    pub fn build(self) -> Result<Client, Error> {
        let base_url = self.base_url.unwrap_or_else(|| DEFAULT_BASE_URL.to_owned());
        let parsed = reqwest::Url::parse(&base_url)
            .map_err(|e| Error::Config(format!("base url {base_url:?}: {e}")))?;
        if !parsed.has_host() {
            return Err(Error::Config(format!("base url {base_url:?} needs a scheme and a host")));
        }

        let http = match self.http_client {
            Some(client) => client,
            None => reqwest::Client::builder()
                .user_agent(concat!("internetdata-rust/", env!("CARGO_PKG_VERSION")))
                // A total request timeout would cap the SIZE of a database this
                // client can fetch, because it covers the body as well as the
                // response head. These two fail a connection that will not open
                // and a transfer that has stopped moving, and leave a healthy
                // multi-gigabyte download alone.
                .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
                .read_timeout(DEFAULT_READ_TIMEOUT)
                // The download endpoint answers 302 to object storage and the
                // database behind it reaches gigabytes, so the link is the
                // answer and following it is never what a caller wants.
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
        };

        Ok(Client(Arc::new(Inner {
            transport: Transport::new(
                http,
                base_url.trim_end_matches('/').to_owned(),
                self.api_key,
            ),
            retries: self.retries.unwrap_or(DEFAULT_RETRIES),
        })))
    }
}

/// Backs off exponentially, except that a server-supplied `Retry-After` wins
/// over the schedule. A 429 WITHOUT that header is a spent allowance rather than
/// a throttle and is not retried at all, which [`Error::retryable`] decides.
pub(crate) async fn with_retry<T, F, Fut>(retries: u32, mut attempt: F) -> Result<T, Error>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, Error>>,
{
    let mut delay = RETRY_BASE_DELAY;
    let mut remaining = retries;
    loop {
        match attempt().await {
            Ok(value) => return Ok(value),
            Err(err) if remaining == 0 || !err.retryable() => return Err(err),
            Err(err) => {
                tokio::time::sleep(err.retry_after().unwrap_or(delay)).await;
                delay *= 2;
                remaining -= 1;
            }
        }
    }
}
