// Every code block in README.md, compiled but never run.
//
// The README is the API contract customers actually read, so a rename or a
// signature change that invalidates it should fail the build rather than reach
// a reader. Mirror any README edit here.
#![allow(unused, path_statements, clippy::no_effect)]

use std::time::Duration;

use internetdata::{Client, DatabaseFormat, DeviceAuthorizationOptions, ErrorKind, OauthError};

async fn snippets() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::builder().api_key(std::env::var("INTERNETDATA_API_KEY")?).build()?;

    for database in client.database().list().await? {
        println!("{} ({}): {}", database.name, database.standing, database.summary);
    }

    let key = std::env::var("INTERNETDATA_API_KEY")?;
    let client = Client::builder().api_key(key.clone()).retries(4).build()?;
    let client = Client::builder().api_key(key).timeout(Duration::from_secs(5)).build()?;

    for database in client.database().list().await? {
        println!("{} is {}", database.base, database.standing);

        for version in &database.versions {
            println!("  {} built in {:?}", version.id, version.formats);
        }
    }

    let meta = client.database().metadata("bogon_ip_v1").await?;

    println!("{} rows, built {}", meta.entries, meta.updated);
    println!("{:?} bytes", meta.size.get("csvgz"));

    for column in &meta.schema["csvgz"] {
        println!("{}: {}", column.name, column.r#type);
    }

    let written = client
        .database()
        .download("bogon_ip_v1", DatabaseFormat::Csvgz, "./bogon_ip.csv.gz")
        .await?;

    let raw = client.database().download_bytes("bogon_asn_v1", DatabaseFormat::Csvgz).await?;

    let url = client.database().download_url("bogon_ip_v1", DatabaseFormat::Csvgz).await?;

    let sums = client.database().checksums("bogon_ip_v1", DatabaseFormat::Csvgz).await?;
    println!("{}", sums.sha256);

    for attempt in client.database().downloads(Some(20)).await? {
        println!(
            "{} {} {:?} {}",
            attempt.created,
            attempt.dataset_id,
            attempt.outcome,
            attempt.http_status.unwrap_or(0)
        );
    }

    match client.database().metadata("bogon_ip_v1").await {
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

async fn oauth_snippet() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new()?;
    let scope = DeviceAuthorizationOptions::new().scope("account.read apikeys.read apikeys.reveal");
    let device = client.oauth().device_authorization_with("your-client-id", scope).await?;
    println!("Open {} and enter {}", device.verification_uri, device.user_code);

    let token = client.oauth().poll_device_token("your-client-id", &device).await?;
    let refresh_token = token.refresh_token.clone().unwrap_or_default();
    let Some(apikey) = token.apikey else {
        return Err("no API key came back: none was picked, or it cannot be shown again".into());
    };
    let keyed = Client::builder().api_key(apikey).build()?;

    match client.oauth().revoke("your-client-id", &refresh_token).await {
        Err(OauthError::AccessDenied(_)) | Err(OauthError::ExpiredToken(_)) => {}
        _ => {}
    }
    Ok(())
}

fn blocking_snippet() -> Result<(), Box<dyn std::error::Error>> {
    let key = std::env::var("INTERNETDATA_API_KEY")?;
    let runtime = tokio::runtime::Runtime::new()?;
    let client = Client::builder().api_key(key).build()?;
    let databases = runtime.block_on(client.database().list())?;
    Ok(())
}
