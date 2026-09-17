// The database transfer half: what `download` and `download_bytes` must do with
// the 302, the bytes, and a transfer that does not finish.
//
// The stub records request HEADERS, because two of the guarantees here are about
// what a request did NOT carry, and a header that was never sent is invisible to
// any assertion made on a response.

mod support;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use internetdata::{DatabaseFormat, ErrorKind};
use support::{KEY, Route, Stub};

const DATASET: &str = "bogon_ip_v1";
const DOWNLOAD: &str = "/api/v2/database/download";
const STORAGE_PATH: &str = "/files/bogon_ip_v1.csv.gz";

#[tokio::test]
async fn download_streams_a_database_to_a_path_and_leaves_no_part_file() {
    let stub = serving(Route::ok(payload())).await;
    let client = stub.client().build().expect("build");
    let scratch = Scratch::new("streamed");
    let path = scratch.join("bogon_ip_v1.csv.gz");

    let written =
        client.database().download(DATASET, DatabaseFormat::Csvgz, &path).await.expect("download");

    assert_eq!(written, payload().len() as u64);
    assert_eq!(std::fs::read(&path).expect("reading the download back"), payload().as_bytes());
    assert!(!partial_of(&path).exists(), "the .part file outlived a successful transfer");
}

/// The presigned URL authorizes itself, so the request that follows the 302 must
/// carry no credential. reqwest's redirect policy is a CLIENT-level setting and
/// some versions carry headers across a redirect, which is why the SDK issues
/// this request itself rather than trusting a policy.
#[tokio::test]
async fn the_object_storage_request_carries_no_credential() {
    let stub = serving(Route::ok(payload())).await;
    let client = stub.client().build().expect("build");
    let scratch = Scratch::new("credential");

    client
        .database()
        .download(DATASET, DatabaseFormat::Csvgz, scratch.join("bogon_ip_v1.csv.gz"))
        .await
        .expect("download");

    let requests = stub.requests();
    let api = requests
        .iter()
        .find(|call| call.path == DOWNLOAD)
        .expect("the download endpoint was never asked");
    assert_eq!(api.header("authorization"), Some(format!("Bearer {KEY}").as_str()));

    let storage =
        requests.iter().find(|call| call.path == STORAGE_PATH).expect("no 302 was followed");
    assert_eq!(storage.header("authorization"), None, "the key was sent to object storage");
    // Every header, plus the synthetic one holding the request target, so a key
    // smuggled into a query string is caught as well as one in a header.
    for (name, value) in &storage.headers {
        assert!(!value.contains(KEY), "the API key reached object storage in {name}");
    }
}

#[tokio::test]
async fn download_bytes_agrees_with_the_streamed_copy() {
    let stub = serving(Route::ok(payload())).await;
    let client = stub.client().build().expect("build");
    let scratch = Scratch::new("agreement");
    let path = scratch.join("bogon_ip_v1.csv.gz");

    client.database().download(DATASET, DatabaseFormat::Csvgz, &path).await.expect("download");
    let bytes =
        client.database().download_bytes(DATASET, DatabaseFormat::Csvgz).await.expect("bytes");

    assert_eq!(bytes, std::fs::read(&path).expect("reading the download back"));
}

/// A short body must never be published as a whole database. The stub declares
/// four times the length it writes, and reqwest fails the stream against the
/// declared length before the crate's own backstop is reached; what this pins is
/// the contract, not which of the two layers enforces it.
#[tokio::test]
async fn a_truncated_transfer_fails_and_leaves_nothing_behind() {
    let stub = serving(short()).await;
    let client = stub.client().build().expect("build");
    let scratch = Scratch::new("truncated");
    let path = scratch.join("bogon_ip_v1.csv.gz");

    let err = client
        .database()
        .download(DATASET, DatabaseFormat::Csvgz, &path)
        .await
        .expect_err("a short body must not be written out as a whole database");

    assert!(err.retryable(), "a transfer that ended early is worth another attempt: {err}");
    assert!(!path.exists(), "a truncated transfer left a file that reads as a whole database");
    assert!(!partial_of(&path).exists(), "the .part file survived a failed transfer");
}

/// What the `.part` file is FOR, and the mutation that first went missed:
/// writing straight to the destination passes every "no truncated file survives"
/// check, because the failure path then deletes the destination. What `.part`
/// actually buys is that a failed download does not take the copy already on
/// disk with it - the destination would be truncated the moment the file was
/// opened, long before anyone knew whether the transfer would finish.
#[tokio::test]
async fn a_failed_transfer_leaves_the_database_already_on_disk_untouched() {
    let stub = serving(short()).await;
    let client = stub.client().build().expect("build");
    let scratch = Scratch::new("existing");
    let path = scratch.join("bogon_ip_v1.csv.gz");
    std::fs::write(&path, b"yesterday's database").expect("seeding the destination");

    client
        .database()
        .download(DATASET, DatabaseFormat::Csvgz, &path)
        .await
        .expect_err("the transfer must fail");

    assert_eq!(
        std::fs::read(&path).expect("yesterday's database is gone"),
        b"yesterday's database",
        "a failed download replaced a good database with nothing"
    );
}

#[tokio::test]
async fn a_truncated_download_bytes_fails_rather_than_returning_a_short_buffer() {
    let stub = serving(short()).await;
    let client = stub.client().build().expect("build");

    let err = client
        .database()
        .download_bytes(DATASET, DatabaseFormat::Csvgz)
        .await
        .expect_err("a short body must not come back as the database");

    assert!(err.retryable(), "a transfer that ended early is worth another attempt: {err}");
}

