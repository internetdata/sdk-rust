// Asserts the shared conformance corpus that every InternetData SDK asserts.
//
// The corpus is generated into testdata/ and is identical across languages, so a
// behavior that drifts here fails here rather than surfacing as two client
// libraries quietly disagreeing about the same address or the same refusal.

mod support;

use std::collections::BTreeSet;
use std::time::Duration;

use internetdata::{
    BatchOptions, Client, Database, DatabaseFormat, Error, LicenseType, LookupResponse, Outcome,
    Standing, is_bogon,
};
use serde_json::{Value, json};
use support::corpus::{self, Corpus, ErrorCase};
use support::{Route, Stub};

const METADATA: &str = "/api/v2/database/metadata";
const LIST: &str = "/api/v2/database/list";
const PROBE: &str = "146.70.116.130";

/// Every refusal the API can give, mapped the same way in every SDK and on both
/// kinds of call: a lookup refusal names itself in `error`, a database one in
/// `rc`, and either can reach either call. The two 429s differ ONLY by the
/// presence of `Retry-After`, and every 4xx outside 401, 403 and 429 is a CLIENT
/// error: letting one fall through to the retryable `server_error` default is
/// what three of the four VPNDetection SDKs shipped.
#[tokio::test]
async fn every_refusal_is_classified_the_way_the_corpus_says() {
    let cases = corpus::load().errors;
    assert!(!cases.is_empty(), "the corpus has no error cases, so this test asserts nothing");

    for case in cases {
        for call in CALLS {
            let stub = Stub::start([(call.path().to_owned(), refusal(&case))]).await;
            // No retries, so a retryable failure surfaces rather than looping.
            let client = stub.client().retries(0).build().expect("build");

            let err = call.run(&client).await.expect_err(&case.name);

            let name = format!("{} on {}", case.name, call.name());
            assert_eq!(err.kind().as_str(), case.expect.kind, "{name}: kind");
            assert_eq!(err.retryable(), case.expect.retryable, "{name}: retryable");
            assert_eq!(err.status(), Some(case.status), "{name}: status");
            if let Some(message) = &case.expect.message {
                assert_eq!(&err.message(), message, "{name}: message");
            }
            assert_eq!(
                err.retry_after(),
                case.expect.retry_after_seconds.map(Duration::from_secs),
                "{name}: retry_after"
            );
        }
    }
}

/// A refusal the corpus marks non-retryable must cost exactly one request. The
/// classification test above runs with retries off, so on its own it would pass
/// for a client that retried everything.
#[tokio::test]
async fn a_non_retryable_refusal_is_issued_exactly_once() {
    for case in corpus::load().errors.into_iter().filter(|case| !case.expect.retryable) {
        for call in CALLS {
            let stub = Stub::start([(call.path().to_owned(), refusal(&case))]).await;
            let client = stub.client().no_cache().retries(3).build().expect("build");

            call.run(&client).await.expect_err(&case.name);

            assert_eq!(
                stub.count(),
                1,
                "{} on {}: a non-retryable refusal was retried",
                case.name,
                call.name()
            );
        }
    }
}

/// The two kinds of call every refusal runs against.
#[derive(Clone, Copy)]
enum Call {
    Lookup,
    Metadata,
}

const CALLS: [Call; 2] = [Call::Lookup, Call::Metadata];

impl Call {
    fn name(self) -> &'static str {
        match self {
            Self::Lookup => "a lookup",
            Self::Metadata => "a database call",
        }
    }

    fn path(self) -> &'static str {
        match self {
            Self::Lookup => "/146.70.116.130",
            Self::Metadata => METADATA,
        }
    }

    async fn run(self, client: &Client) -> Result<(), Error> {
        match self {
            Self::Lookup => client.lookup(PROBE).await.map(drop),
            Self::Metadata => client.database().metadata("any_database_v1").await.map(drop),
        }
    }
}

fn refusal(case: &ErrorCase) -> Route {
    let mut route = Route::json(case.status, case.body.to_string());
    for (name, value) in &case.headers {
        route = route.header(name, value);
    }
    route
}

#[test]
fn is_bogon_matches_the_canonical_ranges() {
    let cases = corpus::load().is_bogon;
    assert!(!cases.is_empty(), "the corpus has no bogon cases");
    for case in cases {
        assert_eq!(
            is_bogon(&case.ip),
            case.expect,
            "is_bogon({:?}) should be {} ({})",
            case.ip,
            case.expect,
            case.why
        );
    }
}

