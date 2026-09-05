# [<img src="https://s3.internetdata.io/internetdata-public/brand/mark.svg" alt="InternetData" height="28"/>](https://internetdata.io/) InternetData Rust Client Library

[![crates.io](https://img.shields.io/crates/v/internetdata.svg)](https://crates.io/crates/internetdata)
[![docs.rs](https://img.shields.io/docsrs/internetdata)](https://docs.rs/internetdata)
[![license](https://img.shields.io/crates/l/internetdata.svg)](LICENSE)

The official Rust client library for the [InternetData](https://internetdata.io) API.

The library helps you browse, verify and download the licensed IP intelligence databases: what each one contains, how fresh today's build is, what it hashes to, and the bytes themselves.

## Getting Started

```bash
cargo add internetdata
```

Requires Rust 1.85 or newer. Everything is `async` and runs on tokio.

## Usage

Every database published today needs an API key carrying the `db.download` scope. Create one in the console, then pass it in. `api_key` is optional on the builder - leave it out and no `Authorization` header is sent at all, ready for a database served without a licence:

```rust
use internetdata::Client;

let client = Client::builder()
    .api_key(std::env::var("INTERNETDATA_API_KEY")?)
    .build()?;

for database in client.database().list().await? {
    println!("{} ({}): {}", database.name, database.standing, database.summary);
}
```

Every call hangs off `client.database()`, which is the whole of this API and is where the sibling VPNDetection crate keeps the same seven calls.

Every setting has a default, and `Client::builder()` is where you change one:

```rust
let client = Client::builder().api_key(key).retries(4).build()?;
```

### The catalog

`list()` answers database FAMILIES. A licence covers the family, while a download names one of its versions, so the id you pass to `download`, `checksums` and `metadata` comes from `versions`:

```rust
for database in client.database().list().await? {
    println!("{} is {}", database.base, database.standing);   // "bogon_ip is licensed"

    for version in &database.versions {
        println!("  {} built in {:?}", version.id, version.formats);
    }
}
```

`standing` is `Licensed` for a live grant, `Expired` for one whose term has ended, and `Unlicensed` for a database published but never bought, so you can see what else exists without asking us. `redistribution` says what your licence lets you do with the data, and is `None` when there is no licence.

**Your catalog is not everyone's catalog.** A database commissioned for a single customer is absent from this listing entirely for an organization that does not license it, rather than listed as `Unlicensed`. The server decides what you may see, so treat the answer as this key's answer: do not build a catalog from anywhere else, and do not reuse one organization's listing for another key.

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
use internetdata::Format;

let written = client.database().download("bogon_ip_v1", Format::Csvgz, "./bogon_ip.csv.gz").await?;
```

`download` streams to disk through a neighboring `.part` file, so nothing bigger than a chunk is ever held in memory and a transfer that dies half way neither leaves a truncated file nor replaces the copy already on disk.

```rust
let raw = client.database().download_bytes("bogon_asn_v1", Format::Csvgz).await?;
```

`download_bytes` holds the whole file in memory. The catalog spans seven orders of magnitude, from a few hundred bytes to several gigabytes, so check `metadata` first for anything you have not measured.

```rust
let url = client.database().download_url("bogon_ip_v1", Format::Csvgz).await?;
```

`download_url` hands back the time-limited link the API redirects to, so you can run the transfer yourself with whatever tool you like. It authorizes itself and carries none of your credentials, so it is safe to pass on; the link authorizes the START of a transfer, so one already running is not interrupted when it lapses.

Not every database is built in every format, which is why `versions` lists the ones it has. Asking for another is a `BadRequest`, not a gap.

### Verifying a download

```rust
let sums = client.database().checksums("bogon_ip_v1", Format::Csvgz).await?;
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

### Errors

Failures return an `internetdata::Error` carrying a `kind()` and a `retryable()` flag:

```rust
use internetdata::ErrorKind;

match client.database().metadata("bogon_ip_v1").await {
    Ok(meta) => println!("{}", meta.entries),
    Err(err) => println!("{} {}", err.kind(), err.retryable()),
}
```

`kind()` is one of `BadRequest`, `Unauthorized`, `Forbidden`, `RateLimited`, `QuotaExceeded`, `ServerError`, `Network` or `Io`, the last of which is a database transfer that could not be written or that ended early. `message()` carries the API's own reason, such as `NOT_LICENSED` or `UNKNOWN_DATASET`.

Note that `RateLimited` and `QuotaExceeded` both arrive as HTTP 429 and are not the same thing. A rate limit is when the API faces extreme traffic bursts and so retrying later works; but a spent quota needs your allowance raised or the window to roll over. The library retries rate limits for you, but not if your quota is exceeded.

### TLS backends

`rustls` is the default, so the crate builds with no system libraries at all. If you would rather link the platform's TLS, or you already depend on `reqwest` with its own defaults and want one backend rather than two:

```toml
internetdata = { version = "1", default-features = false, features = ["native-tls"] }
```

### Calling from synchronous code

There is no blocking facade, on purpose: `reqwest::blocking` builds its own runtime and panics when constructed inside one, so a facade would fail for exactly the callers most likely to reach for it. If you have no runtime, make the cost visible instead:

```rust
let runtime = tokio::runtime::Runtime::new()?;
let client = Client::builder().api_key(key).build()?;
let databases = runtime.block_on(client.database().list())?;
```

## Other Libraries

There are official InternetData client libraries available for many languages including PHP, Python, Go, Java, Ruby, and many popular frameworks such as Django, Rails, and Laravel. See our GitHub at https://github.com/internetdata for more.

## About InternetData

IP, ASN and Domain data to reveal unique insights about the internet. APIs, Databases and Live Feeds available.

[<img src="https://s3.internetdata.io/internetdata-public/brand/mark.svg" alt="InternetData" width="96"/>](https://internetdata.io/)

## License

This project is licensed under the [MIT License](LICENSE).
