# [<img src="https://s3.internetdata.io/internetdata-public/brand/mark.svg" alt="InternetData" height="28"/>](https://internetdata.io/) InternetData Rust Client Library

[![crates.io](https://img.shields.io/crates/v/internetdata.svg)](https://crates.io/crates/internetdata)
[![docs.rs](https://img.shields.io/docsrs/internetdata)](https://docs.rs/internetdata)
[![license](https://img.shields.io/crates/l/internetdata.svg)](LICENSE)

The official Rust client library for the [InternetData](https://internetdata.io) API.

It tells you what every InternetData database says about an IP address, an AS or a domain, and downloads the licensed databases themselves.

## Getting Started

```bash
cargo add internetdata
```

Requires Rust 1.85 or newer. Everything is `async` and runs on tokio.

## Usage

**No API key needed to start.** Unlimited, the free plan, answers the Tor and relay flags, InternetData's own flags such as `is_anycast` and `is_web`, `location` to the country, `asn`, `company`, `hostname` and the abuse email, limited per address per minute, with a key or without one.

```rust
use internetdata::Client;

let client = Client::new()?;

let result = client.lookup("146.70.116.130").await?;
println!("{}", result.location.country);    // AT
println!("{}", result.asn.name);            // M247 Europe SRL
println!("{}", result.is_tor);              // false
```

### With an API key

Every paid plan answers every field: `is_vpn`, `is_hosting`, `is_cdn` and the four proxy flags as well, the detail behind each flag, the city-level location, `whois` and `rwhois`. A free account tries every field, 100 lookups a day. Create a key in the console, then pass it in:

```rust
let client = Client::builder()
    .api_key(std::env::var("INTERNETDATA_API_KEY")?)
    .build()?;

let result = client.lookup("146.70.116.130").await?;
println!("{:?}", result.is_vpn);            // Some(true), on a paid plan
println!("{:?}", result.location.city);     // Some("Vienna"), on a paid plan

if let Some(vpn) = result.vpn.as_deref() {
    println!("{:?}", vpn.provider);         // Some("mullvad"), on a paid plan
}
```

Every setting has a default, and `Client::builder()` is where you change one:

```rust
let client = Client::builder().api_key(key).concurrency(32).retries(4).build()?;
```

Each attempt at a request gives up after 30 seconds, and `timeout` changes that. A database download isn't bound by it, since a large one runs for minutes:

```rust
use std::time::Duration;

let client = Client::builder().api_key(key).timeout(Duration::from_secs(5)).build()?;
```

### Your own address

```rust
let result = client.my_ip().await?;
println!("{}", result.ip);   // the address we saw this call come from
```

Same answer `lookup` would give for that address, and the same cost against your allowance. It is deliberately not cached: which address you are is the whole question, and a machine that moves between networks would otherwise be told where it used to be.

### Your plan and usage

```rust
let acct = client.my_entitlement().await?;
println!("{}", acct.plan.key);            // free
println!("{}", acct.usage.requests);      // 42
println!("{}", acct.usage.window_end);    // when the allowance resets
```

Usage counts against the anniversary of your subscription on a paid plan, not the calendar month and not the billing period, and against the UTC day on the free plan; it is the same number a lookup is gated on. `hard_limit` is `None` on an uncapped plan, which is not the same as zero.

`my_ip_with` and `my_entitlement_with` take this call's own retry budget and timeout, the same as `lookup_with`.

### Batch lookup

Look up many addresses at once. Bogons and cached answers are handled locally, and everything else goes to the batch endpoint in chunks of up to 1000 addresses, in parallel. The batch endpoint needs a paid plan:

```rust
use internetdata::BatchOptions;

let results = client
    .lookup_batch(["146.70.116.130", "8.8.8.8", "1.1.1.1"], BatchOptions::new())
    .await;

for (ip, result) in &results {
    match result {
        Ok(result) => println!("{ip}: {}", result.location.country),
        Err(err) => eprintln!("{ip}: {err}"),
    }
}
```

Results are keyed by address and in the order you first listed each one, so duplicates in your list collapse into a single entry and one address failing never loses the rest: it carries its error as its value, with the status the API would have given that address on its own.

There's no limit on how many addresses you pass. How many chunks are in flight at once, how many times a failed chunk is retried, and how long each attempt may take are configurable per call:

```rust
let results = client
    .lookup_batch(
        many_ips,
        BatchOptions::new().concurrency(4).retries(4).timeout(Duration::from_secs(10)),
    )
    .await;
```

### AS and domain lookups

```rust
let asn = client.lookup_asn(9009).await?;
println!("{} {}", asn.name, asn.registry);   // M247 Europe SRL ripe

let domain = client.lookup_domain("example.com").await?;
println!("{}", domain.is_registered);        // true
```

`lookup` takes an address alone and refuses an AS or a domain before sending it. Neither of these is cached or batched, and `lookup_asn_with` and `lookup_domain_with` take the call's own retry budget and timeout.

### Caching

Address lookups are cached by default, so repeat lookups of the same address are free:

```rust
let client = Client::new()?;

let result = client.lookup("146.70.116.130").await?;
println!("{}", result.location.country);    // AT, API request

let result2 = client.lookup("146.70.116.130").await?;
println!("{}", result2.location.country);   // AT, no API request, result was cached
```

You can change the default cache variables (max size, TTL, etc) on initialization, or even disable it:

```rust
use std::time::Duration;

let client = Client::builder().cache(50_000, Duration::from_secs(6 * 60 * 60)).build()?;
let client_no_cache = Client::builder().no_cache().build()?;
```

### Private and reserved addresses

Private, loopback, link-local, documentation and multicast addresses (and their IPv6 equivalents, including the 6to4 and Teredo ranges) aren't on the public internet, so there's nothing to look up. The library answers them locally, so they cost no request and no quota:

```rust
let result = client.lookup("192.168.1.1").await?;
result.is_bogon;   // true, this answer was computed rather than served
result.is_tor;     // false
```

Because the answer is computed rather than served, it always carries every field, whatever plan you are on. Do not read your plan's shape off a private address.

The check is available on the client, which is handy when your inputs are addresses anyway:

```rust
client.is_bogon("10.0.0.1");   // true
client.is_bogon("8.8.8.8");    // false
```

It is also callable on its own, if you want it without a client:

```rust
internetdata::is_bogon("10.0.0.1");   // true
```

### Errors

Failures return an `internetdata::Error` carrying a `kind()` and a `retryable()` flag:

```rust
use internetdata::ErrorKind;

match client.lookup("146.70.116.130").await {
    Ok(result) => println!("{}", result.location.country),
    Err(err) => println!("{} {}", err.kind(), err.retryable()),
}
```

`kind()` is one of `BadRequest`, `Unauthorized`, `Forbidden`, `RateLimited`, `QuotaExceeded`, `ServerError`, `Network` or `Io`, the last of which is a database transfer that could not be written or that ended early. `message()` carries the API's own reason, such as `invalid API key` from a lookup or `NOT_LICENSED` from a database call.

Note that `RateLimited` and `QuotaExceeded` both arrive as HTTP 429 and are not the same thing. A rate limit carries `Retry-After`, and retrying after it works; a spent quota needs your allowance raised or the window to roll over. The library retries rate limits for you, but not if your quota is exceeded.

### Database downloads

The databases need an API key carrying the `db.download` scope. They hang off `client.database()`, which is where the sibling VPNDetection crate keeps the same seven calls:

```rust
for database in client.database().list().await? {
    println!("{} ({}): {}", database.name, database.standing, database.summary);
}
```

### The catalog

`list()` answers database FAMILIES. A license covers the family, while a download names one of its versions, so the ID you pass to `download`, `checksums` and `metadata` comes from `versions`:

```rust
for database in client.database().list().await? {
    println!("{} is {}", database.base, database.standing);   // "bogon_ip is licensed"

    for version in &database.versions {
        println!("  {} built in {:?}", version.id, version.formats);
    }
}
```

`standing` is `Licensed` for a live grant, `Expired` for one whose term has ended, and `Unlicensed` for a database published but never bought, so you can see what else exists without asking us. `license_type` says what your license lets you do with the data, and is `None` when there is no license.

### What is inside a database

`metadata` carries the column schema, a few real rows, the row count and the size of each artifact, without downloading anything. Poll it to decide whether today's build is worth fetching, and read `size` to know what a transfer will cost before you start it:

```rust
let meta = client.database().metadata("bogon_ip_v1").await?;

println!("{} rows, built {}", meta.entries, meta.updated);   // 1234 rows, built 2026-09-04
println!("{:?} bytes", meta.size.get("csvgz"));              // Some(760)

for column in &meta.schema["csvgz"] {
    println!("{}: {}", column.name, column.r#type);
}
```

### Downloading

```rust
use internetdata::DatabaseFormat;

let written = client.database().download("bogon_ip_v1", DatabaseFormat::Csvgz, "./bogon_ip.csv.gz").await?;
```

`download` streams to disk through a neighboring `.part` file, so nothing bigger than a chunk is ever held in memory and a transfer that dies half way neither leaves a truncated file nor replaces the copy already on disk.

```rust
let raw = client.database().download_bytes("bogon_asn_v1", DatabaseFormat::Csvgz).await?;
```

`download_bytes` holds the whole file in memory. The catalog spans seven orders of magnitude, from a few hundred bytes to several gigabytes, so check `metadata` first for anything you have not measured.

```rust
let url = client.database().download_url("bogon_ip_v1", DatabaseFormat::Csvgz).await?;
```

`download_url` hands back the time-limited link the API redirects to, so you can run the transfer yourself with whatever tool you like. It authorizes itself and carries none of your credentials, so it is safe to pass on; the link authorizes the START of a transfer, so one already running is not interrupted when it lapses.

Each of the three downloads the build that is live unless you name one. A `database.updated` webhook names the build it announces and carries its `sha256`, so a receiver that downloads after the next publish asks for that build, through the `_with` form of the call, to get the file the hash is of:

```rust
use internetdata::DownloadOptions;

let options = DownloadOptions::new().build(build_from_the_webhook);
let written = client.database().download_with("vpn_ip_v1", DatabaseFormat::Mmdb, "./vpn_ip_v1.mmdb", &options).await?;
```

A build is kept 7 days. Past that the call fails as a `BadRequest` with the message `BUILD_GONE`, and downloading without one fetches the live build.

Not every database is built in every format, which is why `versions` lists the ones it has. Asking for another is a `BadRequest`, not a gap.

### Verifying a download

```rust
let sums = client.database().checksums("bogon_ip_v1", DatabaseFormat::Csvgz).await?;
println!("{}", sums.sha256);
```

### Download history

`downloads` lists your organization's recent attempts, newest first, refusals included. A denial is what answers "it stopped working", and its absence answers nothing:

```rust
for attempt in client.database().downloads(Some(20)).await? {
    println!("{} {} {:?} {}", attempt.created, attempt.dataset_id, attempt.outcome, attempt.http_status.unwrap_or(0));
}
```

Pass `None` for the API's own default of 50. Anything above 200 is clamped.

### Sign in with OAuth (device flow)

A program running on the person's own machine can let them sign in with a browser and pick one of their API keys, instead of asking them to paste it:

```rust
use internetdata::{Client, DeviceAuthorizationOptions};

let client = Client::new()?;
let scope = DeviceAuthorizationOptions::new().scope("account.read apikeys.read apikeys.reveal");
let device = client.oauth().device_authorization_with("your-client-id", scope).await?;
println!("Open {} and enter {}", device.verification_uri, device.user_code);

let token = client.oauth().poll_device_token("your-client-id", &device).await?;
let Some(apikey) = token.apikey else {
    return Err("no API key came back: none was picked, or it cannot be shown again".into());
};
let keyed = Client::builder().api_key(apikey).build()?;
```

A denied sign-in fails with `OauthError::AccessDenied` and a code that ran out with `OauthError::ExpiredToken`. Client IDs are issued on request from support@internetdata.io, and `client.oauth().revoke("your-client-id", &refresh_token)` signs the machine out again.

### Sign in with OAuth (authorization code)

An app that can take a browser redirect signs the person in there instead, with a PKCE pair made for that one sign-in:

```rust
use internetdata::{AuthorizationUrlOptions, Client};

let client = Client::new()?;
let redirect_uri = "http://127.0.0.1:8765/callback";
let pkce = client.oauth().create_pkce()?;

let opts = AuthorizationUrlOptions::new().scope("apikeys.use").state("your-state");
let url =
    client.oauth().authorization_url_with("your-client-id", redirect_uri, &pkce.challenge, opts)?;
// Open url in the browser. Its redirect to redirect_uri carries code and state.
let token = client
    .oauth()
    .exchange_authorization_code("your-client-id", &code, &pkce.verifier, redirect_uri)
    .await?;
```

Check that `state` came back as you sent it before you exchange `code`, which works once. The client ID can also be the https URL of a client metadata document your app serves, and such an app is never handed a key, so `token.apikey` stays `None`.

### Register a client

An app with no client ID, and no client metadata document to use as one, registers itself, then signs in with the `client_id` it gets back as the client ID for `authorization_url` and `exchange_authorization_code`:

```rust
let client = Client::new()?;
let opts = RegisterOptions::new().client_name("Your App");
let registration =
    client.oauth().register_with(&["http://127.0.0.1:8765/callback"], opts).await?;
```

Each redirect URI must be https on a public host name, or http on a loopback address, which then matches any port. Nothing is kept until a person approves the client, so an app with no client ID saved can simply register again.

### TLS backends

`rustls` is the default, so the crate builds with no system libraries at all. If you would rather link the platform's TLS, or you already depend on `reqwest` with its own defaults and want one backend rather than two:

```toml
internetdata = { version = "2", default-features = false, features = ["native-tls"] }
```

### Calling from synchronous code

There is no blocking facade, on purpose: `reqwest::blocking` builds its own runtime and panics when constructed inside one, so a facade would fail for exactly the callers most likely to reach for it. If you have no runtime, make the cost visible instead:

```rust
let runtime = tokio::runtime::Runtime::new()?;
let client = Client::new()?;
let result = runtime.block_on(client.lookup("146.70.116.130"))?;
```

### Absent is not false

Unlimited leaves out `is_vpn`, `is_hosting`, `is_cdn` and the four proxy flags, the detail behind every flag but Tor's and relay's, and the paid members of the shared objects, `location.city` and `asn.rpki_status` among them; a free-plan key's answer carries `trial_expired`, true once its daily trial is spent. Every member a plan can leave out is an `Option`, because your key decides which of them the API sends. `None` means "not in your plan", which is not the same answer as `Some(false)`, which means "checked, and no".

```rust
result.is_hosting.unwrap_or(false);   // when you only want the flag
result.is_hosting.is_none();          // on Unlimited: not in your plan
result.trial_expired == Some(true);   // a free-plan key's daily trial is spent
```

## Other Libraries

There are official InternetData client libraries available for many languages including PHP, Python, Go, Java, and Ruby. See our GitHub at https://github.com/internetdata for more.

## About InternetData

Geolocation, anonymity, ownership and network data for IP addresses, AS numbers and domains, answered by the API or licensed as files you download and query yourself.

[<img src="https://s3.internetdata.io/internetdata-public/brand/mark.svg" alt="InternetData" height="64"/>](https://internetdata.io/)

## License

This project is licensed under the [MIT License](LICENSE).
