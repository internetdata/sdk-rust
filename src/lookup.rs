use std::ops::Deref;

use crate::models::LookupResponse;

/// What a lookup answers.
///
/// An `Option` member is one your plan does not include. It never means "we
/// could not check", so `None` and `Some(false)` are genuinely different
/// answers: `None` is "not in your plan", `Some(false)` is "checked, and no".
/// This is the single most important semantic in the library, and it is why
/// every member a plan can leave out is an `Option`, the paid ones inside a
/// shared object too (`location.city`, `asn.rpki_status`).
///
/// A detail object that is present but empty means the flag above it is false.
/// A populated one always carries every one of its keys, empty values included.
///
/// The wire answer's fields are reachable straight off a `Lookup` through
/// [`Deref`], so `result.is_tor` and `result.location` read as if they were its
/// own, and [`Lookup::answer`] is the untouched wire object. `result.is_bogon`
/// is true for an answer computed locally, which happens for a bogon and only
/// for a bogon.
#[derive(Debug, Clone, PartialEq)]
pub struct Lookup {
    /// The answer as it was served, with the wire's own optionality intact.
    pub answer: LookupResponse,
}

impl Lookup {
    pub(crate) fn served(answer: LookupResponse) -> Self {
        Self { answer }
    }
}

/// `result.is_tor` rather than `result.answer.is_tor`. `Lookup` hides nothing
/// of the served answer, so the deref is a widening rather than the inheritance
/// `Deref` is usually cautioned against.
impl Deref for Lookup {
    type Target = LookupResponse;

    fn deref(&self) -> &LookupResponse {
        &self.answer
    }
}

/// Readers that treat "not in your plan" as false, for callers who only want to
/// know whether an address is flagged. Read the `Option` itself wherever absent
/// and false must be told apart, which is the whole reason they are options.
///
/// `result.is_hosting.unwrap_or(false)` says the same thing and is what the
/// README teaches; these exist so a long match arm does not have to.
impl Lookup {
    pub fn is_vpn_or_false(&self) -> bool {
        self.is_vpn.unwrap_or(false)
    }

    pub fn is_hosting_or_false(&self) -> bool {
        self.is_hosting.unwrap_or(false)
    }

    pub fn is_cdn_or_false(&self) -> bool {
        self.is_cdn.unwrap_or(false)
    }

    pub fn is_resproxy_or_false(&self) -> bool {
        self.is_resproxy.unwrap_or(false)
    }

    pub fn is_dcproxy_or_false(&self) -> bool {
        self.is_dcproxy.unwrap_or(false)
    }

    pub fn is_mobproxy_or_false(&self) -> bool {
        self.is_mobproxy.unwrap_or(false)
    }

    pub fn is_pubproxy_or_false(&self) -> bool {
        self.is_pubproxy.unwrap_or(false)
    }
}
