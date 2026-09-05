// The published crate against a real deployment.
//
// CI compiles the working tree; nothing there would notice a tag that never
// landed, an `exclude` that dropped a module from the package, or a feature a
// consumer cannot turn on. Only this suite tests what a stranger resolves.
//
// The transfer is budgeted before it starts. `metadata` publishes a size per
// format, and that size is checked against the ceiling FIRST, so a mistaken id
// can never quietly pull one of the gigabyte databases through CI.

use internetdata::{ErrorKind, Format};
use internetdata_integration::{
    CEILING, Catalog, STAGING, assert_catalog_shape, catalog, client_for, recorder::Fact,
    skip_reason, skip_unless,
};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use tokio::sync::OnceCell;

#[tokio::test]
async fn the_catalog_answers_the_schema_the_client_was_generated_from() {
    skip_unless!(skip_reason());
    let catalog = catalog().await;

    assert_catalog_shape(catalog);

    // Named with what actually arrived: every typed assertion above reads as a
    // zero value when the payload disagrees, and a bare "want a string" costs a
    // whole CI cycle to interpret.
    let served: Vec<&str> = catalog.wire[0]
        .as_object()
        .expect("a family is an object")
        .keys()
        .map(String::as_str)
        .collect();
    for want in ["base", "standing", "versions"] {
        assert!(
            served.contains(&want),
            "the payload carries {served:?}, and Database declares {want}"
        );
    }

    println!("visible: {}", summary(catalog));
}

/// The visibility contract as the server enforces it: this key's listing is this
/// key's listing. Nothing here can prove what a DIFFERENT organization sees, so
/// what is checked is the half an SDK could break - that the client reports the
/// server's answer rather than one it assembled.
#[tokio::test]
async fn the_listing_is_the_servers_answer_and_not_the_clients() {
    skip_unless!(skip_reason());
    let catalog = catalog().await;

    let decoded: Vec<&str> = catalog.databases.iter().map(|d| d.base.as_str()).collect();
    let served: Vec<&str> =
        catalog.wire.iter().map(|d| d["base"].as_str().expect("base is a string")).collect();

    assert_eq!(decoded, served, "the client did not hand back the listing it was served");
}

#[tokio::test]
async fn a_database_the_organization_does_not_license_is_refused_cleanly() {
    skip_unless!(skip_reason());
    let catalog = catalog().await;
    let Some(unlicensed) = catalog.unlicensed() else {
        println!("SKIPPED: this key licenses everything it can see, so nothing is refusable");
        return;
    };
    let (client, recorder) = client_for().await;

    let err = client
        .database()
        .download_url(unlicensed, Format::Csvgz)
        .await
        .unwrap_err_or_explain(unlicensed);

    assert_eq!(err.kind(), ErrorKind::Forbidden, "kind: {err}");
    assert_eq!(err.status(), Some(403));
    assert!(!err.retryable(), "a licence refusal is not worth retrying");
    // The API says which refusal this is (`{"rc":"NOT_LICENSED"}`). Falling back
    // to the status means the client never read the envelope.
    assert!(
        !err.message().starts_with("request failed with status"),
        "message {:?} is the client fallback, so the body went unread",
        err.message()
    );
    assert_eq!(recorder.facts().len(), 1, "a 4xx must not be retried");
    println!("{unlicensed} refused with {}", err.message());
}

#[tokio::test]
async fn download_streams_a_real_database_to_disk_intact() {
    skip_unless!(skip_reason());
    let Some(transfer) = transferred().await else {
        return;
    };

    assert!(transfer.written > 0, "nothing was transferred");
    let body = std::fs::read(&transfer.path).expect("the download is not on disk");
    assert_eq!(body.len() as u64, transfer.written, "the file and the reported size disagree");
    assert!(!partial_of(&transfer.path).exists(), "the .part file outlived a successful transfer");
    assert_eq!(
        transfer.written as i64, transfer.size,
        "the transfer and the published size differ"
    );

    assert_eq!(transfer.sha256.len(), 64, "sha256 did not unwrap past its key");
    assert_eq!(digest(&body), transfer.sha256, "the transferred bytes hash to something else");

    // The presigned URL authorizes itself, so the request that follows the 302
    // must carry no credential.
    let storage: Vec<&Fact> = transfer.facts.iter().filter(|fact| fact.origin != STAGING).collect();
    assert!(!storage.is_empty(), "nothing was fetched from object storage, so no 302 was followed");
    for fact in storage {
        assert!(!fact.carried_key, "the API key was sent to object storage at {}", fact.origin);
    }
}

