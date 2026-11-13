// The published crate looking addresses, AS numbers and domains up against the
// real API.
//
// Without a key on production and on staging, where a lookup is Unlimited's
// answer, and with the staging key on staging. Nothing here pins a field COUNT:
// what holds on every plan is that the answer the client holds is the answer
// the wire carried, member for member, so a served false survives and a member
// the plan leaves out stays absent. The key's plan is read from its entitlement
// rather than assumed, so a change of plan stays a change of plan.

use std::collections::BTreeSet;

use internetdata::{BatchOptions, ErrorKind};
use internetdata_integration::{
    PROBE, PROBE_ASN, PROBE_DOMAIN, PROD, STAGING, assert_same_answer, client_for,
    keyless_client_for, skip_reason, skip_unless,
};
use serde_json::Value;

#[tokio::test]
async fn a_lookup_without_a_key_is_answered_on_production_and_staging() {
    for origin in [PROD, STAGING] {
        let (client, recorder) = keyless_client_for(origin).await;

        let result = client.lookup(PROBE).await.unwrap_or_else(|e| panic!("{origin}: {e}"));

        let facts = recorder.facts();
        assert!(!facts.is_empty(), "{origin}: nothing reached the wire");
        assert!(
            facts.iter().all(|fact| !fact.carried_key),
            "{origin}: a keyless client sent a key"
        );
        assert_eq!(result.ip, PROBE, "{origin}: answered about the wrong address");
        assert!(!result.is_bogon, "{origin}: a served answer is not a local one");
        let wire = recorder.json_body(&format!("/{PROBE}")).expect("the answer was captured");
        let held = serde_json::to_value(&result.answer).expect("re-encoding");
        assert_same_answer(origin, &wire, &held);
        // The one assertion a stub cannot make honestly: Unlimited leaves the
        // paid flags out, so they are ABSENT rather than false.
        assert_eq!(result.is_vpn, None, "{origin}: Unlimited's answer carries no is_vpn");
        println!("{origin}: {PROBE} is in {}", result.location.country);
    }
}

#[tokio::test]
async fn an_as_and_a_domain_without_a_key_are_answered_on_production_and_staging() {
    for origin in [PROD, STAGING] {
        let (client, recorder) = keyless_client_for(origin).await;

        let asn = client.lookup_asn(PROBE_ASN).await.unwrap_or_else(|e| panic!("{origin}: {e}"));
        let domain =
            client.lookup_domain(PROBE_DOMAIN).await.unwrap_or_else(|e| panic!("{origin}: {e}"));

        assert!(recorder.facts().iter().all(|fact| !fact.carried_key), "{origin}: a key was sent");
        assert_eq!(asn.asn, i32::try_from(PROBE_ASN).expect("a small AS"), "{origin}");
        assert_eq!(domain.domain, PROBE_DOMAIN, "{origin}");
        let wire = recorder.json_body(&format!("/asn/{PROBE_ASN}")).expect("the AS was captured");
        assert_same_answer(origin, &wire, &serde_json::to_value(&asn).expect("re-encoding"));
        let wire = recorder.json_body(&format!("/domain/{PROBE_DOMAIN}")).expect("captured");
        assert_same_answer(origin, &wire, &serde_json::to_value(&domain).expect("re-encoding"));
    }
}

#[tokio::test]
async fn a_bogon_is_answered_without_touching_the_network() {
    let (client, recorder) = keyless_client_for(STAGING).await;

    let result = client.lookup("10.0.0.1").await.expect("a private address");

    assert!(result.is_bogon, "a private address must be answered locally");
    assert!(internetdata::is_bogon("10.0.0.1"), "the standalone function must agree");
    assert!(recorder.facts().is_empty(), "the bogon path reached the network");
    // Computed rather than served, so it carries every flag whatever the plan.
    let wire = serde_json::to_value(&result.answer).expect("re-encoding");
    let flags =
        wire.as_object().expect("an object").iter().filter(|(name, _)| name.starts_with("is_"));
    for (name, value) in flags {
        let want = Value::Bool(name == "is_bogon");
        assert_eq!(value, &want, "{name} on a bogon");
    }
}

#[tokio::test]
async fn a_batch_without_a_key_is_refused_once_and_its_bogon_still_answered() {
    let (client, recorder) = keyless_client_for(STAGING).await;

    // Two addresses on the wire: a keyless batch of one is answered as its lookup.
    let answers = client.lookup_batch([PROBE, "8.8.8.8", "10.0.0.1"], BatchOptions::new()).await;

    let bogon = answers["10.0.0.1"].as_ref().expect("10.0.0.1 was not answered locally");
    assert!(bogon.is_bogon);
    for ip in [PROBE, "8.8.8.8"] {
        let refused = answers[ip].as_ref().expect_err("a batch without a key must fail");
        assert_eq!(refused.kind(), ErrorKind::Unauthorized, "{ip}");
        assert_eq!(refused.status(), Some(401), "{ip}");
        assert!(!refused.retryable(), "{ip}");
    }
    let facts = recorder.facts();
    let routes: Vec<(&str, &str)> =
        facts.iter().map(|fact| (fact.method.as_str(), fact.path.as_str())).collect();
    assert_eq!(routes, [("POST", "/batch")], "one request, never retried");
    let sent: BTreeSet<&str> = facts[0].ips.iter().map(String::as_str).collect();
    assert_eq!(sent, BTreeSet::from([PROBE, "8.8.8.8"]), "the batch sent the wrong addresses");
}