/// A license refusal is a client error: it carries the API's own `rc`, it is not
/// retried, and it never reaches object storage at all.
#[tokio::test]
async fn a_database_the_organization_does_not_license_is_refused_once() {
    let stub =
        Stub::start([(DOWNLOAD.to_owned(), Route::json(403, r#"{"rc":"NOT_LICENSED"}"#))]).await;
    let client = stub.client().retries(3).build().expect("build");
    let scratch = Scratch::new("unlicensed");
    let path = scratch.join("vpn_ip_v1.csv.gz");

    let err = client
        .database()
        .download("vpn_ip_v1", DatabaseFormat::Csvgz, &path)
        .await
        .expect_err("an unlicensed database must be refused");

    assert_eq!(err.kind(), ErrorKind::Forbidden);
    assert_eq!(err.status(), Some(403));
    assert!(!err.retryable(), "a license refusal is not worth retrying");
    assert_eq!(err.message(), "NOT_LICENSED", "the API's own reason went unread");
    assert_eq!(stub.count(), 1, "a 4xx must be issued exactly once");
    assert!(!path.exists() && !partial_of(&path).exists(), "a refusal wrote something to disk");
}

/// Object storage refusing a lapsed link is a different failure from the API
/// refusing the request, and the message has to say which one happened.
#[tokio::test]
async fn a_refused_download_link_names_object_storage() {
    let stub = serving(Route::json(403, "<Error><Code>AccessDenied</Code></Error>")).await;
    let client = stub.client().retries(1).build().expect("build");

    let err = client
        .database()
        .download_bytes(DATASET, DatabaseFormat::Csvgz)
        .await
        .expect_err("a refused link must fail");

    assert_eq!(err.kind(), ErrorKind::Forbidden);
    assert!(err.message().contains("object storage"), "{}", err.message());
}

/// Only the header phase of a transfer retries. Object storage failing with a
/// 5xx before any byte has moved is asked again, and nothing has been written.
#[tokio::test]
async fn an_object_storage_5xx_before_the_first_byte_is_retried() {
    let stub = serving(Route::ok(payload())).await;
    stub.sequence(STORAGE_PATH, [Route::json(503, ""), Route::ok(payload())]);
    let client = stub.client().build().expect("build");
    let scratch = Scratch::new("storage-503");
    let path = scratch.join("bogon_ip_v1.csv.gz");

    let written = client
        .database()
        .download(DATASET, DatabaseFormat::Csvgz, &path)
        .await
        .expect("a 503 before the transfer started is worth asking again");

    assert_eq!(storage_requests(&stub), 2, "the 503 was not retried");
    assert_eq!(written, payload().len() as u64);
    assert_eq!(std::fs::read(&path).expect("reading the download back"), payload().as_bytes());
}

/// A body that dies part way is NEVER fetched again: a second copy would land
/// behind the bytes already moved. Paired with the 503 test above, so neither
/// count can pass by accident.
#[tokio::test]
async fn a_transfer_that_dies_part_way_is_never_fetched_again() {
    for method in ["download", "download_bytes"] {
        let stub = serving(short()).await;
        let client = stub.client().retries(3).build().expect("build");
        let scratch = Scratch::new(&format!("died-{method}"));
        let path = scratch.join("bogon_ip_v1.csv.gz");
        let database = client.database();

        let failed = match method {
            "download" => database.download(DATASET, DatabaseFormat::Csvgz, &path).await.err(),
            _ => database.download_bytes(DATASET, DatabaseFormat::Csvgz).await.err(),
        };

        assert!(failed.is_some(), "{method}: a short body must fail");
        assert_eq!(storage_requests(&stub), 1, "{method}: the dead transfer was fetched again");
    }
}

/// The transfer is exempt from the client's timeout: a download runs for as long
/// as the file takes, and only one that stops moving fails. This one trickles for
/// several times the bound.
#[tokio::test]
async fn a_slow_transfer_is_not_cut_off_by_the_timeout() {
    let body = "slow,but,moving\n".repeat(4);
    let stub = serving(Route::ok(body.clone()).trickling(Duration::from_millis(20))).await;
    let timeout = Duration::from_millis(150);
    let client = stub.client().retries(0).timeout(timeout).build().expect("build");

    let start = Instant::now();
    let bytes = client
        .database()
        .download_bytes(DATASET, DatabaseFormat::Csvgz)
        .await
        .expect("a slow transfer is not a stalled one");

    assert_eq!(bytes, body.as_bytes());
    assert!(start.elapsed() > timeout * 3, "the transfer was not slow");
}

fn storage_requests(stub: &Stub) -> usize {
    stub.calls().iter().filter(|path| path.as_str() == STORAGE_PATH).count()
}

/// The API answers 302 and the whole point of `download` is the SECOND request,
/// so both must be on the record: one asking for the link, one taking the bytes.
async fn serving(storage: Route) -> std::sync::Arc<Stub> {
    let stub = Stub::start([]).await;
    let location = format!("{}{STORAGE_PATH}?X-Amz-Signature=presigned", stub.base_url);
    stub.route(DOWNLOAD, Route::json(302, "").header("Location", &location));
    stub.route(STORAGE_PATH, storage);
    stub
}

/// An origin that promises four times what it writes.
fn short() -> Route {
    Route::ok(payload()).promising(payload().len() as u64 * 4)
}

/// A body big enough to arrive in more than one chunk, and recognizable in a
/// diff when one of these assertions fails.
fn payload() -> String {
    "ip_range_start,ip_range_end\n".repeat(512)
}

fn partial_of(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.part", path.display()))
}

/// A scratch directory that removes itself. The crate has no tempfile
/// dependency and does not need one for this: the tests write kilobytes, and a
/// directory named after the test is easier to read in a failure than a random
/// one.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("internetdata-rust-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("creating a scratch directory");
        Self(dir)
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
