#!/bin/bash

# Regenerates the wire MODELS under src/generated/models from the PINNED spec.
#
# The generator runs in its official container, so nothing has to be installed
# locally, and it reads the committed spec rather than a URL, so the build is
# reproducible and offline. Refresh the spec with scripts/download-spec.sh, run
# this, and commit both together so a reviewer sees which spec produced which
# client.
#
# MODELS ONLY, deliberately. `-g rust --library reqwest` also emits request
# functions, and two of the semantics this SDK is required to get right are
# unreachable through them: its ResponseContent carries `status` and `content`
# but no HEADERS, so a 429's Retry-After (the only thing separating a retryable
# rate limit from a spent quota) cannot be read, and `downloadDatabaseV2` is
# generated as `Result<(), _>`, discarding the 302 whose Location IS that
# endpoint's answer. Both are pinned by the tests. Five GET operations with no
# request bodies are ~70 lines of reqwest in transport.rs; patching the
# generated ones on every regeneration would be more code and more fragile. The
# models are kept because they carry the spec's optionality and nullability
# exactly, which no hand-written struct would stay honest about.
#
# The output is COMMITTED. crates.io publishes SOURCE and docs.rs compiles it,
# and neither runs a pre-build step, so a gitignored client would ship a crate
# that cannot compile itself.
#
# Only the models the hand-written layer uses are generated, selected by name.
# The pinned spec is the whole published document, so it also describes v1, IAM
# and OAuth: v1 and IAM are not wrapped, and the OAuth accessor and the
# entitlement, like the VPNDetection crate's, hand-write their types. Generated,
# all of them would be dead code, and IAM's would need the uuid and serde_with
# crates, which nothing else here uses. Filtering the pinned spec instead would
# make the committed copy something other than what was published.

set -euo pipefail

cd "$(dirname "$0")/.."

GENERATOR_IMAGE="${GENERATOR_IMAGE:-openapitools/openapi-generator-cli:v7.25.0}"

PROPS="packageName=internetdata,supportAsync=true,hideGenerationTimestamp=true"

# `DbChecksums` keeps a `Db` prefix from a schema shared with v1; under
# `client.database()` it is noise.
MODELS="DbChecksums=Checksums"

# The three wrapper schemas are inline in the spec, so the generator names them
# after the operation and status code (databaseChecksumV2_200_response).
# --model-name-mappings does NOT reach an inline schema; only
# --inline-schema-name-mappings does, keyed by the generator's own placeholder
# name rather than by the Rust name.
NAMES="listDatabases_200_response=DatabaseList"
NAMES="${NAMES},listDownloads_200_response=DownloadList"
NAMES="${NAMES},databaseChecksumV2_200_response=ChecksumsResponse"

# What lib.rs re-exports, and every schema those reference: the list does NOT
# follow a $ref, so a model left off it is simply not generated and the build
# fails on its name. A schema is named as the SPEC names it, or by its mapped
# name when it is one of the inline wrappers above.
SELECTED="Database:DatabaseVersion:DatabaseFormat:Standing:Download"
SELECTED="${SELECTED}:DatabaseMetadata:DatabaseMetadataColumn:DbChecksums"
SELECTED="${SELECTED}:DatabaseList:DownloadList:ChecksumsResponse"
SELECTED="${SELECTED}:LookupResponse:Location:Asn:Company:AbuseContact:Whois:Rwhois"
SELECTED="${SELECTED}:HostedDomains:WebDetail:BatchLookupRequest:BatchLookupError"
SELECTED="${SELECTED}:AsnLookupResponse:AsnPrefix:AsnHostedDomains:DomainLookupResponse:DomainDns"
SELECTED="${SELECTED}:DnsAddressRecord:DnsMxRecord:DnsNsRecord:DnsCnameRecord"
SELECTED="${SELECTED}:DomainIpRanges:DomainIpRange:ClassDetail:ProxyDetail:PubproxyDetail"
SELECTED="${SELECTED}:CarrierDetail:SatelliteDetail:SshDetail:SmtpDetail:DnsResolverDetail"
SELECTED="${SELECTED}:BitTorrentDetail:DisposableDetail"
SELECTED="${SELECTED}:DomainHost:CohostedDomains:DomainWeb:DomainWebLinks:DomainRedirects:DomainBacklinks"
SELECTED="${SELECTED}:BotDetail:UserAgentVerdict"

rm -rf .gen
mkdir -p .gen/spec

# The generator reads a copy of the pinned spec with two of its changes held
# back until a major, so a minor changes no public type: every `asn` stays the
# i32 it has been, not the i64 that `format: int64` asks for, and the AS's
# `country` and `rpki_status` stay Option<String>, as they were while the spec
# left them optional. The API serves both members on every answer, and no AS it
# serves is past i32. The pin itself stays the published document. Either edit
# finding nothing to change stops the script, rather than reaching the release
# unnoticed.
#
# The eight flags spec 2026.11.26 stopped requiring (Unlimited no longer serves
# them) stay the plain `bool` they have been, so a caller reads what it read
# before: the generator reads them as required again, and one the wire leaves
# out defaults to false below, while a served one that is no boolean still
# fails the call. src/lookup.rs keeps which ones the wire left out, for the
# middleware. The next major types them `Option<bool>`.
HELD_FLAGS="is_bot is_carrier is_satellite is_ssh is_smtp is_web is_dns_resolver is_bittorrent"
export HELD_FLAGS

