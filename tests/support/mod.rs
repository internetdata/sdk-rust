// A stub origin that answers from a table and records what it was asked for, so
// "the key never went to object storage", "never touched the network" and "kept
// at most N in flight" are asserted rather than assumed.
//
// Hand-rolled on tokio rather than taken from a mock-HTTP crate because two of
// the things that have to be proved here are outside what one offers: the PEAK
// number of concurrent connections, and an origin that PROMISES a multi-gigabyte
// body so a followed redirect is caught by the request count rather than by
// waiting for the transfer.
#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use internetdata::{Client, ClientBuilder};

pub mod corpus;
pub mod oauth;

/// A response the stub is prepared to give for one path.
#[derive(Clone, Default)]
pub struct Route {
    pub status: u16,
    pub body: String,
    pub headers: Vec<(String, String)>,
    /// Sent as `Content-Length` while the body stays empty, so a client that
    /// follows a redirect it should not is told the file is enormous without the
    /// test having to produce one.
    pub promised_length: Option<u64>,
    /// Waits this long before sending anything, head included.
    pub wait: Option<Duration>,
    /// Writes the head and this many bytes of the body, then sends nothing more
    /// until the client gives up, so only a deadline covering the BODY ends it.
    pub stall_after: Option<usize>,
    /// Writes the body a byte at a time at this pace, so no single read ever
    /// stalls long enough for a per-read timeout to fire.
    pub trickle: Option<Duration>,
}

impl Route {
    pub fn ok(body: impl Into<String>) -> Self {
        Self { status: 200, body: body.into(), ..Self::default() }
    }

    pub fn json(status: u16, body: impl Into<String>) -> Self {
        Self { status, body: body.into(), ..Self::default() }
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    pub fn promising(mut self, bytes: u64) -> Self {
        self.promised_length = Some(bytes);
        self
    }

    pub fn waiting(mut self, wait: Duration) -> Self {
        self.wait = Some(wait);
        self
    }

    pub fn stalling_after(mut self, bytes: usize) -> Self {
        self.stall_after = Some(bytes);
        self
    }

    pub fn trickling(mut self, pace: Duration) -> Self {
        self.trickle = Some(pace);
        self
    }
}

/// One request the stub was asked for. The headers come along because two of
/// the download guarantees are about what a request did NOT carry, and a header
/// that was never sent is invisible to any assertion made on the response.
#[derive(Clone, Debug)]
pub struct Call {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    /// The request body, empty for a GET.
    pub body: String,
    /// When the request finished arriving, for measuring the gap between two.
    pub at: Instant,
}

impl Call {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Default)]
struct State {
    routes: HashMap<String, Route>,
    sequences: HashMap<String, VecDeque<Route>>,
    calls: Vec<Call>,
    in_flight: usize,
    peak: usize,
}

pub struct Stub {
    pub base_url: String,
    state: Arc<Mutex<State>>,
    delay: Duration,
}

impl Stub {
    /// Binds an ephemeral port and starts serving. The accept loop is detached
    /// and dies with the test process.
    pub async fn start(routes: impl IntoIterator<Item = (String, Route)>) -> Arc<Self> {
        Self::start_with_delay(routes, Duration::ZERO).await
    }

