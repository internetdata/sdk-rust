// A smoke test against a real deployment, ignored by default so `cargo test`
// stays offline and costs no quota. Set INTERNETDATA_API_KEY to exercise a key's
// fields and the databases it licenses; without one the lookups run without a
// key and the database half skips.
//
//     cargo test --test live -- --ignored --nocapture
//
// INTERNETDATA_BASE_URL points it somewhere other than production.

use internetdata::{Client, Standing};

/// The transfer is budgeted before it starts: `metadata` publishes a size per
/// format, so a mistaken ID fails this ceiling rather than pulling gigabytes.
const CEILING: i64 = 8 << 20;

#[tokio::test]
#[ignore = "queries a real deployment"]
async fn live_lookup() {
    let (client, key) = client();

    let answer = client.lookup("146.70.116.130").await.expect("lookup 146.70.116.130");
    println!(
        "146.70.116.130: country={} asn={} is_vpn={:?} is_hosting={:?}",
        answer.location.country, answer.asn.asn, answer.is_vpn, answer.is_hosting
    );
    assert!(!answer.is_bogon, "a served answer is not a local one");
    assert_eq!(answer.location.country, "AT", "146.70.116.130 should be in Austria");

    let asn = client.lookup_asn(9009).await.expect("lookup_asn 9009");
    println!("AS9009: {} {} {:?}", asn.name, asn.registry, asn.num_ips);
    assert_eq!(asn.asn, 9009);

    let domain = client.lookup_domain("example.com").await.expect("lookup_domain example.com");
    println!(
        "example.com: registered={} disposable={:?}",
        domain.is_registered, domain.is_disposable
    );
    assert_eq!(domain.domain, "example.com");

    if key.is_none() {
        // The one assertion a stub cannot make honestly: an answer without a key
        // does not include is_hosting, so it is ABSENT rather than false.
        assert_eq!(answer.is_hosting, None, "an answer without a key does not include is_hosting");
    }
}

#[tokio::test]
#[ignore = "queries a real deployment"]
async fn live_catalog_and_download() {
    let (client, key) = client();
    if key.is_none() {
        println!("SKIPPED: INTERNETDATA_API_KEY is not set, and every database here needs a key");
        return;
    }

    let databases = client.database().list().await.expect("list");
    println!("{} families visible to this key", databases.len());
    for database in &databases {
        println!("  {} {} {:?}", database.base, database.standing, database.license_type);
    }

    // The first LICENSED version this key holds, so the run works for any key
    // rather than naming a database only one organization has.
    let (id, format) = databases
        .iter()
        .filter(|database| database.standing == Standing::Licensed)
        .flat_map(|database| &database.versions)
        .find_map(|version| version.formats.first().map(|format| (version.id.clone(), *format)))
        .expect("this key licenses nothing, so there is nothing to download");

    let meta = client.database().metadata(&id).await.expect("metadata");
    let size = *meta.size.get(format.as_str()).expect("no size is published for this format");
    println!("{id}.{format}: {size} bytes, built {}", meta.updated);
    assert!(size > 0 && size <= CEILING, "{id} is {size} bytes, past the {CEILING} ceiling");

    let bytes = client.database().download_bytes(&id, format).await.expect("download_bytes");
    assert_eq!(bytes.len() as i64, size, "the transfer and the published size disagree");

    let sums = client.database().checksums(&id, format).await.expect("checksums");
    assert_eq!(sums.sha256.len(), 64, "sha256 {:?} did not unwrap past its key", sums.sha256);
}

/// A client with this run's key, if it has one, and the key.
fn client() -> (Client, Option<String>) {
    let key = std::env::var("INTERNETDATA_API_KEY").ok().filter(|key| !key.is_empty());
    let mut builder = Client::builder();
    if let Some(key) = &key {
        builder = builder.api_key(key);
    }
    if let Ok(base) = std::env::var("INTERNETDATA_BASE_URL") {
        if !base.is_empty() {
            builder = builder.base_url(base);
        }
    }
    (builder.build().expect("build"), key)
}
