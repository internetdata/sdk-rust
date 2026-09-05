// Asserts the shared conformance corpus that every InternetData SDK asserts.
//
// The corpus is generated into testdata/ and is identical across languages, so a
// behavior that drifts here fails here rather than surfacing as two client
// libraries quietly disagreeing about the same refusal.

mod support;

use std::collections::BTreeSet;
use std::time::Duration;

use internetdata::{Database, Format, Formats, Outcome, LicenseType, Standing};
use serde_json::{Value, json};
use support::corpus;
use support::{Route, Stub};

const METADATA: &str = "/api/v2/database/metadata";
const LIST: &str = "/api/v2/database/list";

/// Every refusal the API can give, mapped the same way in every SDK. The two
/// 429s differ ONLY by the presence of `Retry-After`, and the two 404s are
/// CLIENT errors: letting a 4xx fall through to the retryable `server_error`
/// default is what three of the four VPNDetection SDKs shipped.
#[tokio::test]
async fn every_refusal_is_classified_the_way_the_corpus_says() {
    let cases = corpus::load().errors;
    assert!(!cases.is_empty(), "the corpus has no error cases, so this test asserts nothing");

    for case in cases {
        let mut route = Route::json(case.status, case.body.to_string());
        for (name, value) in &case.headers {
            route = route.header(name, value);
        }
        let stub = Stub::start([(METADATA.to_owned(), route)]).await;
        // No retries, so a retryable failure surfaces rather than looping.
        let client = stub.client().retries(0).build().expect("build");

        let err = client.database().metadata("any_database_v1").await.expect_err(&case.name);

        assert_eq!(err.kind().as_str(), case.expect.kind, "{}: kind", case.name);
        assert_eq!(err.retryable(), case.expect.retryable, "{}: retryable", case.name);
        assert_eq!(err.status(), Some(case.status), "{}: status", case.name);
        if let Some(message) = &case.expect.message {
            assert_eq!(&err.message(), message, "{}: message", case.name);
        }
        assert_eq!(
            err.retry_after(),
            case.expect.retry_after_seconds.map(Duration::from_secs),
            "{}: retry_after",
            case.name
        );
    }
}

/// A refusal the corpus marks non-retryable must cost exactly one request. The
/// classification test above runs with retries off, so on its own it would pass
/// for a client that retried everything.
#[tokio::test]
async fn a_non_retryable_refusal_is_issued_exactly_once() {
    for case in corpus::load().errors.into_iter().filter(|case| !case.expect.retryable) {
        let mut route = Route::json(case.status, case.body.to_string());
        for (name, value) in &case.headers {
            route = route.header(name, value);
        }
        let stub = Stub::start([(METADATA.to_owned(), route)]).await;
        let client = stub.client().retries(3).build().expect("build");

        client.database().metadata("any_database_v1").await.expect_err(&case.name);

        assert_eq!(stub.count(), 1, "{}: a non-retryable refusal was retried", case.name);
    }
}

#[test]
fn the_standing_vocabulary_is_exactly_what_the_corpus_declares() {
    let wire = [Standing::Licensed, Standing::Expired, Standing::Unlicensed];
    assert_eq!(spellings(&wire), set(&corpus::load().standings));
    assert_readers_match_the_wire(&wire);
}

#[test]
fn the_license_type_vocabulary_is_exactly_what_the_corpus_declares() {
    let wire = [LicenseType::Evaluation, LicenseType::Standard, LicenseType::Redistribute];
    assert_eq!(spellings(&wire), set(&corpus::load().license_type));
    assert_readers_match_the_wire(&wire);
}

/// `Outcome` is not in the corpus - a download history is a per-organization
/// answer rather than a shared fixture - but its readers can drift the same way,
/// so they are held to the same rule.
#[test]
fn the_outcome_readers_agree_with_the_wire() {
    assert_readers_match_the_wire(&[
        Outcome::Ok,
        Outcome::Unauthorized,
        Outcome::Denied,
        Outcome::Expired,
        Outcome::Unknown,
        Outcome::Unavailable,
    ]);
}

/// The spec spells the same two formats twice, once for a version's built
/// formats and once for a checksum's, so the crate carries two enums for it.
/// Both are pinned, and so is the conversion between them.
#[test]
fn the_format_vocabulary_is_exactly_what_the_corpus_declares() {
    let expected = set(&corpus::load().formats);
    assert_eq!(spellings(&[Format::Csvgz, Format::Mmdb]), expected);
    assert_eq!(spellings(&[Formats::Csvgz, Formats::Mmdb]), expected);

    for (built, single) in [(Formats::Csvgz, Format::Csvgz), (Formats::Mmdb, Format::Mmdb)] {
        assert_eq!(Format::from(built), single, "{built} does not convert to itself");
        assert_eq!(Formats::from(single), built, "{single} does not convert back");
    }
    assert_readers_match_the_wire(&[Format::Csvgz, Format::Mmdb]);
    assert_readers_match_the_wire(&[Formats::Csvgz, Formats::Mmdb]);
}