// A listener on :: sees an IPv4 visitor as ::ffff:a.b.c.d, which read whole is
// inside ::ffff:0:0/96, so an SDK that judged it whole answered every such
// visitor as a bogon with no request made.
#[tokio::test]
async fn an_ipv4_mapped_address_is_the_ipv4_address_it_carries() {
    for case in corpus::load().ipv4_mapped {
        assert_eq!(is_bogon(&case.ip), case.expect, "is_bogon({:?}) ({})", case.ip, case.why);
        let body = support::answer(&case.carries);
        let routes = || [(format!("/{}", case.carries), Route::ok(body.clone()))];
        let want_sent: Vec<String> =
            if case.expect { Vec::new() } else { vec![case.carries.clone()] };

        let stub = Stub::start(routes()).await;
        let client = stub.client().build().expect("build");
        let result = client.lookup(&case.ip).await.unwrap_or_else(|e| panic!("{}: {e}", case.ip));
        assert_eq!(result.is_bogon, case.expect, "{}", case.ip);
        assert_eq!(result.ip, case.carries, "{}", case.ip);
        client.lookup(&case.carries).await.expect("a lookup of the carried address");
        let paths: Vec<String> = stub.requests().into_iter().map(|call| call.path).collect();
        let want_paths: Vec<String> = want_sent.iter().map(|ip| format!("/{ip}")).collect();
        assert_eq!(paths, want_paths, "{}: one request, cached as {}", case.ip, case.carries);

        let batch_stub = Stub::start(routes()).await;
        let uncached = batch_stub.client().no_cache().build().expect("build");
        let mut asked = vec![case.ip.clone()];
        if case.carries != case.ip {
            asked.push(case.carries.clone());
        }
        let got = uncached.lookup_batch(&asked, BatchOptions::new()).await;
        assert_eq!(got.keys().cloned().collect::<Vec<_>>(), asked, "{}", case.ip);
        let answer = got[&case.ip].as_ref().unwrap_or_else(|e| panic!("{}: {e}", case.ip));
        assert_eq!(answer.ip, case.carries, "{}", case.ip);
        let sent: Vec<String> = batch_stub
            .requests()
            .into_iter()
            .filter_map(|call| serde_json::from_str::<Value>(&call.body).ok())
            .flat_map(|body| body["ips"].as_array().cloned().unwrap_or_default())
            .filter_map(|ip| ip.as_str().map(str::to_owned))
            .collect();
        assert_eq!(sent, want_sent, "{}", case.ip);
    }
}

/// The corpus answer with the address filled in, member for member: a flag or a
/// detail object left absent, or a member read as some other value, differs.
#[tokio::test]
async fn a_bogon_is_answered_locally_in_the_widest_shape() {
    let data = corpus::load();
    let stub = Stub::start([]).await;
    let client = stub.client().build().expect("build");

    for ip in ["10.0.0.1", "fe80::1", "::ffff:192.168.1.1"] {
        let result = client.lookup(ip).await.expect("lookup");

        let mut want = data.bogon_response.answer.clone();
        let carried = ip.strip_prefix("::ffff:").unwrap_or(ip);
        want.insert("ip".to_owned(), json!(carried));
        let want: LookupResponse =
            serde_json::from_value(Value::Object(want)).expect("the corpus answer decodes");
        assert_eq!(result.answer, want, "{ip}: {}", data.bogon_response.why);
        assert!(result.is_bogon, "{ip}: is_bogon marks a locally computed answer");
    }
    assert_eq!(stub.count(), 0, "a bogon must not reach the network");
}