    /// The delay is what makes concurrent requests overlap, so a peak-in-flight
    /// measurement has something to measure.
    pub async fn start_with_delay(
        routes: impl IntoIterator<Item = (String, Route)>,
        delay: Duration,
    ) -> Arc<Self> {
        let state = Arc::new(Mutex::new(State {
            routes: routes.into_iter().collect(),
            ..State::default()
        }));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();

        let stub = Arc::new(Self {
            base_url: format!("http://127.0.0.1:{port}"),
            state: state.clone(),
            delay,
        });
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let state = state.clone();
                tokio::spawn(async move {
                    let _ = serve(socket, state, delay).await;
                });
            }
        });
        stub
    }

    /// Successful lookups for a set of addresses, which is what most cases want.
    pub fn ok_routes(ips: &[&str]) -> Vec<(String, Route)> {
        ips.iter().map(|ip| (format!("/{ip}"), Route::ok(answer(ip)))).collect()
    }

    /// Adds a route after binding, for a response whose body has to name the
    /// stub's own address.
    pub fn route(&self, path: impl Into<String>, route: Route) {
        self.state.lock().unwrap().routes.insert(path.into(), route);
    }

    /// Answers a path with these responses in order, ahead of any route. Once
    /// they run out every further request there is a 599, so an extra attempt
    /// is counted and fails rather than picking up an answer meant for another.
    pub fn sequence(&self, path: impl Into<String>, routes: impl IntoIterator<Item = Route>) {
        self.state.lock().unwrap().sequences.insert(path.into(), routes.into_iter().collect());
    }

    /// A builder already pointed at this stub and carrying a key, which is what
    /// every endpoint needs.
    pub fn client(&self) -> ClientBuilder {
        Client::builder().base_url(&self.base_url).api_key(KEY)
    }

    /// The same, without a key, for the unauthorized case.
    pub fn anonymous(&self) -> ClientBuilder {
        Client::builder().base_url(&self.base_url)
    }

    pub fn count(&self) -> usize {
        self.state.lock().unwrap().calls.len()
    }

    pub fn calls(&self) -> Vec<String> {
        self.state.lock().unwrap().calls.iter().map(|call| call.path.clone()).collect()
    }

    pub fn requests(&self) -> Vec<Call> {
        self.state.lock().unwrap().calls.clone()
    }

    pub fn peak_in_flight(&self) -> usize {
        self.state.lock().unwrap().peak
    }

    /// The full request target, query string included, for the call to `path`.
    /// A credential smuggled into a query string is only visible here.
    pub fn target(&self, path: &str) -> Option<String> {
        self.requests()
            .iter()
            .find(|call| call.path == path)
            .and_then(|call| call.header("x-stub-target").map(str::to_owned))
    }
}

/// Recognizable in a failure, and obviously not a real credential.
pub const KEY: &str = "test-api-key-never-real";

/// The least a lookup answer carries, Unlimited's members with nothing flagged,
/// for an address.
pub fn answer(ip: &str) -> String {
    answer_value(ip).to_string()
}

/// [`answer`], as a value a test can change before serving it.
pub fn answer_value(ip: &str) -> serde_json::Value {
    serde_json::json!({
        "ip": ip,
        "is_bogon": false,
        "is_tor": false,
        "is_relay": false,
        "is_anycast": false,
        "is_carrier": false,
        "is_satellite": false,
        "is_ssh": false,
        "is_smtp": false,
        "is_web": false,
        "is_dns_resolver": false,
        "is_bittorrent": false,
        "tor": {},
        "relay": {},
        "location": {
            "continent": "EU",
            "continent_name": "Europe",
            "country": "AT",
            "country_name": "Austria",
            "timezone": "Europe/Vienna",
            "currency_code": "EUR",
            "currency_name": "Euro",
            "currency_symbol": "E",
            "flag_emoji": "AT",
            "flag_unicode": "U+1F1E6 U+1F1F9",
        },
        "asn": {
            "asn": 9009,
            "name": "M247 Europe SRL",
            "domain": "m247global.com",
            "type": "hosting",
            "prefix": "146.70.116.0/24",
        },
        "company": {"name": "M247 LTD Vienna Infrastructure", "domain": "m247.ro"},
        "hostname": "",
        "abuse": {"email": "abuse@m247.ro"},
    })
}

/// Past this many requests the stub answers nothing at all, so a client caught
/// in a loop fails its test's own time limit instead of growing without bound.
pub const REQUEST_BOUND: usize = 64;

