//! The fixtures the test files share: whether this run has a key at all, one
//! catalog listing for the whole binary, the shape rules a listing has to keep,
//! and the rule a lookup answer keeps against the wire.
//!
//! Every request goes through [`recorder::Recorder`], a recording reverse proxy
//! in front of the API, because reqwest offers no seam of its own and two of the
//! things that have to be proved here are about the REQUEST rather than the
//! answer: that the key reached the wire, or did not, and that it did not travel
//! any further than the API.

pub mod recorder;

use std::sync::Arc;

use internetdata::{Client, Database, Standing};
use recorder::{Fact, Recorder};
use serde_json::Value;

/// The deployment these tests run against, reached through the client's own
/// base-URL option, which is what makes that option worth testing.
pub const STAGING: &str = "https://api-staging.internetdata.io";

/// Staging's apex, which issues its tokens and serves its OAuth endpoints.
pub const STAGING_APEX: &str = "https://staging.internetdata.io";

/// Production, which the keyless lookups run against too: they need no secret.
pub const PROD: &str = "https://api.internetdata.io";

/// The address, the AS and the domain the lookups ask about. AS112 announces
/// four prefixes, so its full answer stays under what the recorder keeps.
pub const PROBE: &str = "146.70.116.130";
pub const PROBE_ASN: u32 = 112;
pub const PROBE_DOMAIN: &str = "example.com";

/// The one credential this suite has, a free-plan organization's. There is no
/// plan ladder here: a keyless lookup is Unlimited's answer, the key's is its
/// plan's, and the databases a read-only key is licensed to exercise the rest.
pub const KEY_SECRET: &str = "INTERNETDATA_STAGING_KEY";

/// 8 MiB, against databases of a few hundred and a few thousand bytes. Three
/// orders of magnitude of headroom, so tripping it means the suite is pointed
/// somewhere unintended, which is exactly when a transfer must not go ahead: the
/// published catalog reaches several gigabytes and CI would pull every byte.
pub const CEILING: i64 = 8 << 20;

/// This run's key, or `None` when it cannot be exercised.
///
/// Empty counts as absent: Actions interpolates a secret that does not exist to
/// an EMPTY STRING rather than leaving the variable unset, and a client built
/// with an empty key sends no authorization header at all, so an empty key would
/// run as an anonymous client and every call would fail as a 401 that looks like
/// a broken credential.
pub fn key() -> Option<String> {
    std::env::var(KEY_SECRET).ok().map(|key| key.trim().to_owned()).filter(|key| !key.is_empty())
}

/// A reason, or `None` when this run can go ahead.
pub fn skip_reason() -> Option<String> {
    key().is_none().then(|| format!("{KEY_SECRET} is not set, so there is no key to run with"))
}

/// Rust's test harness has no skip, so a run without a key says so and passes.
/// Printed rather than silent: a suite that quietly tests nothing is the exact
/// failure this crate exists to prevent, and `run.sh` passes `--nocapture` so
/// the line lands in the log.
#[macro_export]
macro_rules! skip_unless {
    ($reason:expr) => {
        if let Some(reason) = $reason {
            println!("SKIPPED: {reason}");
            return;
        }
    };
}

/// A client pointed at the recorder in front of staging, and the recorder
/// itself. A recorder per caller, so one test's request record cannot be read
/// through another's.
pub async fn client_for() -> (Client, Arc<Recorder>) {
    let key = key().expect("client_for called without a key: guard with skip_unless!");
    let recorder = Recorder::start(STAGING, Some(key.clone())).await;
    let client =
        Client::builder().base_url(&recorder.base_url).api_key(key).build().expect("building");
    (client, recorder)
}

/// A client with no key, pointed at a recorder in front of `origin`.
pub async fn keyless_client_for(origin: &str) -> (Client, Arc<Recorder>) {
    let recorder = Recorder::start(origin, None).await;
    let client = Client::builder().base_url(&recorder.base_url).build().expect("building");
    (client, recorder)
}

/// A lookup answer as the client holds it against the answer as it came off the
/// wire: every member served reached the result, with the value served, and the
/// result holds no member the wire did not serve, at every depth. That is what
/// makes a served `false` survive and an absent member stay absent rather than
/// defaulted.
pub fn assert_same_answer(label: &str, wire: &Value, held: &Value) {
    same_member(label, "the answer", wire, held);
}

