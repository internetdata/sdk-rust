//! The official Rust client library for the [InternetData](https://internetdata.io)
//! API: the licensed IP intelligence databases, their metadata and their
//! checksums.
//!
//! Start with [`Client::builder`] and an API key carrying the `db.download`
//! scope. There is no anonymous tier and no per-address lookup here: the whole
//! API is the database catalog and the files behind it.
//!
//! ```no_run
//! # async fn run() -> Result<(), internetdata::Error> {
//! let client = internetdata::Client::builder().api_key("your-api-key").build()?;
//!
//! for database in client.list().await? {
//!     println!("{}: {}", database.base, database.standing);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Your catalog is not everyone's catalog
//!
//! A database commissioned for a single customer is ABSENT from
//! [`Client::list`] for an organization that does not license it, rather than
//! present with an [`Standing::Unlicensed`] standing. The server decides what
//! you may see, so treat the listing as this key's answer: do not build a
//! catalog from any other source, and do not reuse one organization's listing
//! for another key.
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
//! let databases = runtime.block_on(client.list())?;
//! # let _ = databases;
//! # Ok(())
//! # }
//! ```

mod client;
mod database;
mod enums;
mod error;
mod generated;
mod transport;

pub use client::{Client, ClientBuilder, DEFAULT_BASE_URL};
pub use error::{Error, ErrorKind};

use generated::models;

// The v2 wire types, re-exported from the generated models so a consumer never
// names the module path. The spec still describes the legacy v1 endpoints and
// their schemas are generated too; they are not re-exported, because this crate
// targets v2 alone.
pub use models::checksums_response::Format;
pub use models::database::{Redistribution, Standing};
pub use models::database_version::Formats;
pub use models::download::Outcome;
pub use models::{
    Checksums, ChecksumsResponse, Database, DatabaseMetadata, DatabaseMetadataColumn,
    DatabaseVersion, Download,
};
