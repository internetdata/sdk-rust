//! The official Rust client library for the [InternetData](https://internetdata.io)
//! API: the licensed IP intelligence databases, their metadata and their
//! checksums.
//!
//! Start with [`Client::builder`] and an API key carrying the `db.download`
//! scope. Every database published today is licensed, so a keyless client is
//! answered `401` - the key is nevertheless OPTIONAL, and a client built without
//! one sends no `Authorization` header at all. There is no per-address lookup
//! here: the database catalog and the files behind it are reached through
//! [`Client::database`], and the OAuth device-flow sign-in through
//! [`Client::oauth`].
//!
//! ```no_run
//! # async fn run() -> Result<(), internetdata::Error> {
//! let client = internetdata::Client::builder().api_key("your-api-key").build()?;
//!
//! for database in client.database().list().await? {
//!     println!("{}: {}", database.base, database.standing);
//! }
//! # Ok(())
//! # }
//! ```
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
//! let client = internetdata::Client::builder().api_key("your-api-key").build()?;
//! let databases = runtime.block_on(client.database().list())?;
//! # let _ = databases;
//! # Ok(())
//! # }
//! ```

mod client;
mod database;
mod enums;
mod error;
mod generated;
mod oauth;
mod transport;

// The OAuth unit tests reuse the integration suite's stub, which names this
// crate from outside.
#[cfg(test)]
extern crate self as internetdata;

pub use client::{Client, ClientBuilder, DEFAULT_BASE_URL};
pub use database::DatabaseApi;
pub use error::{Error, ErrorKind};
pub use oauth::{
    DeviceAuthorization, DeviceAuthorizationOptions, OauthApi, OauthError, OauthErrorResponse,
    OauthMetadata, OauthOptions, TokenResponse,
};

use generated::models;

// The v2 wire types, re-exported from the generated models so a consumer never
// names the module path.
pub use models::database::LicenseType;
pub use models::download::Outcome;
pub use models::{
    Checksums, ChecksumsResponse, Database, DatabaseMetadata, DatabaseMetadataColumn,
    DatabaseVersion, Download,
};
pub use models::{DatabaseFormat, Standing};
