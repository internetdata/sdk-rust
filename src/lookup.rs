use std::ops::Deref;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

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
/// Eight flags are the exception, kept `bool` for the type they have always
/// had: `is_bot`, `is_carrier`, `is_satellite`, `is_ssh`, `is_smtp`, `is_web`,
/// `is_dns_resolver` and `is_bittorrent`. They are a paid plan's or the
/// trial's, and an answer that leaves one out, Unlimited's, reads `false`, as
/// it did before Unlimited stopped serving them (2026-11-26). The detail
/// beside each (`result.bot`, `result.carrier`) tells the two apart: `None`
/// where the plan lacks the flag, present where it is served. So is the AS's
/// `type`, a `String` that reads `""` when your plan leaves it out. The next
/// major makes them `Option`s.
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
    /// The answer as it was served, with the wire's own optionality intact but
    /// for the eight flags and `asn.type` above, which read `false` and `""`
    /// when it left one out.
    pub answer: LookupResponse,
}

impl Lookup {
    pub(crate) fn served(answer: LookupResponse) -> Self {
        Self { answer }
    }
}

/// The members a paid plan or the trial serves and Unlimited does not, held
/// at the type they have always had until the next major, as JSON pointers
/// into the wire answer: the eight flags the generator's `HELD_FLAGS` names,
/// and the AS's type, which it holds beside them.
pub(crate) const HELD: [&str; 9] = [
    "/is_bot",
    "/is_carrier",
    "/is_satellite",
    "/is_ssh",
    "/is_smtp",
    "/is_web",
    "/is_dns_resolver",
    "/is_bittorrent",
    "/asn/type",
];

/// A [`Lookup`] and which held members its wire answer left out. The public
/// value reads those `false` or `""`; the middleware reads them as absent,
/// which is what makes a condition on one report the plan gap instead of
/// never matching quietly. It is what the cache and the requests in flight
/// hold, so a cached answer keeps what the wire said.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Served {
    pub(crate) lookup: Lookup,
    pub(crate) absent: Vec<&'static str>,
}

impl Served {
    /// An answer computed here, which carries every member.
    pub(crate) fn whole(lookup: Lookup) -> Self {
        Self { lookup, absent: Vec::new() }
    }

    pub(crate) fn from_value(wire: serde_json::Value) -> Result<Self, serde_json::Error> {
        let absent = HELD.into_iter().filter(|member| wire.pointer(member).is_none()).collect();
        let answer = serde_json::from_value::<LookupResponse>(wire)?;
        Ok(Self { lookup: Lookup::served(answer), absent })
    }

    /// The answer as the wire served it: JSON that leaves out each held member
    /// the wire left out.
    pub(crate) fn wire(&self) -> serde_json::Value {
        let mut wire = serde_json::to_value(&self.lookup.answer).unwrap_or(serde_json::Value::Null);
        for path in &self.absent {
            let Some((parent, member)) = path.rsplit_once('/') else { continue };
            if let Some(members) = wire.pointer_mut(parent).and_then(|at| at.as_object_mut()) {
                members.remove(member);
            }
        }
        wire
    }
}

impl<'de> Deserialize<'de> for Served {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = serde_json::Value::deserialize(deserializer)?;
        Self::from_value(wire).map_err(D::Error::custom)
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
