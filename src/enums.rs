//! The readers the generator does not emit for the API's closed vocabularies.
//!
//! Every one of these arrives as a string on the wire and is a Rust enum here,
//! so `{:?}` would print `Csvgz` where the API says `csvgz`. `as_str` is the
//! wire spelling and [`std::fmt::Display`] uses it, which is what makes a
//! catalog listing or a log line read as the API's own words. The conformance
//! suite asserts each one against what serde serializes, so a reader cannot
//! drift from the value actually sent.

use std::fmt;

use crate::models::database::LicenseType;
use crate::models::download::Outcome;
use crate::models::{DatabaseFormat, Standing};

/// Writes the `as_str` + `Display` pair for an enum the generator leaves bare.
macro_rules! wire_spelling {
    ($type:ty { $($variant:ident => $wire:literal),+ $(,)? }) => {
        impl $type {
            /// The spelling the API uses, which is not the Rust variant name.
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $wire),+
                }
            }
        }

        impl fmt::Display for $type {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

/// `as_str` alone, for an enum the spec NAMES - the generator emits a Display
/// for those, printing the same wire spelling, so writing a second one is a
/// conflicting impl rather than a duplicate.
macro_rules! wire_str {
    ($type:ty { $($variant:ident => $wire:literal),+ $(,)? }) => {
        impl $type {
            /// The spelling the API uses, which is not the Rust variant name.
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $wire),+
                }
            }
        }
    };
}

// Not every database is built in every format: the `_provider` catalogs are
// keyed by provider id rather than by IP range, so no MMDB exists for them.
// `Database::versions` says which formats each version has. Plain comments
// rather than doc ones, because rustdoc does not document what a macro
// invocation expands to.
wire_str!(DatabaseFormat { Csvgz => "csvgz", Mmdb => "mmdb" });

wire_str!(Standing {
    Licensed => "licensed",
    Expired => "expired",
    Unlicensed => "unlicensed",
});

wire_spelling!(LicenseType {
    Evaluation => "evaluation",
    Standard => "standard",
    Redistribute => "redistribute",
});

wire_spelling!(Outcome {
    Ok => "ok",
    Unauthorized => "unauthorized",
    Denied => "denied",
    Expired => "expired",
    Unknown => "unknown",
    Unavailable => "unavailable",
});