fn same_member(label: &str, path: &str, wire: &Value, held: &Value) {
    match (wire, held) {
        (Value::Object(wire), Value::Object(held)) => {
            for name in wire.keys() {
                assert!(
                    held.contains_key(name),
                    "{label}: the wire served {path}.{name} and the client dropped it, or the \
                     pinned spec does not model it yet"
                );
            }
            for (name, value) in held {
                let served = wire.get(name).unwrap_or_else(|| {
                    panic!("{label}: the client holds {path}.{name}, which the wire never served")
                });
                same_member(label, &format!("{path}.{name}"), served, value);
            }
        }
        (Value::Array(wire), Value::Array(held)) => {
            assert_eq!(wire.len(), held.len(), "{label}: {path} lost or gained entries");
            for (i, (served, value)) in wire.iter().zip(held).enumerate() {
                same_member(label, &format!("{path}[{i}]"), served, value);
            }
        }
        (Value::Number(served), Value::Number(value)) => {
            assert_eq!(served.as_f64(), value.as_f64(), "{label}: {path} reads other than served");
        }
        _ => assert_eq!(wire, held, "{label}: {path} reads other than served"),
    }
}

/// One listing for the whole binary. Memoized because every test here starts
/// from "what does this key license", and asking five times would be five
/// identical requests against a real deployment.
static CATALOG: tokio::sync::OnceCell<Catalog> = tokio::sync::OnceCell::const_new();

pub struct Catalog {
    pub databases: Vec<Database>,
    /// The listing as it came off the wire. The client keeps the decoded
    /// families; a field the pinned spec does not model is only visible here.
    pub wire: Vec<Value>,
    pub facts: Vec<Fact>,
}

impl Catalog {
    /// The versions this organization may actually download, family order.
    pub fn licensed(&self) -> Vec<(&str, internetdata::DatabaseFormat)> {
        self.databases
            .iter()
            .filter(|database| database.standing == Standing::Licensed)
            .flat_map(|database| &database.versions)
            .flat_map(|version| version.formats.iter().map(|format| (version.id.as_str(), *format)))
            .collect()
    }

    /// A published database this organization holds no license for, discovered
    /// rather than named: hard-coding one goes stale the day it is bought, and
    /// `standing` is the field that answers this question anyway.
    pub fn unlicensed(&self) -> Option<&str> {
        self.databases
            .iter()
            .find(|database| database.standing == Standing::Unlicensed)
            .and_then(|database| database.versions.first())
            .map(|version| version.id.as_str())
    }
}

pub async fn catalog() -> &'static Catalog {
    CATALOG.get_or_init(fetch_catalog).await
}

async fn fetch_catalog() -> Catalog {
    let (client, recorder) = client_for().await;
    let databases = client.database().list().await.expect("list");

    let wire = recorder
        .json_body("/api/v2/database/list")
        .expect("no JSON listing was captured, so nothing here can be checked against the wire");
    let wire = wire["databases"].as_array().expect("databases is an array").clone();

    // Checked HERE rather than in one test, so no comparison anywhere can be
    // made against a run that silently went unauthenticated. An unsent key is a
    // 401, which would fail loudly today, but the assertion is what keeps that
    // true if the API ever grows an anonymous listing.
    assert!(recorder.carried_key(), "the key never reached the wire");

    Catalog { databases, wire, facts: recorder.facts() }
}

/// What holds of every family in a listing, whatever this key licenses.
pub fn assert_catalog_shape(catalog: &Catalog) {
    assert!(!catalog.databases.is_empty(), "this key can see no databases at all");
    assert_eq!(
        catalog.databases.len(),
        catalog.wire.len(),
        "the decode dropped or invented a family"
    );

    for (family, wire) in catalog.databases.iter().zip(&catalog.wire) {
        let base = &family.base;
        assert!(!base.is_empty(), "a family carries no base");
        assert!(!family.name.is_empty(), "{base} carries no name");
        assert_eq!(
            wire["base"].as_str(),
            Some(base.as_str()),
            "the decode reordered the listing, so nothing below lines up"
        );

        // A license covers the FAMILY and a download names one of its versions,
        // so this list is what makes list -> download possible at all. The
        // VPNDetection spec claimed `{id, formats}` on the family until it was
        // corrected, which broke that path in every SDK.
        assert!(!family.versions.is_empty(), "{base} carries no versions");
        for version in &family.versions {
            assert!(!version.id.is_empty(), "{base} has a version with no ID");
            assert!(!version.formats.is_empty(), "{} is built in no format", version.id);
        }

        // Presence follows the standing: an unlicensed family has no terms
        // because there is no license to state them.
        match family.standing {
            Standing::Unlicensed => {
                assert!(
                    family.license_type.is_none(),
                    "{base} is unlicensed but names a license_type right"
                );
                assert!(family.starts.is_none(), "{base} is unlicensed but has a start date");
            }
            Standing::Licensed | Standing::Expired => {
                assert!(
                    family.license_type.is_some(),
                    "{base} is {} but names no license_type right",
                    family.standing
                );
                assert!(family.starts.is_some(), "{base} is {} with no start", family.standing);
            }
        }
    }
}
