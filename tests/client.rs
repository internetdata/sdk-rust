// The Rust-specific API surface, as distinct from the shared conformance corpus
// in conformance.rs: what each call unwraps, what it sends, and what the retry
// loop does with a 429.

mod support;

use std::time::{Duration, Instant};

use internetdata::{Client, DatabaseFormat, Error, ErrorKind, Outcome, Standing};
use support::{KEY, Route, Stub};

const LIST: &str = "/api/v2/database/list";
const METADATA: &str = "/api/v2/database/metadata";
const CHECKSUM: &str = "/api/v2/database/checksum";
const DOWNLOADS: &str = "/api/v2/database/downloads";
const DOWNLOAD: &str = "/api/v2/database/download";

/// A license is held against a FAMILY, and the ids a download takes are one
/// level further down. Reading `{id, formats}` off the family is how list ->
/// download broke in every VPNDetection SDK, so the depth is pinned here.
#[tokio::test]
async fn the_listing_unwraps_a_family_and_its_versions() {
    let stub = Stub::start([(
        LIST.to_owned(),
        Route::ok(
            r#"{"databases":[{"base":"bogon_ip","name":"Bogon IP","summary":"unroutable ranges","standing":"licensed","license_type":"standard","starts":"2026-01-01T00:00:00.000Z","expires":null,"renews_at":null,"notice_due_at":null,"versions":[{"id":"bogon_ip_v1","version":1,"summary":"v1","formats":["csvgz","mmdb"]}]}]}"#,
        ),
    )])
    .await;
    let client = stub.client().build().expect("build");

    let databases = client.database().list().await.expect("list");

    assert_eq!(databases.len(), 1);
    let family = &databases[0];
    assert_eq!(family.base, "bogon_ip");
    assert_eq!(family.standing, Standing::Licensed);
    assert!(family.expires.is_none(), "a license with no end date reads as None");
    assert!(family.starts.is_some(), "starts is present and non-null here");
    let version = &family.versions[0];
    assert_eq!(version.id, "bogon_ip_v1", "the id a download takes lives on the VERSION");
    assert_eq!(version.version, 1);
    assert_eq!(version.formats, vec![DatabaseFormat::Csvgz, DatabaseFormat::Mmdb]);
}

/// Which digests a database publishes is the API's choice, so the whole set
/// comes back. They nest under `checksums`, and reading a top-level `sha256` is
/// how the VPNDetection Node SDK shipped this broken in 1.0.x.
#[tokio::test]
async fn checksums_returns_the_whole_digest_set_from_under_its_key() {
    let stub = Stub::start([(
        CHECKSUM.to_owned(),
        Route::ok(
            r#"{"id":"bogon_ip_v1","format":"mmdb","checksums":{"md5":"m","sha1":"s1","sha256":"s256","sha512":"s512"}}"#,
        ),
    )])
    .await;
    let client = stub.client().build().expect("build");

    let sums =
        client.database().checksums("bogon_ip_v1", DatabaseFormat::Mmdb).await.expect("checksums");

    assert_eq!(sums.md5, "m");
    assert_eq!(sums.sha1, "s1");
    assert_eq!(sums.sha256, "s256");
    assert_eq!(sums.sha512, "s512");
    let target = stub.target(CHECKSUM).expect("the checksum endpoint was never asked");
    assert!(target.contains("id=bogon_ip_v1") && target.contains("format=mmdb"), "{target}");
}

/// The metadata document is what a caller budgets a transfer against, so `size`
/// and `entries` have to survive the decode, and `sample`/`update_freq` have to
/// be optional rather than required.
#[tokio::test]
async fn metadata_carries_the_sizes_a_transfer_is_budgeted_against() {
    let stub = Stub::start([(
        METADATA.to_owned(),
        Route::ok(
            r#"{"id":"bogon_ip_v1","update_freq":"daily","updated":"2026-09-04","entries":1234,"schema":{"csvgz":[{"name":"ip_range_start","type":"string","description":"first address"}]},"size":{"csvgz":760,"mmdb":3524}}"#,
        ),
    )])
    .await;
    let client = stub.client().build().expect("build");

    let meta = client.database().metadata("bogon_ip_v1").await.expect("metadata");

    assert_eq!(meta.id, "bogon_ip_v1");
    assert_eq!(meta.entries, 1234);
    assert_eq!(meta.update_freq.as_deref(), Some("daily"));
    assert_eq!(meta.updated.to_string(), "2026-09-04");
    assert_eq!(meta.size.get("csvgz"), Some(&760));
    assert_eq!(meta.size.get("mmdb"), Some(&3524));
    assert_eq!(meta.schema["csvgz"][0].name, "ip_range_start");
    assert!(meta.sample.is_none(), "a document without samples must decode, not fail");
}