/// Absent is "not in your plan" and present-and-false is "checked, and no", for
/// every member, the paid ones inside a shared object included. Each served
/// answer decodes and re-encodes as it was served, so nothing was dropped,
/// defaulted or invented on the way through.
#[tokio::test]
async fn a_lookup_preserves_absent_versus_false_across_every_plan_shape() {
    let cases = corpus::load().lookup;
    for call in ["lookup", "lookupAsn", "lookupDomain"] {
        assert!(cases.iter().any(|case| case.call == call), "the corpus has no {call} case");
    }

    for case in cases {
        let target = match &case.target {
            Value::String(target) => target.clone(),
            other => other.to_string(),
        };
        let path = match case.call.as_str() {
            "lookup" => format!("/{target}"),
            "lookupAsn" => format!("/asn/{target}"),
            "lookupDomain" => format!("/domain/{target}"),
            call => panic!("{}: a call this suite does not know: {call}", case.name),
        };
        let stub = Stub::start([(path, Route::json(case.status, case.body.to_string()))]).await;
        let client = stub.client().build().expect("build");

        let wire = match case.call.as_str() {
            "lookup" => to_wire(client.lookup(&target).await.map(|result| result.answer)),
            "lookupAsn" => {
                let asn = case.target.as_u64().and_then(|asn| u32::try_from(asn).ok());
                to_wire(client.lookup_asn(asn.expect("an AS number")).await)
            }
            _ => to_wire(client.lookup_domain(&target).await),
        }
        .unwrap_or_else(|e| panic!("{}: {e}", case.name));

        let name = &case.name;
        assert_eq!(wire, case.body, "{name}: the answer does not re-encode as served");
        for (member, want) in &case.expect.present {
            let got = wire.get(member);
            assert_eq!(got, Some(want), "{name}: {member} must be present and {want}");
        }
        for member in &case.expect.objects {
            let got = wire.get(member);
            let container = got.is_some_and(|value| value.is_object() || value.is_array());
            assert!(container, "{name}: {member} must be present, an object or a list");
        }
        for member in &case.expect.empty {
            let got = wire.get(member);
            let empty = got.is_some_and(|value| *value == json!({}) || *value == json!([]));
            assert!(empty, "{name}: {member} must be present and EMPTY, not {got:?}");
        }
        for member in &case.expect.absent {
            let got = wire.get(member);
            assert!(got.is_none(), "{name}: {member} must be ABSENT, not {got:?}");
        }
        for (object, members) in &case.expect.absent_in {
            let held = wire.get(object).unwrap_or_else(|| panic!("{name}: {object} is absent"));
            for member in members {
                let got = held.get(member);
                assert!(got.is_none(), "{name}: {object}.{member} must be ABSENT, not {got:?}");
            }
        }
    }
}

/// Serializing the wire answer is what pins the names the API actually serves:
/// an absent option is missing from the JSON, a present one is there and false.
fn to_wire<T: serde::Serialize>(answer: Result<T, Error>) -> Result<Value, Error> {
    answer.map(|answer| serde_json::to_value(answer).expect("serializing the answer"))
}

#[tokio::test]
async fn a_batch_dedupes_short_circuits_bogons_and_keys_by_address() {
    let data = corpus::load();
    let case = data.batch_case("dedup-bogon-and-order-free-keying");
    let stub = Stub::start(Stub::ok_routes(&["1.1.1.1", "8.8.8.8"])).await;
    let client = stub.client().build().expect("build");

    let got = client.lookup_batch(&case.input, BatchOptions::new()).await;

    // Keyed by address, not positional, and in the order each was first seen.
    assert_eq!(got.keys().cloned().collect::<Vec<_>>(), case.expect.keys);
    assert_eq!(stub.count(), case.expect.http_requests.expect("httpRequests"));
    for ip in &case.expect.bogon_keys {
        let answer = got[ip].as_ref().unwrap_or_else(|e| panic!("{ip}: {e}"));
        assert!(answer.is_bogon, "{ip} should be a local answer");
    }
}

#[tokio::test]
async fn one_bad_address_does_not_lose_the_rest_of_the_batch() {
    let data = corpus::load();
    let case = data.batch_case("partial-failure-does-not-fail-the-batch");
    let stub = Stub::start(Stub::ok_routes(&["1.1.1.1"])).await;
    let client = stub.client().retries(0).build().expect("build");

    let got = client.lookup_batch(&case.input, BatchOptions::new()).await;

    assert_eq!(got.keys().cloned().collect::<Vec<_>>(), case.expect.keys);
    for ip in &case.expect.error_keys {
        let err = got[ip].as_ref().expect_err(&format!("{ip} should carry its own error"));
        assert_eq!(err.kind().as_str(), case.expect.error_kinds[ip], "{ip}");
    }
    let good = got["1.1.1.1"].as_ref().expect("the good address should still have answered");
    assert_eq!(good.ip, "1.1.1.1");
}

#[tokio::test]
async fn a_cache_hit_issues_no_second_request() {
    let data = corpus::load();
    let case = data.batch_case("cache-hit-issues-no-second-request");
    let stub = Stub::start(Stub::ok_routes(&["1.1.1.1"])).await;
    let client = stub.client().build().expect("build");

    for _ in 0..case.repeat.unwrap_or(1) {
        client.lookup_batch(&case.input, BatchOptions::new()).await;
    }
    assert_eq!(stub.count(), case.expect.http_requests.expect("httpRequests"));
}

