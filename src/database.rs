use std::path::{Path, PathBuf};

use tokio::io::AsyncWriteExt;

use crate::client::{Client, with_retry};
use crate::error::Error;
use crate::models::checksums_response::Format;
use crate::models::{
    Checksums, ChecksumsResponse, Database, DatabaseList, DatabaseMetadata, Download, DownloadList,
};

const LIST: &str = "/api/v2/database/list";
const METADATA: &str = "/api/v2/database/metadata";
const CHECKSUM: &str = "/api/v2/database/checksum";
const DOWNLOADS: &str = "/api/v2/database/downloads";
const DOWNLOAD: &str = "/api/v2/database/download";

/// The database catalog and its downloads, which is the whole of InternetData's
/// public surface.
///
/// Reached through [`Client::database`]. Named `DatabaseApi` rather than
/// `Database` because [`Database`] is already the shape of one database family,
/// and it is what the other InternetData SDKs call this type.
#[derive(Debug, Clone, Copy)]
pub struct DatabaseApi<'a> {
    client: &'a Client,
}

impl<'a> DatabaseApi<'a> {
    pub(crate) fn new(client: &'a Client) -> Self {
        Self { client }
    }

    /// Every database FAMILY your organization may see, with where each one
    /// stands: `licensed`, `expired`, or `unlicensed` for one published but
    /// never bought.
    ///
    /// A licence covers a family while a download names one of its versions, so
    /// the ids [`DatabaseApi::download`] and [`DatabaseApi::checksums`] take come from
    /// [`Database::versions`] rather than from the family itself.
    ///
    /// **This listing is yours, not everyone's.** A database commissioned for a
    /// single customer is ABSENT from it entirely for an organization that does
    /// not license it, rather than present with an `unlicensed` standing. The
    /// server decides that, so do not reconstruct a catalog from anywhere else,
    /// do not reuse one organization's listing for another key, and do not treat
    /// what you see as the whole published catalog.
    pub async fn list(&self) -> Result<Vec<Database>, Error> {
        let response: DatabaseList = self.get(LIST, &[]).await?;
        Ok(response.databases)
    }

    /// What is inside one database: its column schema and sample rows per
    /// format, the row count, the byte size of each artifact, and the day it was
    /// built.
    ///
    /// It carries `updated` and `entries` without downloading anything, so poll
    /// it to decide whether today's build is worth fetching, and read `size` to
    /// know what a transfer will cost before you start it.
    pub async fn metadata(&self, id: &str) -> Result<DatabaseMetadata, Error> {
        self.get(METADATA, &[("id", id)]).await
    }

    /// The digests of one published file, for verifying a download.
    ///
    /// The whole set is returned rather than one algorithm, because which
    /// digests a database publishes is the API's choice. They nest under
    /// `checksums` in the response, and reading a top-level `sha256` is how the
    /// VPNDetection Node SDK shipped this broken in 1.0.x.
    pub async fn checksums(&self, id: &str, format: impl Into<Format>) -> Result<Checksums, Error> {
        let format = format.into();
        let response: ChecksumsResponse =
            self.get(CHECKSUM, &[("id", id), ("format", format.as_str())]).await?;
        Ok(*response.checksums)
    }

    /// Your organization's recent download attempts, newest first, refusals
    /// included. `None` takes the API's own default of 50; the API clamps
    /// anything above 200.
    ///
    /// A denial is what answers "it stopped working", and its absence answers
    /// nothing, which is why they are listed alongside the successes.
    pub async fn downloads(&self, limit: Option<u32>) -> Result<Vec<Download>, Error> {
        let limit = limit.map(|n| n.to_string());
        let query: Vec<(&str, &str)> = match &limit {
            Some(n) => vec![("limit", n.as_str())],
            None => vec![],
        };
        let response: DownloadList = self.get(DOWNLOADS, &query).await?;
        Ok(response.downloads)
    }

    /// The time-limited URL for one database file.
    ///
    /// The API answers 302 to object storage and the link authorizes itself, so
    /// what comes back carries no credential of yours and can be handed to
    /// anything that speaks HTTP. It is returned rather than followed because
    /// the caller decides how to transfer a file that reaches gigabytes; the
    /// link authorizes the START of a transfer, so one already running is not
    /// interrupted when it lapses.
    pub async fn download_url(&self, id: &str, format: impl Into<Format>) -> Result<String, Error> {
        let format = format.into();
        let query = [("id", id), ("format", format.as_str())];
        with_retry(self.client.retries(), || self.client.transport().get_redirect(DOWNLOAD, &query))
            .await
    }