#[tokio::test]
async fn a_metadata_document_without_its_optional_fields_still_decodes() {
    let stub = Stub::start([(
        METADATA.to_owned(),
        Route::ok(
            r#"{"id":"bogon_asn_v1","updated":"2026-09-04","entries":7,"schema":{},"size":{"csvgz":264}}"#,
        ),
    )])
    .await;
    let client = stub.client().build().expect("build");

    let meta = client.database().metadata("bogon_asn_v1").await.expect("metadata");

    assert_eq!(meta.update_freq, None);
    assert_eq!(meta.sample, None);
}

/// Refusals are listed alongside successes, and their nullable columns are
/// nullable rather than missing, so a denied attempt has to decode.
#[tokio::test]
async fn the_download_history_decodes_a_refusal_as_well_as_a_success() {
    let stub = Stub::start([(
        DOWNLOADS.to_owned(),
        Route::ok(
            r#"{"downloads":[{"dataset_id":"bogon_ip_v1","format":"csvgz","outcome":"ok","bytes":760,"http_status":302,"apikey_id":"ak_1","client_ip":"203.0.113.7","user_agent":"curl/8","created":"2026-09-04T10:00:00.000Z"},{"dataset_id":"vpn_ip_v1","format":"mmdb","outcome":"denied","bytes":null,"http_status":403,"apikey_id":null,"client_ip":null,"user_agent":null,"created":"2026-09-04T09:00:00.000Z"}]}"#,
        ),
    )])
    .await;
    let client = stub.client().build().expect("build");

    let attempts = client.database().downloads(Some(2)).await.expect("downloads");

    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0].outcome, Outcome::Ok);
    assert_eq!(attempts[0].bytes, Some(760));
    assert_eq!(attempts[1].outcome, Outcome::Denied);
    assert_eq!(attempts[1].bytes, None, "a refusal moved no bytes and says so with null");
    assert_eq!(attempts[1].apikey_id, None);
    let target = stub.target(DOWNLOADS).expect("the downloads endpoint was never asked");
    assert!(target.contains("limit=2"), "{target}");
}

