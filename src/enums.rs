//! The readers the generator does not emit for the API's closed vocabularies.
//!
//! Every one of these arrives as a string on the wire and is a Rust enum here,
//! so `{:?}` would print `Csvgz` where the API says `csvgz`. `as_str` is the
//! wire spelling and [`std::fmt::Display`] uses it, which is what makes a
//! catalog listing or a log line read as the API's own words. The conformance
//! suite asserts each one against what serde serializes, so a reader cannot
//! drift from the value actually sent.

use std::fmt;

use crate::models::checksums_response::Format;
use crate::models::database::{Redistribution, Standing};
use crate::models::database_version::Formats;
use crate::models::download::Outcome;

/// Writes the four `as_str` + `Display` pairs, which are otherwise the same
/// eleven lines four times over.
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

// Not every database is built in every format: the `_provider` catalogs are
// keyed by provider id rather than by IP range, so no MMDB exists for them.
// `Database::versions` says which formats each version has. Plain comments
// rather than doc ones, because rustdoc does not document what a macro
// invocation expands to.
wire_spelling!(Format { Csvgz => "csvgz", Mmdb => "mmdb" });

wire_spelling!(Formats { Csvgz => "csvgz", Mmdb => "mmdb" });

wire_spelling!(Standing {
    Licensed => "licensed",
    Expired => "expired",
    Unlicensed => "unlicensed",
});

wire_spelling!(Redistribution {
    Evaluation => "evaluation",
    Internal => "internal",
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

// The spec spells the same two-value format enum twice, once beside a version's
// built formats and once beside a checksum's, so the generator emits two types
// for it. These are the total conversions between them, and every call that
// takes a format takes `impl Into<Format>`, so `download(id, version.formats[0],
// path)` works without a caller ever seeing the seam. Exhaustive matches, so a
// third format added to the spec fails the build here rather than being silently
// dropped on one side.
impl From<Formats> for Format {
    fn from(format: Formats) -> Self {
        match format {
            Formats::Csvgz => Self::Csvgz,
            Formats::Mmdb => Self::Mmdb,
        }
    }
}

impl From<Format> for Formats {
    fn from(format: Format) -> Self {
        match format {
            Format::Csvgz => Self::Csvgz,
            Format::Mmdb => Self::Mmdb,
        }
    }
}