# The seen days below (spec 2026.11.23) stay the String they have been rather
# than the chrono::NaiveDate their `format: date` asks for; each still reads
# through src/dates.rs, which refuses a value that is not a full-date (below).
SEEN_DAYS="AsnPrefix.first_seen BitTorrentDetail.first_seen BitTorrentDetail.last_seen"
SEEN_DAYS="${SEEN_DAYS} BotDetail.first_seen BotDetail.last_seen ClassDetail.last_seen"
SEEN_DAYS="${SEEN_DAYS} DisposableDetail.first_seen DisposableDetail.last_seen"
SEEN_DAYS="${SEEN_DAYS} DnsAddressRecord.last_seen DnsCnameRecord.last_seen DnsMxRecord.last_seen"
SEEN_DAYS="${SEEN_DAYS} DnsNsRecord.last_seen DnsResolverDetail.first_seen DnsResolverDetail.last_seen"
SEEN_DAYS="${SEEN_DAYS} ProxyDetail.first_seen ProxyDetail.last_seen PubproxyDetail.last_seen"
SEEN_DAYS="${SEEN_DAYS} SmtpDetail.first_seen SmtpDetail.last_seen SshDetail.first_seen SshDetail.last_seen"
SEEN_DAYS="${SEEN_DAYS} WebDetail.first_seen WebDetail.last_seen"
export SEEN_DAYS
perl -0777 -pe '
    $n = s/(\n {8}asn:\n {10}type: integer\n) {10}format: int64\n/$1/g;
    die "no int64 asn member to hold back\n" unless $n;
    s{(\n    LookupResponse:\n.*?\n      required:\n)((?: {8}- \w+\n)*)}{
        my ($head, $list) = ($1, $2);
        for my $flag (split " ", $ENV{HELD_FLAGS}) {
            die "the spec requires $flag again: stop holding it\n" if $list =~ /^ {8}- \Q$flag\E$/m;
            $list .= "        - $flag\n";
        }
        $head . $list;
    }se or die "LookupResponse has no required list to hold the flags in\n";
    s{(\n    Asn:\n.*?\n      required:\n(?: {8}- \w+\n)*?) {8}- country\n {8}- rpki_status\n}{$1}s
        or die "the AS no longer requires country and rpki_status\n";
    for my $day (split " ", $ENV{SEEN_DAYS}) {
        my ($schema, $member) = split /\./, $day;
        s{(\n    \Q$schema\E:\n(?: {6}.*\n|\n)*? {8}\Q$member\E:\n {10}type: string\n) {10}format: date\n}{$1}
            or die "$day is no longer a format: date member to hold back\n";
    }
' spec/openapi.yaml >.gen/spec/openapi.yaml

docker run --rm \
    -v "$PWD/.gen/spec:/spec:ro" \
    -v "$PWD/.gen:/out" \
    "$GENERATOR_IMAGE" generate \
    -i /spec/openapi.yaml \
    -g rust --library reqwest \
    -o /out \
    --global-property "models=${SELECTED}" \
    --global-property supportingFiles,modelDocs=false,modelTests=false \
    --model-name-mappings "$MODELS" \
    --inline-schema-name-mappings "$NAMES" \
    --additional-properties="$PROPS" \
    >/dev/null

# src/generated/mod.rs is HAND-WRITTEN and is not regenerated.
rm -rf src/generated/models
mkdir -p src/generated
cp -R .gen/src/models src/generated/models

rm -rf .gen

# A member the spec makes required becomes a parameter of the generated
# `new()`, so a minor that adds one would break every call to it. `is_bot`
# (spec 2026.11.21) is read as required, so an answer without it still fails
# the call, while `LookupResponse::new` keeps the arguments it had and starts
# it at false. A member named here that the generator stops passing to `new()`
# stops the script.
for member in is_bot; do
    MEMBER="$member" perl -0777 -pi -e '
        my $m = $ENV{MEMBER};
        s/(pub fn new\([^)]*?)\s*\b$m: bool,([^)]*\) -> LookupResponse \{\s*LookupResponse \{[^}]*?)\b$m,/$1$2$m: false,/s
            or die "LookupResponse::new no longer takes $m: teach this script its shape\n";
    ' src/generated/models/lookup_response.rs
done

