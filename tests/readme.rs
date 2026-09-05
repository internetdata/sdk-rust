// Every code block in README.md, compiled but never run.
//
// The README is the API contract customers actually read, so a rename or a
// signature change that invalidates it should fail the build rather than reach
// a reader. Mirror any README edit here.
#![allow(unused, path_statements, clippy::no_effect)]

use internetdata::{Client, ErrorKind, Format};

async fn snippets() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::builder().api_key(std::env::var("INTERNETDATA_API_KEY")?).build()?;

    for database in client.list().await? {
        println!("{} ({}): {}", database.name, database.standing, database.summary);
    }

    let key = std::env::var("INTERNETDATA_API_KEY")?;
    let client = Client::builder().api_key(key).retries(4).build()?;

    for database in client.list().await? {
        println!("{} is {}", database.base, database.standing);

        for version in &database.versions {
            println!("  {} built in {:?}", version.id, version.formats);
        }
    }

    let meta = client.metadata("bogon_ip_v1").await?;

    println!("{} rows, built {}", meta.entries, meta.updated);
    println!("{:?} bytes", meta.size.get("csvgz"));

    for column in &meta.schema["csvgz"] {
        println!("{}: {}", column.name, column.r#type);
    }

    let written = client.download("bogon_ip_v1", Format::Csvgz, "./bogon_ip.csv.gz").await?;

    let raw = client.download_bytes("bogon_asn_v1", Format::Csvgz).await?;

    let url = client.download_url("bogon_ip_v1", Format::Csvgz).await?;

    let sums = client.checksums("bogon_ip_v1", Format::Csvgz).await?;
    println!("{}", sums.sha256);

    for attempt in client.downloads(Some(20)).await? {
        println!(
            "{} {} {:?} {}",
            attempt.created,
            attempt.dataset_id,
            attempt.outcome,
            attempt.http_status.unwrap_or(0)
        );
    }

    match client.metadata("bogon_ip_v1").await {
        Ok(meta) => println!("{}", meta.entries),
        Err(err) => println!("{} {}", err.kind(), err.retryable()),
    }
    let _ = [
        ErrorKind::BadRequest,
        ErrorKind::Unauthorized,
        ErrorKind::Forbidden,
        ErrorKind::RateLimited,
        ErrorKind::QuotaExceeded,
        ErrorKind::ServerError,
        ErrorKind::Network,
        ErrorKind::Io,
    ];
    Ok(())
}

fn blocking_snippet() -> Result<(), Box<dyn std::error::Error>> {
    let key = std::env::var("INTERNETDATA_API_KEY")?;
    let runtime = tokio::runtime::Runtime::new()?;
    let client = Client::builder().api_key(key).build()?;
    let databases = runtime.block_on(client.list())?;
    Ok(())
}
