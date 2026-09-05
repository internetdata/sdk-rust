// A stub origin that answers from a table and records what it was asked for, so
// "the key never went to object storage" and "the redirect was not followed" are
// asserted rather than assumed.
//
// Hand-rolled on tokio rather than taken from a mock-HTTP crate because one of
// the things that has to be proved here is outside what one offers: an origin
// that PROMISES a multi-gigabyte body, so a followed redirect is caught by the
// request count rather than by waiting for the transfer.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use internetdata::{Client, ClientBuilder};

pub mod corpus;

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
}

/// One request the stub was asked for. The headers come along because two of
/// the download guarantees are about what a request did NOT carry, and a header
/// that was never sent is invisible to any assertion made on the response.
#[derive(Clone, Debug)]
pub struct Call {
    pub path: String,
    pub headers: Vec<(String, String)>,
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
    calls: Vec<Call>,
}

pub struct Stub {
    pub base_url: String,
    state: Arc<Mutex<State>>,
}

impl Stub {
    /// Binds an ephemeral port and starts serving. The accept loop is detached
    /// and dies with the test process.
    pub async fn start(routes: impl IntoIterator<Item = (String, Route)>) -> Arc<Self> {
        let state = Arc::new(Mutex::new(State {
            routes: routes.into_iter().collect(),
            ..State::default()
        }));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();

        let stub =
            Arc::new(Self { base_url: format!("http://127.0.0.1:{port}"), state: state.clone() });
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let state = state.clone();
                tokio::spawn(async move {
                    let _ = serve(socket, state).await;
                });
            }
        });
        stub
    }

    /// Adds a route after binding, for a response whose body has to name the
    /// stub's own address.
    pub fn route(&self, path: impl Into<String>, route: Route) {
        self.state.lock().unwrap().routes.insert(path.into(), route);
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

async fn serve(mut socket: TcpStream, state: Arc<Mutex<State>>) -> std::io::Result<()> {
    let call = match read_request(&mut socket).await? {
        Some(call) => call,
        None => return Ok(()),
    };
    let path = call.path.clone();

    let route = {
        let mut state = state.lock().unwrap();
        state.calls.push(call);
        state.routes.get(&path).cloned()
    };

    // An unrouted path gets what the real API gives an unknown database, so a
    // test that forgets a route fails as a refusal rather than as a hang.
    let route = route.unwrap_or_else(|| Route::json(404, r#"{"rc":"UNKNOWN_DATASET"}"#));
    write_response(&mut socket, &route).await
}

/// The request line and its headers. The path is stripped of any query string,
/// which is the key routes are held under; the full target is kept in the
/// synthetic `x-stub-target` header, so an assertion about a credential in a
/// query string has something to read.
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
    let Some(target) = lines.next().and_then(|line| line.split_whitespace().nth(1)) else {
        return Ok(None);
    };

    let mut headers = vec![("x-stub-target".to_owned(), target.to_owned())];
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_lowercase(), value.trim().to_owned()));
        }
    }
    let path = target.split('?').next().unwrap_or(target).to_owned();
    Ok(Some(Call { path, headers }))
}

async fn write_response(socket: &mut TcpStream, route: &Route) -> std::io::Result<()> {
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
    socket.write_all(route.body.as_bytes()).await?;
    socket.flush().await?;
    // A promised body that is never written would leave the client waiting for
    // the rest of it, so the connection is closed instead: whoever followed the
    // redirect gets an error, and the request is on the record either way.
    socket.shutdown().await
}
