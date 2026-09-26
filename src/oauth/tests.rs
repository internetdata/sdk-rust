// The corpus's poll cases, run through a clock that replaces both the sleep and
// the monotonic reading, so every wait is asserted exactly and none is slept.
// A unit test because the seam is private; tests/oauth.rs covers the rest of
// the accessor through the public surface, the real clock included.

#[path = "../../tests/support/mod.rs"]
mod support;

use std::sync::Mutex;
use std::time::Duration;

use super::{OauthOptions, PollClock};
use crate::DeviceAuthorization;
use support::corpus;
use support::oauth::{assert_failure, form, route};
use support::{REQUEST_BOUND, Stub};

/// Past this many waits the clock never wakes again, so a poll that does not
/// end fails the test's time limit instead of spinning.
const WAIT_BOUND: usize = 16;

#[tokio::test]
async fn the_poll_follows_every_corpus_case() {
    for case in corpus::load().oauth.poll.cases {
        let stub = Stub::start([]).await;
        stub.sequence("/oauth/token", case.responses.iter().map(route));
        let client = stub.anonymous().build().expect("build");
        let device: DeviceAuthorization =
            serde_json::from_value(case.device.clone()).expect("the corpus device");
        let clock = FakeClock::default();

        let outcome = tokio::time::timeout(
            Duration::from_secs(10),
            client.oauth().poll(&case.client_id, &device, OauthOptions::new(), &clock),
        )
        .await
        .unwrap_or_else(|_| panic!("{}: the poll never ended", case.name));

        let requests = stub.requests();
        assert!(requests.len() <= REQUEST_BOUND, "{}: the stub's bound was hit", case.name);
        assert_eq!(Some(requests.len()), case.expect.requests, "{}: requests", case.name);
        let waits: Vec<u64> = clock.waits().iter().map(Duration::as_secs).collect();
        assert_eq!(waits, case.expect.waits, "{}: waits", case.name);
        for call in &requests {
            assert_eq!((call.method.as_str(), call.path.as_str()), ("POST", "/oauth/token"));
            let fields = form(call);
            assert_eq!(fields.len(), 3, "{}: {fields:?}", case.name);
            assert_eq!(fields["grant_type"], "urn:ietf:params:oauth:grant-type:device_code");
            assert_eq!(fields["device_code"], device.device_code, "{}", case.name);
            assert_eq!(fields["client_id"], case.client_id, "{}", case.name);
        }
        match outcome {
            Ok(token) => {
                assert_eq!(case.expect.outcome, "token", "{}: settled with a token", case.name);
                for (name, want) in case.expect.token.iter().flatten() {
                    let got = support::oauth::token_member(&token, name);
                    assert_eq!(got.as_ref(), Some(want), "{}: {name}", case.name);
                }
            }
            Err(err) => assert_failure(&case.name, &err, &case.expect),
        }
    }
}

#[derive(Default)]
struct FakeClock {
    waits: Mutex<Vec<Duration>>,
}

impl FakeClock {
    fn waits(&self) -> Vec<Duration> {
        self.waits.lock().unwrap().clone()
    }
}

impl PollClock for FakeClock {
    fn now(&self) -> Duration {
        self.waits.lock().unwrap().iter().sum()
    }

    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send {
        let mut waits = self.waits.lock().unwrap();
        let bounded = waits.len() < WAIT_BOUND;
        if bounded {
            waits.push(duration);
        }
        async move {
            if !bounded {
                std::future::pending::<()>().await;
            }
        }
    }
}

/// `interval += 5` past the top of an `i64` panicked in a debug build and wrapped
/// to a negative, zero-second interval in release, so a server's `slow_down` near
/// that value would have polled without pause. It saturates, and the sleep after
/// it ends at the deadline.
#[tokio::test]
async fn a_slow_down_at_the_top_of_the_interval_saturates() {
    let stub = Stub::start([]).await;
    stub.sequence("/oauth/token", [support::Route::json(400, r#"{"error":"slow_down"}"#)]);
    let client = stub.anonymous().build().expect("build");
    let device: DeviceAuthorization = serde_json::from_value(serde_json::json!({
        "device_code": "mo_dc_x", "user_code": "BCDF-GHJK",
        "verification_uri": "https://app.example.test/device",
        "expires_in": i64::MAX, "interval": i64::MAX - 2,
    }))
    .expect("device");
    let clock = FakeClock::default();

    let err = client
        .oauth()
        .poll("internetdata-cli", &device, OauthOptions::new(), &clock)
        .await
        .expect_err("expired");

    assert!(matches!(err, crate::OauthError::ExpiredToken(ref e) if e.status.is_none()), "{err:?}");
    assert_eq!(stub.count(), 1);
    let waits: Vec<u64> = clock.waits().iter().map(Duration::as_secs).collect();
    assert_eq!(waits, [i64::MAX as u64 - 2, 2]);
}
