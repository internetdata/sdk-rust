// The lookups' Rust surface, as distinct from the shared conformance corpus in
// conformance.rs: the batch's concurrency and chunks, each call's own options,
// the cache sharing one request per address, the self-endpoints, the AS and
// domain lookups, and the host the OAuth requests go to.

mod support;

use std::time::{Duration, Instant};

use internetdata::{BatchOptions, Client, Error, ErrorKind, LookupOptions};
use support::{KEY, Route, Stub, answer, answer_value};

/// An origin that holds every request this long before answering, so only a
/// timeout ends the wait, and how long a call took says WHICH timeout fired.
const STALL: Duration = Duration::from_secs(60);
/// Far above the per-call value, so a per-call timeout that was accepted and
/// ignored shows up as a call that ran long.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);
const CALL_TIMEOUT: Duration = Duration::from_millis(150);
/// Short enough to wait for, long above `CALL_TIMEOUT`, and far below the
/// client's 30 second read timeout, so only the whole-attempt deadline fits.
const BODY_CLIENT_TIMEOUT: Duration = Duration::from_secs(1);
/// Long enough that a call started a quarter of it later is still concurrent.
const STAGGER: Duration = Duration::from_millis(400);

/// Enough addresses for seven chunks of the batch endpoint's 1000, so a
/// concurrency bound has something to bound: one request per chunk, and only
/// the chunks overlap.
fn many_addrs() -> Vec<String> {
    (0..6001).map(|i| format!("9.{}.{}.{}", 1 + i / 65536, (i / 256) % 256, i % 256)).collect()
}

fn many_routes() -> Vec<(String, Route)> {
    let addrs = many_addrs();
    Stub::ok_routes(&addrs.iter().map(String::as_str).collect::<Vec<_>>())
}

fn served(ip: &str) -> (String, Route) {
    (format!("/{ip}"), Route::ok(answer(ip)))
}

/// Peak in-flight is the only measurement that tells a real limit from an option
/// that was accepted and ignored.
#[tokio::test]
async fn batch_concurrency_is_configurable_per_call() {
    let stub = Stub::start_with_delay(many_routes(), Duration::from_millis(20)).await;
    let client = stub.client().no_cache().build().expect("build");

    client.lookup_batch(many_addrs(), BatchOptions::new().concurrency(3)).await;

    assert_eq!(stub.count(), 7, "one request per chunk of 1000");
    assert!(
        stub.peak_in_flight() <= 3,
        "peak in flight was {}, want at most 3",
        stub.peak_in_flight()
    );
    assert!(stub.peak_in_flight() > 1, "requests never overlapped");
}

#[tokio::test]
async fn a_per_call_concurrency_overrides_the_client_default() {
    let stub = Stub::start_with_delay(many_routes(), Duration::from_millis(20)).await;
    let client = stub.client().no_cache().concurrency(2).build().expect("build");

    client.lookup_batch(many_addrs(), BatchOptions::new().concurrency(6)).await;

    let peak = stub.peak_in_flight();
    assert!(peak > 2, "override ignored: peak in flight was {peak}, want above 2");
    assert!(peak <= 6, "peak in flight was {peak}, want at most 6");
}

#[tokio::test]
async fn without_an_override_the_client_concurrency_still_applies() {
    let stub = Stub::start_with_delay(many_routes(), Duration::from_millis(20)).await;
    let client = stub.client().no_cache().concurrency(2).build().expect("build");

    client.lookup_batch(many_addrs(), BatchOptions::new()).await;

    assert!(stub.peak_in_flight() <= 2, "peak in flight was {}", stub.peak_in_flight());
}

/// Zero would leave nothing to run the chunks, so the whole batch is refused as a
/// bad request, every address answered with that refusal, before any request.
#[tokio::test]
async fn a_per_call_concurrency_below_one_is_refused_before_any_request() {
    let stub = Stub::start(Stub::ok_routes(&["1.1.1.1", "9.9.9.9"])).await;
    let client = stub.client().no_cache().build().expect("build");

    // Bounded: a limit of zero handed to the stream never polls a chunk at all,
    // so without the refusal this call would wait forever.
    let got = tokio::time::timeout(
        Duration::from_secs(10),
        client.lookup_batch(["1.1.1.1", "10.0.0.1", "9.9.9.9"], BatchOptions::new().concurrency(0)),
    )
    .await
    .expect("a batch at concurrency 0 never settled");

    assert_eq!(stub.count(), 0, "a refused batch still sent a request");
    assert_eq!(got.keys().collect::<Vec<_>>(), ["1.1.1.1", "10.0.0.1", "9.9.9.9"]);
    for (ip, answer) in &got {
        let err = answer.as_ref().expect_err("a refused batch answered an address");
        assert_eq!(err.kind(), ErrorKind::BadRequest, "{ip}: {err}");
        assert!(!err.retryable(), "{ip}: retrying cannot fix the option");
    }
}