# A held flag the wire leaves out reads false. A member named here that the
# generator stops emitting as a plain `bool` stops the script.
for member in $HELD_FLAGS; do
    MEMBER="$member" perl -0777 -pi -e '
        my $m = $ENV{MEMBER};
        s/(#\[serde\(rename = "$m")(\)\]\s+pub $m: bool,)/$1, default$2/
            or die "LookupResponse.$m is no plain bool member: teach this script its shape\n";
    ' src/generated/models/lookup_response.rs
done

# Every date member reads through src/dates.rs, which takes only the form the
# spec's `format` names: chrono's own serde reader is a relaxed one, and took
# `2026-10-5` or `+0500` as some date (dates.rs). A date member in a shape
# these three do not rewrite stops the script, rather than reading through
# chrono unnoticed.
perl -0777 -pi -e '
    s/(#\[serde\(rename = "\w+", deserialize_with = ")Option::deserialize("\)\]\s+pub \w+: Option<chrono::DateTime<chrono::FixedOffset>>,)/${1}crate::dates::option_date_time$2/g;
    s/(#\[serde\(rename = "\w+")(\)\]\s+pub \w+: chrono::DateTime<chrono::FixedOffset>,)/$1, deserialize_with = "crate::dates::date_time"$2/g;
    s/(#\[serde\(rename = "\w+")(\)\]\s+pub \w+: chrono::NaiveDate,)/$1, deserialize_with = "crate::dates::date"$2/g;
' src/generated/models/*.rs
if grep -h -B1 -E '^\s*pub \w+: .*chrono::' src/generated/models/*.rs | grep -E '#\[serde' | grep -v 'crate::dates::' ; then
    echo "a date member above reads through chrono's relaxed parser: teach this script its shape" >&2
    exit 1
fi

# The seen days held back above are Strings, read through src/dates.rs's
# `day` and `option_day`, which hand back the value as served once it is a
# full-date. One in a shape these two do not rewrite stops the script.
for day in $SEEN_DAYS; do
    schema=${day%.*}
    file="src/generated/models/$(perl -pe 's/(?<!^)([A-Z])/_$1/g; $_ = lc' <<<"$schema").rs"
    MEMBER="${day#*.}" DAY="$day" perl -0777 -pi -e '
        my $m = $ENV{MEMBER};
        s/(#\[serde\(rename = "$m")(, skip_serializing_if = "Option::is_none"\)\]\s+pub $m: Option<String>,)/$1, default, deserialize_with = "crate::dates::option_day"$2/
            or s/(#\[serde\(rename = "$m")(\)\]\s+pub $m: String,)/$1, deserialize_with = "crate::dates::day"$2/
            or die "$ENV{DAY} is in a shape this script does not rewrite\n";
    ' "$file"
done

# A `type: object` with no properties is generated as serde_json::Value, which
# takes any JSON at all, so a `sample` row of `1` or `null` came back as the
# answer (2.7.4). Each one reads through src/objects.rs, and one in another
# shape stops the script, as a date member does.
perl -0777 -pi -e '
    s/(#\[serde\(rename = "\w+", skip_serializing_if = "Option::is_none")(\)\]\s+pub \w+: Option<std::collections::HashMap<String, Vec<serde_json::Value>>>,)/$1, default, deserialize_with = "crate::objects::option_object_lists"$2/g;
' src/generated/models/*.rs
if grep -h -B1 -E '^\s*pub \w+: .*serde_json::Value' src/generated/models/*.rs | grep -E '#\[serde' | grep -v 'crate::objects::' ; then
    echo "an object member above reads as any JSON value: teach this script its shape" >&2
    exit 1
fi

# A closed vocabulary in an ANSWER is the API's to extend, and an exhaustive
# enum failed the whole call on a value it was not built with (2.12.0). Each
# generated enum is opened: `#[non_exhaustive]`, no `Copy`, and a last variant,
# `Other`, carrying the value as served, while a number or a null still fails.
# `Other` rather than `Unknown`, which `Outcome` already has for the API's
# `unknown`. The generated `Display` goes too, since its match has no arm for
# `Other`; src/enums.rs writes one for every enum. An enum in another shape
# stops the script.
perl -0777 -pi -e '
    s/#\[derive\(Clone, Copy, (Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)\)\]\npub enum (\w+) \{\n((?:    #\[serde\(rename = "[^"]+"\)\]\n    \w+,\n)+)\n?\}/#[derive(Clone, $1)]\n#[non_exhaustive]\npub enum $2 {\n$3    \/\/\/ A value this release was not built with, as the API served it.\n    #[serde(untagged)]\n    Other(String),\n}/g;
    s/\nimpl std::fmt::Display for \w+ \{\n.*?\n\}\n//gs;
' src/generated/models/*.rs
if grep -h -B1 '^pub enum' src/generated/models/*.rs | grep -v -e '^pub enum' -e '^#\[non_exhaustive\]$' -e '^--$' ; then
    echo "an enum above refuses a value it was not built with: teach this script its shape" >&2
    exit 1
fi

# The repo is rustfmt-clean and CI gates on it, so the generator's output is
# normalized here rather than left as a diff for the next `cargo fmt` to find.
./scripts/cargo.sh fmt

echo "regenerated src/generated/models from spec/openapi.yaml"
grep -m1 '^  version:' spec/openapi.yaml
