// The shared conformance corpus, generated into testdata/ by the monorepo and
// identical across every InternetData SDK. It is embedded rather than read at
// run time so a missing or malformed corpus is a build failure.
//
// Deliberately smaller than VPNDetection's: this API has no per-address lookup,
// so there is no bogon table, no plan ladder and no batch to pin. What is left
// is error mapping and the enum vocabularies, which is exactly where those SDKs
// drifted.
#![allow(dead_code)]

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Value;

pub fn load() -> Corpus {
    serde_json::from_str(include_str!("../../testdata/testdata.json")).expect("parsing the corpus")
}

#[derive(Deserialize)]
pub struct Corpus {
    pub errors: Vec<ErrorCase>,
    pub standings: Vec<String>,
    pub license_type: Vec<String>,
    pub formats: Vec<String>,
    pub visibility: Visibility,
}

#[derive(Deserialize)]
pub struct ErrorCase {
    pub name: String,
    pub status: u16,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    pub body: Value,
    pub expect: ErrorExpect,
}

#[derive(Deserialize)]
pub struct ErrorExpect {
    pub kind: String,
    pub retryable: bool,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default, rename = "retryAfterSeconds")]
    pub retry_after_seconds: Option<u64>,
}

/// The contract a listing has to keep: a private family is ABSENT for an
/// organization that does not license it, never present with an `unlicensed`
/// standing. The private ids are deliberately not in the corpus, since it is
/// committed into public repositories, so what is pinned is the rules a client
/// must follow rather than the data they protect.
#[derive(Deserialize)]
pub struct Visibility {
    pub why: String,
    #[serde(rename = "clientRules")]
    pub client_rules: Vec<String>,
}