/// Both the boundary and the absence of any cap: a batch of any size is chunks
/// of 1000, never a refusal.
#[tokio::test]
async fn a_large_batch_is_sent_in_chunks_of_a_thousand() {
    let data = corpus::load();
    for name in ["chunks-of-one-thousand", "uncapped-input-is-chunked"] {
        let case = data.batch_case(name);
        let addrs: Vec<&str> = case.input.iter().map(String::as_str).collect();
        let stub = Stub::start(Stub::ok_routes(&addrs)).await;
        let client = stub.client().no_cache().build().expect("build");

        let got = client.lookup_batch(&case.input, BatchOptions::new()).await;

        assert_eq!(got.len(), case.expect.key_count.expect("keyCount"), "{name}");
        assert_eq!(stub.count(), case.expect.http_requests.expect("httpRequests"), "{name}");
        for ip in &case.input {
            let answer = got[ip].as_ref().unwrap_or_else(|e| panic!("{name}: {ip}: {e}"));
            assert_eq!(&answer.ip, ip, "{name}: {ip} should be answered for itself");
        }
    }
}

/// A per-entry failure carries no headers, so its 429 can only be a spent
/// allowance, and a 500 is the server's; neither is retried per entry, because
/// retries belong to the call and the call succeeded.
#[tokio::test]
async fn an_entry_error_is_classified_by_its_status() {
    let data = corpus::load();
    let case = data.batch_case("an-entry-error-is-classified-by-its-status");
    let mut routes = Stub::ok_routes(&["1.1.1.1"]);
    routes.push((
        "/8.8.8.8".to_owned(),
        Route::json(429, r#"{"error":"overage cap reached; raise or remove it in Billing"}"#),
    ));
    routes.push(("/9.9.9.9".to_owned(), Route::json(500, r#"{"error":"lookup failed"}"#)));
    let stub = Stub::start(routes).await;
    let client = stub.client().retries(3).build().expect("build");

    let got = client.lookup_batch(&case.input, BatchOptions::new()).await;

    assert_eq!(got.keys().cloned().collect::<Vec<_>>(), case.expect.keys);
    for (ip, kind) in &case.expect.error_kinds {
        let err = got[ip].as_ref().expect_err(&format!("{ip} should carry its error"));
        assert_eq!(err.kind().as_str(), kind, "{ip}");
    }
    assert_eq!(stub.count(), case.expect.http_requests.expect("httpRequests"));
}

#[tokio::test]
async fn two_clients_never_share_a_cached_answer() {
    let stub = Stub::start(Stub::ok_routes(&["1.1.1.1"])).await;
    let a = stub.client().api_key("key-a").build().expect("build");
    let b = stub.client().api_key("key-b").build().expect("build");

    a.lookup("1.1.1.1").await.expect("lookup");
    b.lookup("1.1.1.1").await.expect("lookup");

    // Two keys can be on different plans and so entitled to different fields; a
    // shared cache would serve one of them the other's shape.
    assert_eq!(stub.count(), 2);
}

#[tokio::test]
async fn caching_can_be_turned_off() {
    let stub = Stub::start(Stub::ok_routes(&["1.1.1.1"])).await;
    let client = stub.client().no_cache().build().expect("build");

    client.lookup("1.1.1.1").await.expect("lookup");
    client.lookup("1.1.1.1").await.expect("lookup");

    assert_eq!(stub.count(), 2);
}

/// Every generated range has to be reachable through the predicate, which is the
/// only public surface the table has. The exact table-versus-corpus comparison
/// is a unit test in src/bogon.rs, where the `pub(crate)` table is visible.
#[test]
fn every_generated_range_is_reachable_through_the_predicate() {
    let data: Corpus = corpus::load();
    for cidr in data.bogons.v4.iter().chain(data.bogons.v6.iter()) {
        let host = cidr.split('/').next().expect("a cidr has a network part");
        assert!(is_bogon(host), "{cidr}: the network address should be a bogon");
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

/// A version's built formats and a checksum's both `$ref` one named schema, so
/// there is a single enum here and no cross-conversion left to pin.
#[test]
fn the_format_vocabulary_is_exactly_what_the_corpus_declares() {
    let expected = set(&corpus::load().formats);
    assert_eq!(spellings(&[DatabaseFormat::Csvgz, DatabaseFormat::Mmdb]), expected);
    assert_readers_match_the_wire(&[DatabaseFormat::Csvgz, DatabaseFormat::Mmdb]);
}

/// Every visibility rule the corpus names has a test here, and a rule it grows
/// later fails this until one is written. The rules are what is pinned rather
/// than the private IDs they protect: this corpus is committed into public
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

/// One family, shaped exactly as the API serves it: a license held against the
/// FAMILY, and the IDs a download takes one level down in `versions`.
fn listing(bases: &[&str]) -> String {
    let databases: Vec<Value> = bases
        .iter()
        .map(|base| {
            json!({
                "base": base,
                "name": base,
                "summary": "one line",
                "standing": "licensed",
                "open": false,
                "license_type": "standard",
                "starts": "2026-01-01T00:00:00.000Z",
                "expires": null, "renews_at": null, "notice_due_at": null,
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