#[tokio::test]
async fn download_bytes_agrees_with_the_streamed_copy() {
    skip_unless!(skip_reason());
    let Some(transfer) = transferred().await else {
        return;
    };
    let (client, _) = client_for().await;

    let raw = client
        .database()
        .download_bytes(&transfer.id, transfer.format)
        .await
        .expect("download_bytes");

    assert_eq!(raw.len() as u64, transfer.written, "the in-memory copy is a different size");
    assert_eq!(
        digest(&raw),
        transfer.sha256,
        "the in-memory copy hashes to something the API does not publish"
    );
}

/// `download_url` hands back a link a caller may pass on, so it has to work
/// without this client: fetched with a bare reqwest that has never seen the key.
#[tokio::test]
async fn the_presigned_link_works_with_no_credential_at_all() {
    skip_unless!(skip_reason());
    let Some(transfer) = transferred().await else {
        return;
    };
    let (client, _) = client_for().await;

    // Straight at staging rather than through the recorder: the recorder is what
    // proves what was SENT, and here the point is that a stranger's HTTP client
    // needs nothing from us.
    let url =
        client.database().download_url(&transfer.id, transfer.format).await.expect("download_url");
    let stranger = reqwest::Client::builder().build().expect("building a plain client");
    let body =
        stranger.get(&url).send().await.expect("fetching the link").bytes().await.expect("body");

    assert_eq!(digest(&body), transfer.sha256, "the link served something else");
}

struct Transfer {
    id: String,
    format: Format,
    size: i64,
    written: u64,
    path: PathBuf,
    sha256: String,
    facts: Vec<Fact>,
}

/// Memoized so every transfer test shares one download rather than pulling the
/// database again each time. `None` when nothing licensed fits under the
/// ceiling, which is a deliberate licence change rather than a bug, so it skips
/// with a reason instead of failing.
static TRANSFER: OnceCell<Option<Transfer>> = OnceCell::const_new();

async fn transferred() -> Option<&'static Transfer> {
    let transfer = TRANSFER.get_or_init(transfer).await.as_ref();
    if transfer.is_none() {
        println!("SKIPPED: nothing this key licenses is under the {CEILING} byte ceiling");
    }
    transfer
}

async fn transfer() -> Option<Transfer> {
    let (client, recorder) = client_for().await;
    let catalog = catalog().await;

    let licensed = catalog.licensed();
    // An empty set is a broken credential rather than a licence decision: this
    // key exists to download something.
    assert!(!licensed.is_empty(), "this key licenses nothing, so there is nothing to download");

    // The SMALLEST licensed artifact, chosen from published sizes before a byte
    // moves. Mutating the ceiling to something under it proves the gate fires
    // ahead of the transfer rather than after it.
    let mut smallest: Option<(String, Format, i64)> = None;
    for (id, format) in licensed {
        let format = Format::from(format);
        let meta = client.database().metadata(id).await.expect("metadata");
        assert_eq!(meta.id, id, "metadata answered about the wrong database");
        let Some(&size) = meta.size.get(format.as_str()) else {
            continue;
        };
        assert!(size > 0, "{id}.{format} publishes a size of {size}");
        if smallest.as_ref().is_none_or(|(_, _, best)| size < *best) {
            smallest = Some((id.to_owned(), format, size));
        }
    }

    let (id, format, size) = smallest?;
    if size > CEILING {
        return None;
    }

    let path = scratch().join(format!("{id}.{format}"));
    let written = client.database().download(&id, format, &path).await.expect("download");
    // Read AFTER the transfer, so a rebuild between the two calls shows up as a
    // digest mismatch rather than passing against a digest of nothing.
    let sums = client.database().checksums(&id, format).await.expect("checksums");
    println!("{id}.{format}: {written} bytes, metadata says {size}");

    Some(Transfer { id, format, size, written, path, sha256: sums.sha256, facts: recorder.facts() })
}

fn summary(catalog: &Catalog) -> String {
    catalog
        .databases
        .iter()
        .map(|database| format!("{}={}", database.base, database.standing))
        .collect::<Vec<_>>()
        .join(", ")
}

fn digest(body: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(body);
    hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A scratch directory for the whole binary rather than for one test, because
/// the transfer tests share the download and the later ones still have to read
/// what the first wrote.
fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("internetdata-integration-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("creating a scratch directory");
    dir
}

fn partial_of(path: &std::path::Path) -> PathBuf {
    PathBuf::from(format!("{}.part", path.display()))
}

/// A refusal that arrives as anything else is almost always the database having
/// been licensed since, and a bare `unwrap` on it would say only "called
/// unwrap_err on an Ok value".
trait ExplainRefusal {
    fn unwrap_err_or_explain(self, id: &str) -> internetdata::Error;
}

impl<T> ExplainRefusal for Result<T, internetdata::Error> {
    fn unwrap_err_or_explain(self, id: &str) -> internetdata::Error {
        match self {
            Err(err) => err,
            Ok(_) => panic!(
                "{id} was not refused, though the catalog lists it as unlicensed. Either the \
                 licence changed under this run or `standing` and the download gate disagree"
            ),
        }
    }
}
