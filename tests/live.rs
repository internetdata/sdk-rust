// A smoke test against a real deployment, ignored by default so `cargo test`
// stays offline and costs no quota. There is no anonymous tier here, so this one
// needs a key to say anything at all.
//
//     INTERNETDATA_API_KEY=... cargo test --test live -- --ignored --nocapture
//
// INTERNETDATA_BASE_URL points it somewhere other than production.

use internetdata::{Client, Standing};

/// The transfer is budgeted before it starts: `metadata` publishes a size per
/// format, so a mistaken id fails this ceiling rather than pulling gigabytes.
const CEILING: i64 = 8 << 20;

#[tokio::test]
#[ignore = "queries a real deployment"]
async fn live_catalog_and_download() {
    let key = std::env::var("INTERNETDATA_API_KEY")
        .ok()
        .filter(|key| !key.is_empty())
        .expect("INTERNETDATA_API_KEY must be set: every endpoint here needs a key");

    let mut builder = Client::builder().api_key(key);
    if let Ok(base) = std::env::var("INTERNETDATA_BASE_URL") {
        if !base.is_empty() {
            builder = builder.base_url(base);
        }
    }
    let client = builder.build().expect("build");

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