/// Every visibility rule the corpus names has a test here, and a rule it grows
/// later fails this until one is written. The rules are what is pinned rather
/// than the private ids they protect: this corpus is committed into public
/// repositories, so naming those would publish the customer relationship the
/// contract exists to keep private.
#[test]
fn every_visibility_rule_the_corpus_names_is_asserted_here() {
    let asserted = set(&[
        "listing-is-returned-as-served",
        "no-catalog-is-compiled-into-the-client",
        "a-listing-is-never-reused-across-clients",
    ]);
    let declared = set(&corpus::load().visibility.client_rules);
    assert!(!declared.is_empty(), "the corpus declares no visibility rules");
    assert_eq!(
        declared, asserted,
        "a visibility rule is declared with no test, or tested with no declaration"
    );
}

/// `listing-is-returned-as-served`. A private family is ABSENT from a listing
/// for an organization that does not license it, rather than present with an
/// `unlicensed` standing, so the client must hand back exactly what the server
/// sent: neither dropping an entry nor inventing one, and in the server's order.
#[tokio::test]
async fn a_listing_is_returned_as_served() {
    let served = ["bogon_asn", "bogon_ip", "cdn_ip"];
    let stub = Stub::start([(LIST.to_owned(), Route::ok(listing(&served)))]).await;
    let client = stub.client().build().expect("build");

    let databases = client.database().list().await.expect("list");

    let got: Vec<&str> = databases.iter().map(|database| database.base.as_str()).collect();
    assert_eq!(got, served, "{}", corpus::load().visibility.why);
}

/// `no-catalog-is-compiled-into-the-client`. The client holds no idea of what is
/// published, so a family it has never heard of survives the decode and an empty
/// listing stays empty: a compiled-in catalog would show through in one of the
/// two.
#[tokio::test]
async fn no_catalog_is_compiled_into_the_client() {
    let unheard_of = ["a_database_this_crate_has_never_heard_of"];
    let stub = Stub::start([(LIST.to_owned(), Route::ok(listing(&unheard_of)))]).await;
    let client = stub.client().build().expect("build");

    let databases = client.database().list().await.expect("list");
    assert_eq!(bases(&databases), set(&unheard_of), "an unknown family did not survive the decode");

    let empty = Stub::start([(LIST.to_owned(), Route::ok(r#"{"databases":[]}"#))]).await;
    let client = empty.client().build().expect("build");

    assert!(
        client.database().list().await.expect("list").is_empty(),
        "an organization that licenses nothing was shown a catalog from somewhere else"
    );
}

/// `a-listing-is-never-reused-across-clients`. Two keys belong to two
/// organizations and see two different catalogs, so a listing must never be
/// cached anywhere a second client could read it. Nothing static, nothing
/// process-wide.
#[tokio::test]
async fn a_listing_is_never_reused_across_clients() {
    let stub = Stub::start([(LIST.to_owned(), Route::ok(listing(&["bogon_asn"])))]).await;
    let first = stub.client().api_key("key-of-one-org").build().expect("build");
    first.database().list().await.expect("list");

    // The same endpoint now answers what the OTHER organization may see. A
    // client that reused the first answer would report the first org's catalog.
    stub.route(LIST, Route::ok(listing(&["cdn_ip", "hosting_ip"])));
    let second = stub.client().api_key("key-of-another-org").build().expect("build");

    let databases = second.database().list().await.expect("list");

    assert_eq!(bases(&databases), set(&["cdn_ip", "hosting_ip"]));
    assert_eq!(stub.count(), 2, "the second client did not ask for its own listing");
}

/// One family, shaped exactly as the API serves it: a licence held against the
/// FAMILY, and the ids a download takes one level down in `versions`.
fn listing(bases: &[&str]) -> String {
    let databases: Vec<Value> = bases
        .iter()
        .map(|base| {
            json!({
                "base": base,
                "name": base,
                "summary": "one line",
                "standing": "licensed",
                "license_type": "standard",
                "starts": "2026-01-01T00:00:00.000Z",
                "expires": null,
                "versions": [{
                    "id": format!("{base}_v1"),
                    "version": 1,
                    "summary": "one line",
                    "formats": ["csvgz"],
                }],
            })
        })
        .collect();
    json!({ "databases": databases }).to_string()
}

fn bases(databases: &[Database]) -> BTreeSet<String> {
    databases.iter().map(|database| database.base.clone()).collect()
}

/// The wire spellings of an enum's variants, read through serde rather than
/// from a hand-written table, so the corpus is compared against what the crate
/// would actually send and decode.
fn spellings<T: serde::Serialize>(variants: &[T]) -> BTreeSet<String> {
    variants
        .iter()
        .map(|variant| {
            serde_json::to_value(variant)
                .expect("serializing a variant")
                .as_str()
                .expect("an enum variant serializes to a string")
                .to_owned()
        })
        .collect()
}

/// `as_str` and `Display` are hand-written per variant, so they can drift from
/// the value serde actually sends. Printing a standing the API does not use is
/// the kind of wrong nothing else here would catch.
fn assert_readers_match_the_wire<T>(variants: &[T])
where
    T: serde::Serialize + std::fmt::Display + Copy,
{
    for variant in variants {
        let sent = serde_json::to_value(variant)
            .expect("serializing a variant")
            .as_str()
            .expect("an enum variant serializes to a string")
            .to_owned();
        assert_eq!(variant.to_string(), sent, "Display disagrees with the wire spelling");
    }
}

fn set(values: &[impl AsRef<str>]) -> BTreeSet<String> {
    values.iter().map(|value| value.as_ref().to_owned()).collect()
}