async fn serve(
    mut socket: TcpStream,
    state: Arc<Mutex<State>>,
    delay: Duration,
) -> std::io::Result<()> {
    let call = match read_request(&mut socket).await? {
        Some(call) => call,
        None => return Ok(()),
    };
    let path = call.path.clone();
    let body = call.body.clone();

    let over_bound = {
        let mut state = state.lock().unwrap();
        state.calls.push(call);
        state.calls.len() > REQUEST_BOUND
    };
    if over_bound {
        return hold(&mut socket).await;
    }
    let route = {
        let mut state = state.lock().unwrap();
        state.in_flight += 1;
        state.peak = state.peak.max(state.in_flight);
        if let Some(queue) = state.sequences.get_mut(&path) {
            Some(queue.pop_front().unwrap_or_else(|| Route::json(599, r#"{"stub":"exhausted"}"#)))
        } else if path == "/batch" {
            Some(batch_route(&state.routes, &body))
        } else {
            state.routes.get(&path).cloned()
        }
    };
    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }

    // An unrouted path gets what the real API gives an unknown database, so a
    // test that forgets a route fails as a refusal rather than as a hang.
    let route = route.unwrap_or_else(|| Route::json(404, r#"{"rc":"UNKNOWN_DATASET"}"#));
    let result = write_response(&mut socket, &route).await;
    state.lock().unwrap().in_flight -= 1;
    result
}

/// The request line, its headers and any body. The path is percent-decoded and
/// stripped of any query string, which is the key routes are held under; the
/// full target is kept in the synthetic `x-stub-target` header, so an assertion
/// about a credential in a query string has something to read.
async fn read_request(socket: &mut TcpStream) -> std::io::Result<Option<Call>> {
    let mut request = Vec::new();
    let mut byte = [0u8; 1];
    while !request.ends_with(b"\r\n\r\n") {
        if socket.read(&mut byte).await? == 0 {
            return Ok(None);
        }
        request.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&request);
    let mut lines = text.lines();
    let mut request_line = lines.next().unwrap_or_default().split_whitespace();
    let (Some(method), Some(target)) = (request_line.next(), request_line.next()) else {
        return Ok(None);
    };

    let mut headers = vec![("x-stub-target".to_owned(), target.to_owned())];
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_lowercase(), value.trim().to_owned()));
        }
    }
    let path = percent_decode(target.split('?').next().unwrap_or(target));
    // A POST carries its body after the blank line, sized by Content-Length.
    let length = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    if length > 0 {
        socket.read_exact(&mut body).await?;
    }
    Ok(Some(Call {
        method: method.to_owned(),
        path,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
        at: Instant::now(),
    }))
}

/// A POST /batch is answered the way the API answers one: every address the
/// table knows is a result if its route is a 200 and an entry error otherwise,
/// and an unknown address is the 400 the API gives a string that is not one.
/// One call however many addresses, which is what the request counts measure.
fn batch_route(routes: &HashMap<String, Route>, body: &str) -> Route {
    let ips: Vec<String> = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| serde_json::from_value(value.get("ips")?.clone()).ok())
        .unwrap_or_default();
    let mut results = serde_json::Map::new();
    let mut errors = serde_json::Map::new();
    for ip in ips {
        match routes.get(&format!("/{ip}")) {
            None => {
                errors.insert(
                    ip,
                    serde_json::json!({
                        "status": 400,
                        "error": "not a valid IP address, AS number or domain",
                    }),
                );
            }
            Some(route) if route.status == 200 => {
                let value: serde_json::Value =
                    serde_json::from_str(&route.body).unwrap_or(serde_json::Value::Null);
                results.insert(ip, value);
            }
            Some(route) => {
                let message = serde_json::from_str::<serde_json::Value>(&route.body)
                    .ok()
                    .and_then(|value| value.get("error")?.as_str().map(str::to_owned))
                    .unwrap_or_default();
                errors.insert(ip, serde_json::json!({"status": route.status, "error": message}));
            }
        }
    }
    Route::ok(serde_json::json!({"results": results, "errors": errors}).to_string())
}

async fn write_response(socket: &mut TcpStream, route: &Route) -> std::io::Result<()> {
    if let Some(wait) = route.wait {
        tokio::time::sleep(wait).await;
    }
    let length = route.promised_length.unwrap_or(route.body.len() as u64);
    let mut head = format!(
        "HTTP/1.1 {} X\r\nContent-Type: application/json\r\nContent-Length: {length}\r\nConnection: close\r\n",
        route.status
    );
    for (name, value) in &route.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    socket.write_all(head.as_bytes()).await?;
    if let Some(bytes) = route.stall_after {
        socket.write_all(&route.body.as_bytes()[..bytes]).await?;
        socket.flush().await?;
        return hold(socket).await;
    }
    if let Some(pace) = route.trickle {
        for byte in route.body.as_bytes() {
            socket.write_all(&[*byte]).await?;
            socket.flush().await?;
            tokio::time::sleep(pace).await;
        }
        return socket.shutdown().await;
    }
    socket.write_all(route.body.as_bytes()).await?;
    socket.flush().await?;
    // A promised body that is never written would leave the client waiting for
    // the rest of it, so the connection is closed instead: whoever followed the
    // redirect gets an error, and the request is on the record either way.
    socket.shutdown().await
}

/// Sends nothing more, and returns once the client closes the connection.
async fn hold(socket: &mut TcpStream) -> std::io::Result<()> {
    let mut sink = [0u8; 256];
    while socket.read(&mut sink).await? > 0 {}
    Ok(())
}

fn percent_decode(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&path[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
