// The Rust-specific API surface, as distinct from the shared conformance corpus
// in conformance.rs: what each call unwraps, what it sends, and what the retry
// loop does with a 429.

mod support;

use std::time::{Duration, Instant};

use internetdata::{Client, ErrorKind, DatabaseFormat, Outcome, Standing};
use support::{KEY, Route, Stub};

const LIST: &str = "/api/v2/database/list";
const METADATA: &str = "/api/v2/database/metadata";
const CHECKSUM: &str = "/api/v2/database/checksum";
const DOWNLOADS: &str = "/api/v2/database/downloads";
const DOWNLOAD: &str = "/api/v2/database/download";

/// A licence is held against a FAMILY, and the ids a download takes are one
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
    assert!(family.expires.is_none(), "a licence with no end date reads as None");
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

    let sums = client.database().checksums("bogon_ip_v1", DatabaseFormat::Mmdb).await.expect("checksums");

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

    let url =
        client.database().download_url("bogon_ip_v1", DatabaseFormat::Mmdb).await.expect("download_url");

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

    let url =
        client.database().download_url("bogon_ip_v1", DatabaseFormat::Csvgz).await.expect("download_url");

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
/// offered without a licence would need exactly this client. The empty arm is
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
    let stub = Stub::start([(LIST.to_owned(), Route::ok(r#"{"databases":[]}"#))]).await;
    let client = Client::builder()
        .base_url(format!("{}/", stub.base_url))
        .api_key(KEY)
        .build()
        .expect("build");

    client.database().list().await.expect("list");

    assert_eq!(stub.calls(), vec![LIST]);
}