/// A free-plan key's answer is a paid plan's while the day's trial lasts and
/// Unlimited's after, and says which; any other plan's carries no
/// trial_expired at all.
#[tokio::test]
async fn a_keys_lookup_reaches_the_wire_and_answers_as_its_plan() {
    skip_unless!(skip_reason());
    let (client, recorder) = client_for().await;

    let plan = client.my_entitlement().await.expect("my_entitlement").plan;
    let result = client.lookup(PROBE).await.expect("lookup");

    // Checked before anything is compared: an unsent key answers the keyless
    // shape, which every comparison below would accept.
    assert!(recorder.carried_key(), "the key never reached the wire");
    let wire = recorder.json_body(&format!("/{PROBE}")).expect("the answer was captured");
    let held = serde_json::to_value(&result.answer).expect("re-encoding");
    assert_same_answer("with the key", &wire, &held);
    println!(
        "the key is on {} ({}), trial_expired {:?}",
        plan.key, plan.tier, result.trial_expired
    );
    match (plan.key.as_str(), result.trial_expired) {
        ("free", Some(expired)) => {
            assert_eq!(result.is_vpn.is_some(), !expired, "is_vpn is served while the trial lasts");
        }
        ("free", None) => panic!("a free-plan key's answer carries trial_expired"),
        (_, expired) => {
            assert_eq!(expired, None, "only a free-plan key's answer carries trial_expired");
            assert!(result.is_vpn.is_some(), "a paid plan answers is_vpn");
        }
    }
}

#[tokio::test]
async fn a_keys_as_and_domain_lookups_keep_their_shape() {
    skip_unless!(skip_reason());
    let (client, recorder) = client_for().await;

    let asn = client.lookup_asn(PROBE_ASN).await.expect("lookup_asn");
    let domain = client.lookup_domain(PROBE_DOMAIN).await.expect("lookup_domain");

    assert!(recorder.carried_key(), "the key never reached the wire");
    let wire = recorder.json_body(&format!("/asn/{PROBE_ASN}")).expect("the AS was captured");
    let held = serde_json::to_value(&asn).expect("re-encoding");
    assert_same_answer("the AS, with the key", &wire, &held);
    let wire = recorder.json_body(&format!("/domain/{PROBE_DOMAIN}")).expect("captured");
    let held = serde_json::to_value(&domain).expect("re-encoding");
    assert_same_answer("the domain, with the key", &wire, &held);
}

#[tokio::test]
async fn a_keys_entitlement_names_its_plan_and_window() {
    skip_unless!(skip_reason());
    let (client, recorder) = client_for().await;

    let entitlement = client.my_entitlement().await.expect("my_entitlement");

    assert!(recorder.carried_key(), "the key never reached the wire");
    assert!(!entitlement.plan.key.is_empty(), "no plan");
    let tiers = ["free", "starter", "scale", "max"];
    assert!(tiers.contains(&entitlement.plan.tier.as_str()), "tier {}", entitlement.plan.tier);
    assert!(entitlement.usage.window_start < entitlement.usage.window_end, "an empty window");
}

/// A free-plan key's batch of two is refused once as a bad request (402), and a
/// paid plan's is answered; either way duplicates collapse and the bogon never
/// leaves the client.
#[tokio::test]
async fn a_keys_batch_follows_its_plan() {
    skip_unless!(skip_reason());
    let (client, recorder) = client_for().await;
    let plan = client.my_entitlement().await.expect("my_entitlement").plan;

    let answers =
        client.lookup_batch([PROBE, "8.8.8.8", PROBE, "10.0.0.1"], BatchOptions::new()).await;

    let keys: Vec<&str> = answers.keys().map(String::as_str).collect();
    assert_eq!(keys, [PROBE, "8.8.8.8", "10.0.0.1"], "duplicates collapse, first-seen order holds");
    assert!(answers["10.0.0.1"].as_ref().expect("answered locally").is_bogon);
    let batches: Vec<_> = recorder.facts().into_iter().filter(|f| f.path == "/batch").collect();
    let sent: BTreeSet<&str> =
        batches.iter().flat_map(|fact| fact.ips.iter().map(String::as_str)).collect();
    assert_eq!(sent, BTreeSet::from([PROBE, "8.8.8.8"]), "the batch sent the wrong addresses");
    for ip in [PROBE, "8.8.8.8"] {
        match (plan.key.as_str(), &answers[ip]) {
            ("free", Err(refused)) => {
                assert_eq!(refused.kind(), ErrorKind::BadRequest, "{ip}");
                assert_eq!(refused.status(), Some(402), "{ip}");
                assert_eq!(batches.len(), 1, "a refused batch was retried");
            }
            ("free", Ok(_)) => panic!("{ip}: a free-plan key's batch of two was answered"),
            (_, answer) => assert!(answer.is_ok(), "{ip} failed: {:?}", answer.as_ref().err()),
        }
    }
}