/// Chunking is the SDK's job, so a batch has no size limit of its own: 2,500
/// addresses are three requests of at most 1000, never a refusal and never a
/// request per address.
#[tokio::test]
async fn a_batch_of_any_size_is_chunked_rather_than_refused() {
    let addrs: Vec<String> = many_addrs().into_iter().take(2500).collect();
    let stub =
        Stub::start(Stub::ok_routes(&addrs.iter().map(String::as_str).collect::<Vec<_>>())).await;
    let client = stub.client().no_cache().build().expect("build");

    let got = client.lookup_batch(&addrs, BatchOptions::new()).await;

    let requests = stub.requests();
    assert_eq!(requests.len(), 3, "2500 addresses are three chunks of up to 1000");
    for call in &requests {
        assert_eq!((call.method.as_str(), call.path.as_str()), ("POST", "/batch"));
        let body: serde_json::Value = serde_json::from_str(&call.body).expect("a JSON body");
        assert_eq!(body.as_object().map(|body| body.len()), Some(1), "only ips: {body}");
        let sent = body["ips"].as_array().expect("an ips array").len();
        assert!(sent <= 1000, "a chunk carried {sent} addresses");
    }
    assert_eq!(got.len(), 2500);
    for ip in &addrs {
        let answer = got[ip].as_ref().unwrap_or_else(|e| panic!("{ip}: {e}"));
        assert_eq!(&answer.ip, ip, "{ip} should be answered for itself");
    }
}

