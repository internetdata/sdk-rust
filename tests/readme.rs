// Every code block in README.md, compiled but never run.
//
// The README is the API contract customers actually read, so a rename or a
// signature change that invalidates it should fail the build rather than reach
// a reader. Mirror any README edit here.
#![allow(unused, path_statements, clippy::no_effect)]

use std::time::Duration;

use internetdata::{
    AuthorizationUrlOptions, BatchOptions, Client, DatabaseFormat, DeviceAuthorizationOptions,
    DownloadOptions, ErrorKind, OauthError, RegisterOptions,
};

async fn lookup_snippets() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new()?;

    let result = client.lookup("146.70.116.130").await?;
    println!("{}", result.location.country);
    println!("{}", result.asn.name);
    println!("{}", result.is_tor);

    let client = Client::builder().api_key(std::env::var("INTERNETDATA_API_KEY")?).build()?;

    let result = client.lookup("146.70.116.130").await?;
    println!("{:?}", result.is_vpn);
    println!("{:?}", result.location.city);

    if let Some(vpn) = result.vpn.as_deref() {
        println!("{:?}", vpn.provider);
    }

    let key = std::env::var("INTERNETDATA_API_KEY")?;
    let client = Client::builder().api_key(key.clone()).concurrency(32).retries(4).build()?;
    let client = Client::builder().api_key(key).timeout(Duration::from_secs(5)).build()?;

    let result = client.my_ip().await?;
    println!("{}", result.ip);

    let acct = client.my_entitlement().await?;
    println!("{}", acct.plan.key);
    println!("{}", acct.usage.requests);
    println!("{}", acct.usage.window_end);

    let results =
        client.lookup_batch(["146.70.116.130", "8.8.8.8", "1.1.1.1"], BatchOptions::new()).await;

    for (ip, result) in &results {
        match result {
            Ok(result) => println!("{ip}: {}", result.location.country),
            Err(err) => eprintln!("{ip}: {err}"),
        }
    }

    let many_ips = vec!["146.70.116.130".to_owned()];
    let results = client
        .lookup_batch(
            many_ips,
            BatchOptions::new().concurrency(4).retries(4).timeout(Duration::from_secs(10)),
        )
        .await;

    let asn = client.lookup_asn(9009).await?;
    println!("{} {}", asn.name, asn.registry);

    let domain = client.lookup_domain("example.com").await?;
    println!("{}", domain.is_registered);

    let client = Client::new()?;

    let result = client.lookup("146.70.116.130").await?;
    println!("{}", result.location.country);

    let result2 = client.lookup("146.70.116.130").await?;
    println!("{}", result2.location.country);

    let client = Client::builder().cache(50_000, Duration::from_secs(6 * 60 * 60)).build()?;
    let client_no_cache = Client::builder().no_cache().build()?;

    let result = client.lookup("192.168.1.1").await?;
    result.is_bogon;
    result.is_tor;

    client.is_bogon("10.0.0.1");
    client.is_bogon("8.8.8.8");

    internetdata::is_bogon("10.0.0.1");

    match client.lookup("146.70.116.130").await {
        Ok(result) => println!("{}", result.location.country),
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

    result.is_hosting.unwrap_or(false);
    result.is_hosting.is_none();
    result.trial_expired == Some(true);
    Ok(())
}

async fn snippets() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::builder().api_key(std::env::var("INTERNETDATA_API_KEY")?).build()?;

    for database in client.database().list().await? {
        println!("{} ({}): {}", database.name, database.standing, database.summary);
    }

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

    let build_from_the_webhook = String::from("20261105.0t53s5");
    let options = DownloadOptions::new().build(build_from_the_webhook);
    let written = client
        .database()
        .download_with("vpn_ip_v1", DatabaseFormat::Mmdb, "./vpn_ip_v1.mmdb", &options)
        .await?;

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

async fn authorization_code_snippet(code: String) -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new()?;
    let redirect_uri = "http://127.0.0.1:8765/callback";
    let pkce = client.oauth().create_pkce()?;

    let opts = AuthorizationUrlOptions::new().scope("apikeys.use").state("your-state");
    let url = client.oauth().authorization_url_with(
        "your-client-id",
        redirect_uri,
        &pkce.challenge,
        opts,
    )?;
    // Open url in the browser. Its redirect to redirect_uri carries code and state.
    let token = client
        .oauth()
        .exchange_authorization_code("your-client-id", &code, &pkce.verifier, redirect_uri)
        .await?;
    Ok(())
}

async fn register_snippet() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new()?;
    let opts = RegisterOptions::new().client_name("Your App");
    let registration =
        client.oauth().register_with(&["http://127.0.0.1:8765/callback"], opts).await?;
    Ok(())
}

fn blocking_snippet() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Runtime::new()?;
    let client = Client::new()?;
    let result = runtime.block_on(client.lookup("146.70.116.130"))?;
    Ok(())
}