    /// Downloads one database file to `path` and returns the bytes written.
    ///
    /// The bytes stream straight to disk, so nothing beyond a single chunk is
    /// held in memory whatever the database weighs. They land in a neighboring
    /// `.part` file that is renamed on completion, so a transfer that dies half
    /// way cannot leave a truncated file that reads as a whole database, and -
    /// the thing writing straight to `path` would get wrong - cannot take
    /// yesterday's copy with it either.
    pub async fn download(
        &self,
        id: &str,
        format: impl Into<Format>,
        path: impl AsRef<Path>,
    ) -> Result<u64, Error> {
        let path = path.as_ref();
        let partial = partial_path(path);
        let response = self.fetch_file(id, format.into()).await?;

        let outcome = match stream_to_file(response, &partial).await {
            Ok(written) => {
                tokio::fs::rename(&partial, path).await.map(|()| written).map_err(Error::from)
            }
            Err(err) => Err(err),
        };
        if outcome.is_err() {
            // Best effort: the transfer already failed, and a partial file that
            // cannot be removed is not a second failure worth reporting over the
            // first one.
            let _ = tokio::fs::remove_file(&partial).await;
        }
        outcome
    }

    /// Downloads one database file and hands back its bytes.
    ///
    /// **This holds the entire file in memory**, and the catalog spans seven
    /// orders of magnitude, from `bogon_asn_v1` at 264 bytes to the largest
    /// datasets at several gigabytes. Reach for it at the small end, where the
    /// bytes go straight into a parser, and use [`DatabaseApi::download`] for
    /// anything you have not checked with [`DatabaseApi::metadata`].
    pub async fn download_bytes(
        &self,
        id: &str,
        format: impl Into<Format>,
    ) -> Result<Vec<u8>, Error> {
        let mut response = self.fetch_file(id, format.into()).await?;
        let declared = response.content_length();
        // Allocated once from the declared length. A Vec grows by doubling, so
        // the last grow of a large database alone costs twice the file.
        let mut bytes = Vec::with_capacity(declared.unwrap_or(0) as usize);
        while let Some(chunk) = response.chunk().await? {
            bytes.extend_from_slice(&chunk);
        }
        assert_whole_transfer(declared, bytes.len() as u64)?;
        Ok(bytes)
    }

    // The 302 is followed as a SECOND request rather than by loosening the
    // redirect guard, because the presigned URL authorizes itself and forwarding
    // the API key would hand a credential to a host with no business holding it.
    // Transport::get_file builds that request without one.
    async fn fetch_file(&self, id: &str, format: Format) -> Result<reqwest::Response, Error> {
        let url = self.download_url(id, format).await?;
        with_retry(self.client.retries(), || self.client.transport().get_file(&url)).await
    }

    async fn get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<T, Error> {
        with_retry(self.client.retries(), || self.client.transport().get_json(path, query)).await
    }
}

/// Streams a response body into `partial`, returning the bytes written.
///
/// The file is closed before the caller renames it: on some platforms a rename
/// over an open handle is refused, and a flush that fails after the rename has
/// already happened would publish a short file as a whole one.
async fn stream_to_file(mut response: reqwest::Response, partial: &Path) -> Result<u64, Error> {
    let declared = response.content_length();
    let mut file = tokio::fs::File::create(partial).await?;
    let mut written = 0u64;
    while let Some(chunk) = response.chunk().await? {
        file.write_all(&chunk).await?;
        written += chunk.len() as u64;
    }
    file.flush().await?;
    drop(file);
    assert_whole_transfer(declared, written)?;
    Ok(written)
}

/// `path` with `.part` appended, rather than [`Path::with_extension`], which
/// would turn `bogon_ip_v1.csv.gz` into `bogon_ip_v1.csv.part` and take a real
/// extension with it.
fn partial_path(path: &Path) -> PathBuf {
    let mut partial = path.as_os_str().to_owned();
    partial.push(".part");
    PathBuf::from(partial)
}

/// A backstop against a short transfer being written out as a whole database.
///
/// reqwest 0.13 gets there first: hyper compares the body against
/// `Content-Length` itself and fails the stream with "error decoding response
/// body", which is what the truncation tests actually observe, so this check has
/// never fired in anger. It stays because that is a transport promise rather
/// than an API one and this crate owns the guarantee for the version that stops
/// making it. `UnexpectedEof` rather than a kind of our own, because that is
/// what this is and it is what makes [`Error::retryable`] true for it.
fn assert_whole_transfer(declared: Option<u64>, written: u64) -> Result<(), Error> {
    match declared {
        Some(want) if want != written => Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            format!("the transfer ended after {written} of {want} bytes"),
        ))),
        _ => Ok(()),
    }
}