#[tokio::test]
async fn retries_are_configurable_per_call() {
    let stub =
        Stub::start([("/9.9.9.9".to_owned(), Route::json(500, r#"{"error":"lookup failed"}"#))])
            .await;
    let client = stub.client().no_cache().retries(0).build().expect("build");

    client
        .lookup_with("9.9.9.9", LookupOptions::new().retries(2))
        .await
        .expect_err("a 500 should have failed the lookup");

    // One initial attempt plus two retries, rather than the client's zero.
    assert_eq!(stub.count(), 3);
}

/// Every call that takes `LookupOptions` is held to it, because each one reads
/// the option separately and a call that forgot would still compile.
#[tokio::test]
async fn a_per_call_timeout_below_the_clients_is_the_one_that_fires() {
    let stub = Stub::start_with_delay(Stub::ok_routes(&["9.9.9.9"]), STALL).await;
    let client =
        stub.client().no_cache().retries(0).timeout(CLIENT_TIMEOUT).build().expect("build");
    let opts = || LookupOptions::new().timeout(CALL_TIMEOUT);

    for (call, (err, took)) in [
        ("lookup_with", stalled(client.lookup_with("9.9.9.9", opts())).await),
        ("lookup_asn_with", stalled(client.lookup_asn_with(9009, opts())).await),
        ("lookup_domain_with", stalled(client.lookup_domain_with("example.com", opts())).await),
        ("my_ip_with", stalled(client.my_ip_with(opts())).await),
        ("my_entitlement_with", stalled(client.my_entitlement_with(opts())).await),
    ] {
        assert!(
            took < CLIENT_TIMEOUT / 2,
            "{call} waited {took:?}: the call's timeout was ignored"
        );
        assert_timed_out(call, &err);
    }
}

/// Per attempt at each chunk. A chunk that runs out of time marks every address
/// in it, the same as any other chunk that fails as a whole.
#[tokio::test]
async fn a_per_call_timeout_bounds_each_chunk_of_a_batch() {
    let stub = Stub::start_with_delay(Stub::ok_routes(&["1.1.1.1", "9.9.9.9"]), STALL).await;
    let client =
        stub.client().no_cache().retries(0).timeout(CLIENT_TIMEOUT).build().expect("build");

    let start = Instant::now();
    let got = client
        .lookup_batch(["1.1.1.1", "9.9.9.9"], BatchOptions::new().timeout(CALL_TIMEOUT))
        .await;

    let took = start.elapsed();
    assert!(took < CLIENT_TIMEOUT / 2, "the batch waited {took:?}: its timeout was ignored");
    assert_eq!(got.len(), 2);
    for (ip, answer) in &got {
        assert_timed_out(ip, answer.as_ref().expect_err("a stalled chunk cannot have answered"));
    }
}

/// A deadline that stopped at the response head would never end a body stalled
/// after it, nor one trickled a byte at a time so that no single read stalls.
/// The call's own value fires first, then a call without one is held to the
/// client's.
#[tokio::test]
async fn the_timeout_covers_a_body_that_stalls_or_trickles() {
    let mut padded = answer_value("9.9.9.9");
    padded["pad"] = serde_json::json!("x".repeat(400));
    let padded = padded.to_string();
    for (shape, route) in [
        ("stalled", Route::ok(padded.clone()).stalling_after(8)),
        ("trickled", Route::ok(padded.clone()).trickling(Duration::from_millis(20))),
    ] {
        let stub = Stub::start([("/9.9.9.9".to_owned(), route)]).await;
        let client = stub
            .client()
            .no_cache()
            .retries(0)
            .timeout(BODY_CLIENT_TIMEOUT)
            .build()
            .expect("build");

        let opts = LookupOptions::new().timeout(CALL_TIMEOUT);
        let (err, took) = stalled(client.lookup_with("9.9.9.9", opts)).await;
        assert_timed_out(shape, &err);
        assert!(
            took >= CALL_TIMEOUT && took < BODY_CLIENT_TIMEOUT,
            "{shape}: the call's timeout fired after {took:?}"
        );

        let (err, took) = stalled(client.lookup("9.9.9.9")).await;
        assert_timed_out(shape, &err);
        assert!(
            took >= BODY_CLIENT_TIMEOUT && took < BODY_CLIENT_TIMEOUT * 3,
            "{shape}: the client's timeout fired after {took:?}"
        );
    }
}

#[tokio::test]
async fn is_bogon_is_on_the_client_and_agrees_with_the_standalone_function() {
    let stub = Stub::start([]).await;
    let client = stub.client().build().expect("build");
    for case in support::corpus::load().is_bogon {
        assert_eq!(client.is_bogon(&case.ip), case.expect, "{} ({})", case.ip, case.why);
        assert_eq!(client.is_bogon(&case.ip), internetdata::is_bogon(&case.ip), "{}", case.ip);
    }
}

/// The single most important semantic in the library, asserted natively rather
/// than through the corpus's JSON view: a plan that omits a field leaves `None`,
/// and a plan that includes it answers `Some(false)`, inside a shared object too.
#[tokio::test]
async fn absent_and_false_are_different_values() {
    let mut paid = answer_value("8.8.4.4");
    paid["is_hosting"] = serde_json::json!(false);
    paid["location"]["city"] = serde_json::json!("");
    let stub = Stub::start([
        ("/1.1.1.1".to_owned(), Route::ok(answer("1.1.1.1"))),
        ("/8.8.4.4".to_owned(), Route::ok(paid.to_string())),
    ])
    .await;
    let client = stub.client().build().expect("build");

    let unlimited = client.lookup("1.1.1.1").await.expect("lookup");
    assert_eq!(unlimited.is_hosting, None, "a field the plan omits must be None");
    assert_eq!(unlimited.location.city, None, "a member the plan omits must be None");
    assert!(!unlimited.is_hosting_or_false());

    let served = client.lookup("8.8.4.4").await.expect("lookup");
    assert_eq!(served.is_hosting, Some(false), "a checked-and-no answer must be Some(false)");
    assert_eq!(served.location.city.as_deref(), Some(""), "served empty is not absent");
    assert!(!served.is_hosting_or_false());
}

#[tokio::test]
async fn or_false_readers_coalesce_an_absent_flag() {
    let mut body = answer_value("45.83.91.1");
    body["is_vpn"] = serde_json::json!(true);
    body["is_hosting"] = serde_json::json!(true);
    body["is_cdn"] = serde_json::json!(false);
    let stub = Stub::start([("/45.83.91.1".to_owned(), Route::ok(body.to_string()))]).await;
    let client = stub.client().build().expect("build");

    let result = client.lookup("45.83.91.1").await.expect("lookup");

    assert!(!result.is_tor, "the wire answer reads straight off the result");
    assert!(result.is_vpn_or_false());
    assert!(result.is_hosting_or_false());
    assert!(!result.is_cdn_or_false(), "a present false flag reads as false");
    assert!(!result.is_dcproxy_or_false(), "an absent flag reads as false");
    assert_eq!(result.is_dcproxy, None, "the reader must not populate what it reads");
}

/// A 2xx a call cannot read is no answer: the server_error an outage is,
/// carrying the status, and retried like one.
#[tokio::test]
async fn an_answer_a_call_cannot_read_is_a_retried_server_error() {
    const BODIES: [(&str, &str); 9] = [
        ("an HTML page", "<html>gateway</html>"),
        ("a cut-off body", r#"{"ip":"9.9.9.9","is_b"#),
        ("an empty body", ""),
        ("an array", "[]"),
        ("a string", r#""x""#),
        ("null", "null"),
        ("an empty object", "{}"),
        (
            "members of the wrong type",
            r#"{"ip":9,"is_bogon":"no","results":[],"asn":"x","domain":5,"org_id":5}"#,
        ),
        (
            "entries without their members",
            r#"{"ip":"9.9.9.9","results":{"9.9.9.9":{}},"errors":{},"asn":9009,"domain":"example.com","apikey":{}}"#,
        ),
    ];
    const CALLS: [(&str, &str); 6] = [
        ("lookup", "/9.9.9.9"),
        ("lookup_asn", "/asn/9009"),
        ("lookup_domain", "/domain/example.com"),
        ("my_ip", "/myip"),
        ("my_entitlement", "/api/v1/entitlement"),
        ("lookup_batch", "/batch"),
    ];
    // Each case waits out two backoffs, so they run side by side.
    let mut cases = tokio::task::JoinSet::new();
    for (call, path) in CALLS {
        for (name, body) in BODIES {
            cases.spawn(async move {
                let stub = Stub::start([]).await;
                stub.sequence(path, [Route::ok(body), Route::ok(body), Route::ok(body)]);
                let client = stub.client().no_cache().retries(2).build().expect("build");
                let outcome = match call {
                    "lookup" => client.lookup("9.9.9.9").await.map(drop),
                    "lookup_asn" => client.lookup_asn(9009).await.map(drop),
                    "lookup_domain" => client.lookup_domain("example.com").await.map(drop),
                    "my_ip" => client.my_ip().await.map(drop),
                    "my_entitlement" => client.my_entitlement().await.map(drop),
                    _ => {
                        let mut answers =
                            client.lookup_batch(["9.9.9.9"], BatchOptions::new()).await;
                        answers.swap_remove("9.9.9.9").expect("answered").map(drop)
                    }
                };
                let Err(err) = outcome else { panic!("{call}, {name}: returned an answer") };
                assert_eq!(err.kind(), ErrorKind::ServerError, "{call}, {name}: {err}");
                assert_eq!(err.status(), Some(200), "{call}, {name}: the status");
                // A batch answer whose maps read fails only the entry it cannot
                // read, which is not sent again.
                let sends = if call == "lookup_batch" && name == "entries without their members" {
                    1
                } else {
                    3
                };
                assert_eq!(stub.count(), sends, "{call}, {name}: retried like an outage");
            });
        }
    }
    while let Some(case) = cases.join_next().await {
        case.expect("case");
    }
}

/// A batch entry, or an entry's error, this version cannot read is that
/// address's error alone: the rest of the chunk is answered, and the chunk is
/// sent once.
#[tokio::test]
async fn a_batch_entry_it_cannot_read_fails_only_its_address() {
    let served = format!(r#""8.8.4.4":{}"#, answer("8.8.4.4"));
    let entries = [
        (
            "a member of the wrong type",
            format!(
                r#""results":{{"9.9.9.9":{{"ip":"9.9.9.9","is_bogon":"no"}},{served}}},"errors":{{}}"#
            ),
        ),
        (
            "an entry without its members",
            format!(r#""results":{{"9.9.9.9":{{}},{served}}},"errors":{{}}"#),
        ),
        ("a null entry", format!(r#""results":{{"9.9.9.9":null,{served}}},"errors":{{}}"#)),
        (
            "an entry that is no object",
            format!(r#""results":{{"9.9.9.9":5,{served}}},"errors":{{}}"#),
        ),
        (
            "an entry error that is no object",
            format!(r#""results":{{{served}}},"errors":{{"9.9.9.9":5}}"#),
        ),
        (
            "an entry error without its status",
            format!(r#""results":{{{served}}},"errors":{{"9.9.9.9":{{"error":"x"}}}}"#),
        ),
    ];
    for (name, entries) in entries {
        let stub = Stub::start([]).await;
        stub.sequence("/batch", [Route::ok(format!("{{{entries}}}"))]);
        let client = stub.client().retries(2).build().expect("build");

        let mut answers = client.lookup_batch(["9.9.9.9", "8.8.4.4"], BatchOptions::new()).await;

        let Err(err) = answers.swap_remove("9.9.9.9").expect("answered") else {
            panic!("{name}: read as an answer")
        };
        assert_eq!(err.kind(), ErrorKind::ServerError, "{name}: {err}");
        assert_eq!(err.status(), Some(200), "{name}: the status");
        let served = answers.swap_remove("8.8.4.4").expect("answered").expect(name);
        assert_eq!(served.ip, "8.8.4.4", "{name}: the other entry");
        assert_eq!(stub.count(), 1, "{name}: the chunk sent again");
        client.lookup("8.8.4.4").await.expect("the served entry, from the cache");
        assert_eq!(stub.count(), 1, "{name}: the served entry was not cached");
    }
}

/// Bearer only, on a lookup and on a batch's POST alike, and nothing at all
/// from a client built without a key.
#[tokio::test]
async fn a_lookup_and_a_batch_carry_the_key_as_a_bearer_token_or_nothing() {
    let stub = Stub::start(Stub::ok_routes(&["1.1.1.1", "8.8.8.8"])).await;
    for client in [stub.client(), stub.anonymous()] {
        let client = client.no_cache().build().expect("build");
        client.lookup("1.1.1.1").await.expect("lookup");
        let answers = client.lookup_batch(["1.1.1.1", "8.8.8.8"], BatchOptions::new()).await;
        assert!(answers.values().all(Result::is_ok), "{answers:?}");
    }

    let calls = stub.requests();
    assert_eq!(calls.len(), 4);
    let bearer = format!("Bearer {KEY}");
    for (i, call) in calls.iter().enumerate() {
        let want = if i < 2 { Some(bearer.as_str()) } else { None };
        assert_eq!(call.header("authorization"), want, "{} {}", call.method, call.path);
        assert_eq!(call.header("x-api-key"), None, "{}", call.path);
        let target = call.header("x-stub-target").unwrap_or_default();
        assert!(!target.contains("apikey"), "the key went into the query string: {target}");
    }
}

#[test]
fn the_builder_rejects_unusable_options() {
    assert!(Client::builder().concurrency(0).build().is_err());
    assert!(Client::builder().cache(0, Duration::from_secs(60)).build().is_err());
    assert!(Client::builder().cache(10, Duration::ZERO).build().is_err());
    assert!(Client::builder().timeout(Duration::ZERO).build().is_err());
    assert!(Client::builder().base_url("not a url").build().is_err());
    assert!(Client::builder().base_url("/relative").build().is_err());
}

#[tokio::test]
async fn an_ipv6_address_survives_the_path_template() {
    let stub = Stub::start([served("2606:4700:4700::1111")]).await;
    let client = stub.client().build().expect("build");

    let result = client.lookup("2606:4700:4700::1111").await.expect("lookup");

    assert_eq!(result.ip, "2606:4700:4700::1111");
}

/// An empty `error` beside an `rc` read as the empty message, so the error
/// said only its status.
#[tokio::test]
async fn an_empty_error_message_gives_way_to_the_rc() {
    let stub = Stub::start([(
        "/9.9.9.9".to_owned(),
        Route::json(429, r#"{"error":"","rc":"QUOTA_EXCEEDED"}"#),
    )])
    .await;
    let client = stub.client().build().expect("build");

    let err = client.lookup("9.9.9.9").await.expect_err("a spent quota");

    assert_eq!(err.kind(), ErrorKind::QuotaExceeded);
    assert!(err.to_string().contains("QUOTA_EXCEEDED"), "{err}");
}

/// The API answers an AS or a domain at `GET /{ip}` too, in a shape a lookup
/// cannot read, so `lookup` refuses anything but an address before sending it,
/// pointing at the calls that take one. A batch's addresses are the API's to
/// refuse, entry by entry.
#[tokio::test]
async fn a_lookup_refuses_what_is_not_an_ip_address_before_sending_it() {
    let stub = Stub::start([]).await;
    let client = stub.client().retries(2).build().expect("build");

    for input in ["AS15169", "15169", "example.com", "", " 1.1.1.1", "1.1.1.1/32", "fe80::1%eth0"] {
        let err = client.lookup(input).await.expect_err(input);
        assert_eq!(err.kind(), ErrorKind::BadRequest, "{input:?}: {err}");
        assert!(!err.retryable(), "{input:?}: retrying cannot make it an address");
        let want = format!(
            "'{input}' is not an IP address: look up an AS with lookup_asn, and a domain with \
             lookup_domain"
        );
        assert_eq!(err.message(), want);
    }
    assert_eq!(stub.count(), 0, "a refused lookup still sent a request");

    let answers = client.lookup_batch(["example.com", "10.0.0.1"], BatchOptions::new()).await;
    let refused = answers["example.com"].as_ref().expect_err("the API refuses it per entry");
    assert_eq!(refused.status(), Some(400), "{refused}");
    assert_eq!(stub.calls(), ["/batch"], "a batch's entries are sent as given");
}

/// Every call goes to the base URL the client was built with, never to the host
/// the spec names for an operation: a generator that honored the spec's
/// per-path `servers` sent lookups to production whatever the base URL said.
#[tokio::test]
async fn every_lookup_goes_to_the_clients_base_url() {
    let asn = r#"{"asn":9009,"name":"M247 Europe SRL","country":"RO","registry":"ripe","allocated":"2005-06-06","domain":"m247global.com"}"#;
    let domain = r#"{"domain":"example.com","is_registered":true,"has_a":true,"has_aaaa":true,"has_mx":true,"has_ns":true,"has_cname":false}"#;
    let stub = Stub::start([
        served("146.70.116.130"),
        served("8.8.8.8"),
        ("/asn/9009".to_owned(), Route::ok(asn)),
        ("/domain/example.com".to_owned(), Route::ok(domain)),
        ("/myip".to_owned(), Route::ok(answer("146.70.116.130"))),
    ])
    .await;
    let client = stub.client().no_cache().build().expect("build");

    client.lookup("146.70.116.130").await.expect("lookup");
    client.lookup_asn(9009).await.expect("lookup_asn");
    client.lookup_domain("example.com").await.expect("lookup_domain");
    client.my_ip().await.expect("my_ip");
    let answers = client.lookup_batch(["146.70.116.130", "8.8.8.8"], BatchOptions::new()).await;
    assert!(answers.values().all(Result::is_ok), "{answers:?}");

    assert_eq!(
        stub.calls(),
        ["/146.70.116.130", "/asn/9009", "/domain/example.com", "/myip", "/batch"]
    );
}

/// The AS number goes out as given, all 32 bits of it, and each call is its own
/// request: an AS answer has no address to cache it under.
#[tokio::test]
async fn an_as_lookup_sends_its_number_as_given_and_is_never_cached() {
    let body = r#"{"asn":9009,"name":"M247 Europe SRL","country":"RO","registry":"ripe","allocated":"2005-06-06","domain":"m247global.com"}"#;
    let stub = Stub::start([("/asn/9009".to_owned(), Route::ok(body))]).await;
    let client = stub.client().build().expect("build");

    let first = client.lookup_asn(9009).await.expect("lookup_asn");
    client.lookup_asn(9009).await.expect("lookup_asn");
    client.lookup_asn(u32::MAX).await.expect_err("an AS the stub does not know");

    assert_eq!(first.asn, 9009);
    assert_eq!(first.r#type, None, "a member the plan omits must be None");
    assert_eq!(stub.calls(), ["/asn/9009", "/asn/9009", "/asn/4294967295"]);
}

/// A domain is sent as given, escaped so that it stays one path segment, and is
/// never cached either.
#[tokio::test]
async fn a_domain_lookup_escapes_its_domain_and_is_never_cached() {
    let body = r#"{"domain":"example.com","is_registered":true,"has_a":true,"has_aaaa":true,"has_mx":true,"has_ns":true,"has_cname":false}"#;
    let stub = Stub::start([("/domain/example.com".to_owned(), Route::ok(body))]).await;
    let client = stub.client().build().expect("build");

    let first = client.lookup_domain("example.com").await.expect("lookup_domain");
    client.lookup_domain("example.com").await.expect("lookup_domain");
    client.lookup_domain("a/b?c").await.expect_err("a domain the stub does not know");

    assert!(first.is_registered);
    assert_eq!(first.is_disposable, None, "a member the plan omits must be None");
    assert_eq!(stub.count(), 3, "a domain answer was cached");
    assert_eq!(stub.target("/domain/a/b?c").as_deref(), Some("/domain/a%2Fb%3Fc"));
}

const ENTITLEMENT_BODY: &str = r#"{
  "org_id": "85bb51e4-2eb6-4a31-8e4d-02ba8b98fe61",
  "apikey": {
    "id": "0ab424cc-7619-4dad-b027-afacdc2cedb0",
    "expires": null,
    "allowed_cidrs": []
  },
  "plan": {"key": "max", "tier": "max"},
  "usage": {
    "requests": 580,
    "quota": 5000000,
    "hard_limit": null,
    "window_start": "2026-09-04T07:00:00Z",
    "window_end": "2026-10-04T07:00:00Z"
  }
}"#;

#[tokio::test]
async fn my_ip_looks_up_the_calling_address() {
    let stub = Stub::start([("/myip".to_owned(), Route::ok(answer("45.83.91.1")))]).await;
    let client = stub.client().build().expect("build");

    let answer = client.my_ip().await.expect("my_ip");

    assert_eq!(answer.ip, "45.83.91.1");
    assert!(!answer.is_bogon);
}

/// The cache is keyed by address, and which address this is IS the question.
#[tokio::test]
async fn my_ip_is_not_cached() {
    let stub = Stub::start([("/myip".to_owned(), Route::ok(answer("45.83.91.1")))]).await;
    let client = stub.client().build().expect("build");

    client.my_ip().await.expect("my_ip");
    client.my_ip().await.expect("my_ip");

    assert_eq!(stub.count(), 2);
}

#[tokio::test]
async fn my_entitlement_reports_the_plan_and_the_usage() {
    let stub = Stub::start([("/api/v1/entitlement".to_owned(), Route::ok(ENTITLEMENT_BODY))]).await;
    let client = stub.client().build().expect("build");

    let ent = client.my_entitlement().await.expect("my_entitlement");

    assert_eq!(ent.plan.key, "max");
    assert_eq!(ent.plan.tier, "max");
    assert_eq!(ent.usage.requests, 580);
    assert_eq!(ent.usage.quota, 5_000_000);
    // None means NEVER stop, which is not the same as a limit of zero.
    assert_eq!(ent.usage.hard_limit, None);
    assert!(ent.apikey.allowed_cidrs.is_empty());
    assert_eq!(ent.apikey.expires, None);
}

/// The whole point is what has been spent.
#[tokio::test]
async fn my_entitlement_is_not_cached() {
    let stub = Stub::start([("/api/v1/entitlement".to_owned(), Route::ok(ENTITLEMENT_BODY))]).await;
    let client = stub.client().build().expect("build");

    client.my_entitlement().await.expect("my_entitlement");
    client.my_entitlement().await.expect("my_entitlement");

    assert_eq!(stub.count(), 2);
}

#[tokio::test]
async fn my_entitlement_surfaces_a_missing_key() {
    let stub = Stub::start([(
        "/api/v1/entitlement".to_owned(),
        Route::json(401, r#"{"error":"an API key is required"}"#),
    )])
    .await;
    let client = stub.anonymous().retries(0).build().expect("build");

    let err = client.my_entitlement().await.expect_err("expected an error");
    assert_eq!(err.kind(), ErrorKind::Unauthorized);
    assert_eq!(err.message(), "an API key is required");
}

/// A middleware looks the visitor up on every request a page fires, all at once,
/// so concurrent misses for one address must share one request.
#[tokio::test]
async fn concurrent_misses_for_one_address_share_one_request() {
    let stub = Stub::start_with_delay([served("45.83.91.1")], Duration::from_millis(200)).await;
    let client = stub.client().build().expect("build");

    let answers =
        futures_util::future::join_all((0..20).map(|_| client.lookup("45.83.91.1"))).await;

    assert_eq!(stub.count(), 1, "20 concurrent callers sent {} requests", stub.count());
    for answer in answers {
        assert_eq!(answer.expect("every caller is answered").ip, "45.83.91.1");
    }
}

/// A failure is handed to every caller waiting on it and cached for none, so the
/// next call asks again.
#[tokio::test]
async fn a_shared_failure_reaches_every_waiter_and_is_not_cached() {
    let stub = Stub::start_with_delay([], Duration::from_millis(200)).await;
    stub.sequence(
        "/45.83.91.1",
        [Route::json(403, r#"{"error":"forbidden"}"#), Route::ok(answer("45.83.91.1"))],
    );
    let client = stub.client().build().expect("build");

    let answers = futures_util::future::join_all((0..5).map(|_| client.lookup("45.83.91.1"))).await;

    assert_eq!(stub.count(), 1);
    for answer in answers {
        assert_eq!(answer.expect_err("the one request failed").kind(), ErrorKind::Forbidden);
    }
    assert_eq!(client.lookup("45.83.91.1").await.expect("asked again").ip, "45.83.91.1");
    assert_eq!(stub.count(), 2, "the failure was cached");
}

/// A batch awaits the lookup already in flight for one of its addresses rather
/// than sending that address again.
#[tokio::test]
async fn a_batch_awaits_a_lookup_in_flight() {
    let stub = Stub::start_with_delay([served("45.83.91.1"), served("45.83.91.2")], STAGGER).await;
    let client = stub.client().build().expect("build");

    let (single, batch) = tokio::join!(client.lookup("45.83.91.1"), async {
        tokio::time::sleep(STAGGER / 4).await;
        client.lookup_batch(["45.83.91.1", "45.83.91.2"], BatchOptions::new()).await
    });

    single.expect("the lookup");
    assert!(batch.values().all(Result::is_ok), "{batch:?}");
    let calls = stub.requests();
    assert_eq!(calls.len(), 2, "{:?}", stub.calls());
    let posted = calls.iter().find(|call| call.path == "/batch").expect("the batch was sent");
    assert!(!posted.body.contains("45.83.91.1"), "the batch sent it again: {}", posted.body);
}

/// A lookup awaits the batch in flight that carries its address.
#[tokio::test]
async fn a_lookup_awaits_a_batch_in_flight() {
    let stub = Stub::start_with_delay([served("45.83.91.1"), served("45.83.91.2")], STAGGER).await;
    let client = stub.client().build().expect("build");

    let (batch, single) = tokio::join!(
        client.lookup_batch(["45.83.91.1", "45.83.91.2"], BatchOptions::new()),
        async {
            tokio::time::sleep(STAGGER / 4).await;
            client.lookup("45.83.91.2").await
        }
    );

    assert!(batch.values().all(Result::is_ok), "{batch:?}");
    assert_eq!(single.expect("the lookup").ip, "45.83.91.2");
    assert_eq!(stub.calls(), ["/batch"]);
}

/// A lookup that joined a batch takes the batch's answer for its address, a
/// failed one included, which the cache never holds for it to fall back on.
#[tokio::test]
async fn a_lookup_takes_the_failure_of_the_batch_it_joined() {
    let refused = ("/45.83.91.2".to_owned(), Route::json(403, r#"{"error":"forbidden"}"#));
    let stub = Stub::start_with_delay([served("45.83.91.1"), refused], STAGGER).await;
    let client = stub.client().build().expect("build");

    let (_, single) = tokio::join!(
        client.lookup_batch(["45.83.91.1", "45.83.91.2"], BatchOptions::new()),
        async {
            tokio::time::sleep(STAGGER / 4).await;
            client.lookup("45.83.91.2").await
        }
    );

    assert_eq!(single.expect_err("the batch's failure").kind(), ErrorKind::Forbidden);
    assert_eq!(stub.calls(), ["/batch"]);
}

/// Dropping the caller that leads a request must not fail the others waiting on
/// it: one of them asks again.
#[tokio::test]
async fn a_waiter_outlives_the_caller_it_joined() {
    let stub = Stub::start_with_delay([served("45.83.91.1")], STAGGER).await;
    let client = stub.client().build().expect("build");

    let leader = tokio::time::timeout(STAGGER / 2, client.lookup("45.83.91.1"));
    let waiter = async {
        tokio::time::sleep(STAGGER / 4).await;
        client.lookup("45.83.91.1").await
    };
    let (dropped, answer) = tokio::join!(leader, waiter);

    assert!(dropped.is_err(), "the leader should have been cut off");
    assert_eq!(answer.expect("the waiter asked again").ip, "45.83.91.1");
    assert_eq!(stub.count(), 2);
}

/// Without a cache every lookup is served, as `no_cache` promises.
#[tokio::test]
async fn a_client_without_a_cache_shares_nothing() {
    let stub = Stub::start_with_delay([served("45.83.91.1")], Duration::from_millis(100)).await;
    let client = stub.client().no_cache().build().expect("build");

    futures_util::future::join_all((0..3).map(|_| client.lookup("45.83.91.1"))).await;
    let batch = || client.lookup_batch(["45.83.91.1"], BatchOptions::new());
    tokio::join!(batch(), batch());

    assert_eq!(stub.calls(), ["/45.83.91.1", "/45.83.91.1", "/45.83.91.1", "/batch", "/batch"]);
}

/// No attempt can meet a zero timeout, so it is refused where it is set, before a
/// bogon or a cache hit can answer without a request to fail.
#[tokio::test]
async fn a_zero_per_call_timeout_is_refused_before_anything_answers() {
    let stub = Stub::start([served("45.83.91.1")]).await;
    let client = stub.client().build().expect("build");
    client.lookup("45.83.91.1").await.expect("warm the cache");
    let zero = || LookupOptions::new().timeout(Duration::ZERO);

    for (call, answer) in [
        ("a served address", client.lookup_with("45.83.91.2", zero()).await.map(drop)),
        ("a bogon", client.lookup_with("10.0.0.1", zero()).await.map(drop)),
        ("a cache hit", client.lookup_with("45.83.91.1", zero()).await.map(drop)),
        ("lookup_asn", client.lookup_asn_with(9009, zero()).await.map(drop)),
        ("lookup_domain", client.lookup_domain_with("example.com", zero()).await.map(drop)),
        ("my_ip", client.my_ip_with(zero()).await.map(drop)),
        ("my_entitlement", client.my_entitlement_with(zero()).await.map(drop)),
    ] {
        let err = answer.expect_err(call);
        assert_eq!(err.kind(), ErrorKind::BadRequest, "{call}: {err}");
    }
    let batch = client
        .lookup_batch(["10.0.0.1", "45.83.91.1"], BatchOptions::new().timeout(Duration::ZERO))
        .await;
    for (ip, answer) in batch {
        assert_eq!(answer.expect_err(&ip).kind(), ErrorKind::BadRequest, "{ip}");
    }
    assert_eq!(stub.count(), 1, "a refused call still sent a request");
}

#[test]
fn the_default_base_url_is_the_api_host() {
    assert_eq!(internetdata::DEFAULT_BASE_URL, "https://api.internetdata.io");
}

/// The authorization server is the apex: `api.` is dropped and `api-<env>.`
/// read as `<env>.`, and any other base URL is used as given.
#[test]
fn the_oauth_requests_go_to_the_apex_of_the_base_url() {
    for (base, apex) in [
        ("https://api.internetdata.io", "https://internetdata.io"),
        ("https://api-staging.internetdata.io", "https://staging.internetdata.io"),
        ("https://API-Staging.internetdata.io//", "https://staging.internetdata.io"),
        ("http://api.example.test:8443/prefix", "http://example.test:8443/prefix"),
        ("https://internetdata.io", "https://internetdata.io"),
        ("https://apis.internetdata.io", "https://apis.internetdata.io"),
        ("https://api-.internetdata.io", "https://api-.internetdata.io"),
        ("http://127.0.0.1:8080", "http://127.0.0.1:8080"),
        ("http://[::1]:8080", "http://[::1]:8080"),
    ] {
        let client = Client::builder().base_url(base).build().expect(base);
        let url = client.oauth().authorization_url("cid", "http://127.0.0.1/cb", "c").expect(base);
        assert!(url.starts_with(&format!("{apex}/oauth/authorize?")), "{base}: {url}");
    }
    let client = Client::new().expect("build");
    let url = client.oauth().authorization_url("cid", "http://127.0.0.1/cb", "c").expect("url");
    assert!(url.starts_with("https://internetdata.io/oauth/authorize?"), "{url}");
}

/// What leaves the client, by the host each request names: a lookup and a
/// database call go to the API host, and an OAuth request to its apex.
#[tokio::test]
async fn an_oauth_request_names_the_apex_and_a_lookup_the_api_host() {
    let stub = Stub::start([
        served("146.70.116.130"),
        ("/api/v2/database/list".to_owned(), Route::ok(r#"{"databases":[]}"#)),
        ("/oauth/revoke".to_owned(), Route::ok("")),
    ])
    .await;
    let port = stub.base_url.rsplit(':').next().expect("a port").to_owned();
    let local = std::net::SocketAddr::from(([127, 0, 0, 1], 0));
    let http = reqwest::Client::builder()
        .resolve("api.example.test", local)
        .resolve("example.test", local)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("http client");
    let client = Client::builder()
        .base_url(format!("http://api.example.test:{port}"))
        .http_client(http)
        .build()
        .expect("build");

    client.lookup("146.70.116.130").await.expect("lookup");
    client.database().list().await.expect("list");
    client.oauth().revoke("cid", "token").await.expect("revoke");

    let hosts: Vec<(String, String)> = stub
        .requests()
        .into_iter()
        .map(|call| (call.path.clone(), call.header("host").unwrap_or_default().to_owned()))
        .collect();
    let api = format!("api.example.test:{port}");
    let apex = format!("example.test:{port}");
    assert_eq!(
        hosts,
        [
            ("/146.70.116.130".to_owned(), api.clone()),
            ("/api/v2/database/list".to_owned(), api),
            ("/oauth/revoke".to_owned(), apex),
        ]
    );
}

/// Awaits a call that must fail, and how long it took to.
async fn stalled<T: std::fmt::Debug>(
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
