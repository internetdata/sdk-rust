//! The official Rust client library for the [InternetData](https://internetdata.io)
//! API: what every InternetData database says about an IP address, an AS or a
//! domain, and the licensed databases themselves, their metadata and their
//! checksums.
//!
//! Start with [`Client::new`] and [`Client::lookup`]. No API key is needed:
//! without one a lookup is on Unlimited, the free plan, limited per address per
//! minute. A batch needs a paid plan, and the databases a key carrying the
//! `db.download` scope, reached through [`Client::database`]. The OAuth
//! sign-in, by device flow or by authorization code, is [`Client::oauth`].
//!
//! ```no_run
//! # async fn run() -> Result<(), internetdata::Error> {
//! let client = internetdata::Client::new()?;
//!
//! let result = client.lookup("146.70.116.130").await?;
//! println!("{}", result.location.country);
//! # Ok(())
//! # }
//! ```
//!
//! # Absent is not false
//!
//! Every member a plan can leave out is an [`Option`], because your key decides
//! which of them the API sends. `None` means "not in your plan", which is a
//! different answer from `Some(false)`. Read the option itself wherever that
//! matters, or `unwrap_or(false)` when all you want to know is whether an
//! address is flagged.
//!
//! # Async only
//!
//! Everything here is `async` on tokio, and there is no blocking facade.
//! `reqwest::blocking` builds its own runtime and PANICS when constructed inside
//! one, so a facade would fail for exactly the callers most likely to reach for
//! it. A caller with no runtime writes the three lines that make the cost
//! visible:
//!
//! ```no_run
//! # fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let runtime = tokio::runtime::Runtime::new()?;
//! let client = internetdata::Client::new()?;
//! let result = runtime.block_on(client.lookup("146.70.116.130"))?;
//! # let _ = result;
//! # Ok(())
//! # }
//! ```

mod bogon;
mod bogons;
mod client;
mod database;
mod dates;
mod entitlement;
mod enums;
mod error;
mod flight;
mod generated;
mod lookup;
mod oauth;
mod objects;
mod transport;

// The OAuth unit tests reuse the integration suite's stub, which names this
// crate from outside.
#[cfg(test)]
extern crate self as internetdata;

pub use bogon::is_bogon;
pub use client::{BatchOptions, Client, ClientBuilder, DEFAULT_BASE_URL, LookupOptions};
pub use database::{DatabaseApi, DownloadOptions};
pub use entitlement::{Entitlement, EntitlementApikey, EntitlementPlan, EntitlementUsage};
pub use error::{Error, ErrorKind};
pub use lookup::Lookup;
pub use oauth::{
    AuthorizationUrlOptions, ClientRegistration, DeviceAuthorization, DeviceAuthorizationOptions,
    OauthApi, OauthError, OauthErrorResponse, OauthMetadata, OauthOptions, Pkce, RegisterOptions,
    TokenResponse,
};

use generated::models;

// The wire types, re-exported from the generated models so a consumer never
// names the module path.
pub use models::database::LicenseType;
pub use models::download::Outcome;
pub use models::{
    AbuseContact, Asn, BitTorrentDetail, CarrierDetail, ClassDetail, Company, DnsResolverDetail,
    HostedDomains, Location, LookupResponse, ProxyDetail, PubproxyDetail, Rwhois, SatelliteDetail,
    SmtpDetail, SshDetail, WebDetail, Whois,
};
pub use models::{AsnLookupResponse, AsnPrefix};
pub use models::{
    Checksums, ChecksumsResponse, Database, DatabaseMetadata, DatabaseMetadataColumn,
    DatabaseVersion, Download,
};
pub use models::{DatabaseFormat, Standing};
pub use models::{
    DisposableDetail, DnsAddressRecord, DnsCnameRecord, DnsMxRecord, DnsNsRecord, DomainDns,
    DomainIpRange, DomainIpRanges, DomainLookupResponse,
};
