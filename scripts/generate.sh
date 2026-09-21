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
# and OAuth: v1 and IAM are not wrapped, and the OAuth accessor, like the
# VPNDetection crate's, hand-writes its three types. Generated, all of them would
# be dead code, and IAM's would need the uuid and serde_with crates, which
# nothing else here uses. Filtering the pinned spec instead would make the
# committed copy something other than what was published.

set -euo pipefail

cd "$(dirname "$0")/.."

GENERATOR_IMAGE="${GENERATOR_IMAGE:-openapitools/openapi-generator-cli:v7.25.0}"

PROPS="packageName=internetdata,supportAsync=true,hideGenerationTimestamp=true"

# `DbChecksums` keeps a `Db` prefix from a schema shared with v1; in a crate that
# is only a database client it is noise.
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

rm -rf .gen
mkdir -p .gen

docker run --rm \
    -v "$PWD/spec:/spec:ro" \
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

# The repo is rustfmt-clean and CI gates on it, so the generator's output is
# normalized here rather than left as a diff for the next `cargo fmt` to find.
./scripts/cargo.sh fmt

echo "regenerated src/generated/models from spec/openapi.yaml"
grep -m1 '^  version:' spec/openapi.yaml