/// `None` must send no `limit` at all rather than a literal, so the API's own
/// default applies and a later change to it reaches this client for free.
#[tokio::test]
async fn no_limit_sends_no_limit() {
    let stub = Stub::start([(DOWNLOADS.to_owned(), Route::ok(r#"{"downloads":[]}"#))]).await;
    let client = stub.client().build().expect("build");

    client.database().downloads(None).await.expect("downloads");

    let target = stub.target(DOWNLOADS).expect("the downloads endpoint was never asked");
    assert!(!target.contains("limit"), "{target}");
}

/// The download endpoint answers 302 to object storage, and the database behind
/// it reaches gigabytes. The origin here PROMISES 8 GiB, so a client that
/// follows the redirect is caught by the request count rather than by the wait.
#[tokio::test]
async fn download_url_returns_the_redirect_rather_than_following_it() {
    let stub = Stub::start([]).await;
    let location = format!("{}/huge.mmdb", stub.base_url);
    stub.route(DOWNLOAD, Route::json(302, "").header("Location", &location));
    stub.route("/huge.mmdb", Route::ok("").promising(8 * 1024 * 1024 * 1024));
    let client = stub.client().build().expect("build");

    let url = client
        .database()
        .download_url("bogon_ip_v1", DatabaseFormat::Mmdb)
        .await
        .expect("download_url");

    assert_eq!(url, location);
    assert_eq!(stub.calls(), vec![DOWNLOAD], "the redirect must not be followed");
}

/// The link authorizes itself, so it can be handed to anything that speaks HTTP.
/// That is only true if none of the caller's credential rode along in it.
#[tokio::test]
async fn the_returned_link_carries_no_credential_of_ours() {
    let stub = Stub::start([]).await;
    let location = format!("{}/files/bogon_ip_v1.csv.gz?X-Amz-Signature=presigned", stub.base_url);
    stub.route(DOWNLOAD, Route::json(302, "").header("Location", &location));
    let client = stub.client().build().expect("build");

    let url = client
        .database()
        .download_url("bogon_ip_v1", DatabaseFormat::Csvgz)
        .await
        .expect("download_url");

    assert!(!url.contains(KEY), "the API key came back inside the presigned link");
}

/// reqwest follows redirects by DEFAULT and the policy is a client-level setting
/// with no per-request override, so a caller-supplied client is the one way this
/// can still go wrong. It is refused rather than silently downloaded.
#[tokio::test]
async fn a_redirect_following_http_client_is_refused_not_obeyed() {
    let stub = Stub::start([]).await;
    let location = format!("{}/huge.mmdb", stub.base_url);
    stub.route(DOWNLOAD, Route::json(302, "").header("Location", &location));
    stub.route("/huge.mmdb", Route::ok("").promising(8 * 1024 * 1024 * 1024));
    let client = stub.client().http_client(reqwest::Client::new()).build().expect("build");

    let err = client
        .database()
        .download_url("bogon_ip_v1", DatabaseFormat::Mmdb)
        .await
        .expect_err("a followed redirect has no Location left to return");

    assert!(err.message().contains("redirect::Policy::none"), "{}", err.message());
}

/// A 404 from a misspelled database id is a CLIENT error. Letting it fall
/// through to the retryable `server_error` default is the mistake three of the
/// four VPNDetection SDKs shipped with.
#[tokio::test]
async fn an_unknown_database_is_not_retried() {
    let stub =
        Stub::start([(METADATA.to_owned(), Route::json(404, r#"{"rc":"UNKNOWN_DATASET"}"#))]).await;
    let client = stub.client().retries(3).build().expect("build");

    let err = client.database().metadata("no_such_database").await.expect_err("404");

    assert_eq!(err.kind(), ErrorKind::BadRequest);
    assert!(!err.retryable());
    assert_eq!(err.message(), "UNKNOWN_DATASET");
    assert_eq!(stub.count(), 1);
}

/// A 429 with no Retry-After is a spent allowance, and retrying it is hammering
/// a quota that will not recover until its window rolls over.
#[tokio::test]
async fn a_spent_quota_is_never_retried() {
    let stub =
        Stub::start([(LIST.to_owned(), Route::json(429, r#"{"rc":"QUOTA_EXCEEDED"}"#))]).await;
    let client = stub.client().retries(5).build().expect("build");

    let err = client.database().list().await.expect_err("a 429 should fail");

    assert_eq!(err.kind(), ErrorKind::QuotaExceeded);
    assert_eq!(stub.count(), 1);
}

#[tokio::test]
async fn a_rate_limit_is_retried_after_the_server_supplied_wait() {
    let stub = Stub::start([(
        LIST.to_owned(),
        Route::json(429, r#"{"rc":"RATE_LIMITED"}"#).header("Retry-After", "1"),
    )])
    .await;
    let client = stub.client().retries(1).build().expect("build");

    let start = Instant::now();
    client.database().list().await.expect_err("the call should still have failed");

    assert_eq!(stub.count(), 2);
    // The header, not the backoff schedule, decides the wait.
    assert!(start.elapsed() >= Duration::from_secs(1), "waited {:?}", start.elapsed());
}

/// Past 2^31 - 1 ms a `Retry-After` is waited out on the client's own backoff,
/// still a throttle: 2147484 held a call for 24.8 days, and 9223372036854775807
/// or a year-9999 date for good (2.2.1). Object storage's is bounded the same
/// way, in download.rs.
#[tokio::test]
async fn a_retry_after_past_its_bound_waits_the_backoff() {
    for value in ["2147484", "9223372036854775807", "Fri, 31 Dec 9999 23:59:59 GMT"] {
        let stub = Stub::start([]).await;
        stub.sequence(
            LIST,
            [
                Route::json(429, r#"{"rc":"RATE_LIMITED"}"#).header("Retry-After", value),
                Route::ok(r#"{"databases":[]}"#),
            ],
        );
        let client = stub.client().build().expect("build");

        let listed = tokio::time::timeout(Duration::from_secs(5), client.database().list())
            .await
            .unwrap_or_else(|_| panic!("Retry-After {value} held the call"));

        listed.unwrap_or_else(|e| panic!("Retry-After {value}: {e}"));
        assert_eq!(stub.count(), 2, "Retry-After {value}");
    }

    // One below the bound is still the server's to set.
    let stub = Stub::start([]).await;
    stub.sequence(
        LIST,
        [Route::json(429, r#"{"rc":"RATE_LIMITED"}"#).header("Retry-After", "2147483")],
    );
    let client = stub.client().build().expect("build");
    let held = tokio::time::timeout(Duration::from_secs(2), client.database().list()).await;
    assert!(held.is_err(), "Retry-After 2147483 was not waited out");

    // One past u64 never parses, so its 429 stays a spent quota.
    let stub = Stub::start([(
        LIST.to_owned(),
        Route::json(429, r#"{"rc":"RATE_LIMITED"}"#).header("Retry-After", "18446744073709551616"),
    )])
    .await;
    let client = stub.client().build().expect("build");
    let err = client.database().list().await.expect_err("a spent quota");
    assert_eq!(err.kind(), ErrorKind::QuotaExceeded, "{err}");
    assert_eq!(stub.count(), 1);
}

#[tokio::test]
async fn a_server_fault_is_retried_up_to_the_configured_budget() {
    let stub = Stub::start([(LIST.to_owned(), Route::json(503, r#"{"rc":"UNAVAILABLE"}"#))]).await;
    let client = stub.client().retries(2).build().expect("build");

    let err = client.database().list().await.expect_err("a 503 should fail");

    assert!(err.retryable());
    // One initial attempt plus two retries.
    assert_eq!(stub.count(), 3);
}

/// The key travels as a bearer token and nowhere else. The generated client
/// sends a configured key as `?apikey=` too, which the v2 endpoints do not
/// accept and a proxy would log.
#[tokio::test]
async fn the_key_travels_as_a_bearer_token_and_not_in_the_query_string() {
    let stub = Stub::start([(LIST.to_owned(), Route::ok(r#"{"databases":[]}"#))]).await;
    let client = stub.client().build().expect("build");

    client.database().list().await.expect("list");

    let call = &stub.requests()[0];
    assert_eq!(call.header("authorization"), Some(format!("Bearer {KEY}").as_str()));
    let target = stub.target(LIST).expect("the list endpoint was never asked");
    assert!(!target.contains(KEY), "the key reached the query string: {target}");
}

/// Today every endpoint is licensed, so a keyless client only ever gets a 401.
/// It still has to BUILD and to send no credential at all, because a database
/// offered without a license would need exactly this client. The empty arm is
/// what an unset `${{ secrets.X }}` interpolates to, where `Bearer ` with
/// nothing behind it is a worse answer than no header.
#[tokio::test]
async fn a_client_without_a_key_sends_no_authorization_header() {
    let stub = Stub::start([(LIST.to_owned(), Route::json(401, r#"{"rc":"UNAUTHORIZED"}"#))]).await;

    for builder in [stub.anonymous(), stub.anonymous().api_key("")] {
        let client = builder.retries(0).build().expect("build");

        let err = client
            .database()
            .list()
            .await
            .expect_err("an anonymous caller cannot enumerate the catalog");

        assert_eq!(err.kind(), ErrorKind::Unauthorized);
    }

    assert_eq!(stub.count(), 2);
    for call in stub.requests() {
        assert_eq!(call.header("authorization"), None);
    }
}

#[test]
fn the_builder_rejects_an_unusable_base_url() {
    assert!(Client::builder().base_url("not a url").build().is_err());
    assert!(Client::builder().base_url("/relative").build().is_err());
    assert!(Client::builder().base_url("https://internetdata.io").build().is_ok());
}

/// A trailing slash on the base URL must not produce `//api/v2/...`, which some
/// proxies answer with a redirect and others with a 404.
#[tokio::test]
async fn a_trailing_slash_on_the_base_url_is_not_doubled() {
    const METADATA_PATH: &str = "/.well-known/oauth-authorization-server";
    for slashes in 1..=3 {
        let stub = Stub::start([
            (LIST.to_owned(), Route::ok(r#"{"databases":[]}"#)),
            (
                METADATA_PATH.to_owned(),
                Route::ok(
                    r#"{"issuer":"https://api.example.test",
                    "authorization_endpoint":"https://api.example.test/oauth/authorize",
                    "token_endpoint":"https://api.example.test/oauth/token"}"#,
                ),
            ),
            (DOWNLOAD.to_owned(), Route::json(302, "").header("Location", "https://s3.test/x")),
        ])
        .await;
        let client = Client::builder()
            .base_url(format!("{}{}", stub.base_url, "/".repeat(slashes)))
            .api_key(KEY)
            .build()
            .expect("build");

        client.database().list().await.expect("list");
        client.oauth().metadata().await.expect("metadata");
        client.database().download_url("x_v1", DatabaseFormat::Csvgz).await.expect("link");

        assert_eq!(stub.calls(), vec![LIST, METADATA_PATH, DOWNLOAD], "{slashes} slash(es)");
    }
}

/// Far below the client's 30 second read timeout, so the only bound that can
/// end a stalled call this quickly is the whole-attempt one.
const TIMEOUT: Duration = Duration::from_millis(300);

/// A deadline that stopped at the response head would never end a body stalled
/// after it, nor one trickled a byte at a time so that no single read stalls:
/// the read timeout ends the first only after 30 seconds and never fires on the
/// second.
#[tokio::test]
async fn the_timeout_covers_a_body_that_stalls_or_trickles() {
    let answer = format!(r#"{{"databases":[],"pad":"{}"}}"#, "x".repeat(100));
    for (shape, route) in [
        ("stalled", Route::ok(answer.clone()).stalling_after(8)),
        ("trickled", Route::ok(answer.clone()).trickling(Duration::from_millis(20))),
    ] {
        let stub = Stub::start([(LIST.to_owned(), route)]).await;
        let client = stub.client().retries(0).timeout(TIMEOUT).build().expect("build");

        let (err, took) = failing(client.database().list()).await;

        assert_timed_out(shape, &err);
        assert!(took >= TIMEOUT && took < TIMEOUT * 3, "{shape}: the timeout fired after {took:?}");
    }
}

/// Every call reads the client's timeout separately, and one that forgot would
/// still compile. The origin answers nothing for two seconds, so a call left
/// unbounded answers late rather than failing.
#[tokio::test]
async fn the_timeout_bounds_every_database_call() {
    let late = Duration::from_secs(2);
    let stub = Stub::start([
        (LIST.to_owned(), Route::ok(r#"{"databases":[]}"#).waiting(late)),
        (METADATA.to_owned(), Route::ok("{}").waiting(late)),
        (CHECKSUM.to_owned(), Route::ok("{}").waiting(late)),
        (DOWNLOADS.to_owned(), Route::ok(r#"{"downloads":[]}"#).waiting(late)),
        (
            DOWNLOAD.to_owned(),
            Route::json(302, "").header("Location", "https://s3.example/x").waiting(late),
        ),
    ])
    .await;
    let client = stub.client().retries(0).timeout(TIMEOUT).build().expect("build");
    let database = client.database();

    for (call, (err, took)) in [
        ("list", failing(database.list()).await),
        ("metadata", failing(database.metadata("bogon_ip_v1")).await),
        ("checksums", failing(database.checksums("bogon_ip_v1", DatabaseFormat::Csvgz)).await),
        ("downloads", failing(database.downloads(None)).await),
        (
            "download_url",
            failing(database.download_url("bogon_ip_v1", DatabaseFormat::Csvgz)).await,
        ),
    ] {
        assert_timed_out(call, &err);
        assert!(took < late, "{call} waited {took:?}: the client's timeout never fired");
    }
}

/// Retried like any transport failure, and each attempt gets the whole bound.
#[tokio::test]
async fn each_attempt_gets_the_whole_timeout() {
    let stub =
        Stub::start([(LIST.to_owned(), Route::ok(r#"{"databases":[]}"#).stalling_after(4))]).await;
    let client = stub.client().retries(1).timeout(TIMEOUT).build().expect("build");

    let (err, took) = failing(client.database().list()).await;

    assert_eq!(stub.count(), 2, "a timed-out attempt was not retried");
    assert_timed_out("list", &err);
    assert!(
        took >= TIMEOUT * 2 && took < Duration::from_secs(5),
        "two attempts ended after {took:?}"
    );
}

#[tokio::test]
async fn a_zero_timeout_is_refused_at_build() {
    let err = Client::builder().timeout(Duration::ZERO).build().expect_err("a zero timeout built");
    assert!(matches!(err, Error::Config(_)), "{err}");
}

/// Awaits a call that must fail, and how long it took to.
async fn failing<T: std::fmt::Debug>(
    call: impl Future<Output = Result<T, Error>>,
) -> (Error, Duration) {
    let start = Instant::now();
    let err = call.await.expect_err("a stalled origin cannot have answered");
    (err, start.elapsed())
}

/// A timeout is the crate's own retryable transport error, and says it timed out.
fn assert_timed_out(call: &str, err: &Error) {
    assert_eq!(err.kind(), ErrorKind::Network, "{call}: {err}");
    assert!(err.retryable(), "{call}: a timeout is worth another attempt");
    assert!(err.message().contains("timed out"), "{call}: {}", err.message());
}
