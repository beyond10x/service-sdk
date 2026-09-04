#!/usr/bin/env bash
set -euo pipefail

: "${COMPONENT_SERVICE:?component service is required}"
: "${COMPONENT_VERSION:?component version is required}"
: "${ESS_REVISION:?ESS revision is required}"
: "${CHART_NAMESPACE:?chart namespace is required}"
: "${EVIDENCE_REPOSITORY:?evidence repository is required}"
: "${BUNDLE_REPOSITORY:?bundle repository is required}"
: "${PNPM_VERSION:?pnpm version is required}"

case "$COMPONENT_SERVICE" in
  "" | *[!a-z0-9-]*)
    printf 'invalid ESS service identifier: %s\n' "$COMPONENT_SERVICE" >&2
    exit 2
    ;;
esac
case "$COMPONENT_VERSION" in
  *.*.*) ;;
  *)
    printf 'invalid component version: %s\n' "$COMPONENT_VERSION" >&2
    exit 2
    ;;
esac

declared_version="$(awk '$1 == "version:" { print $2; exit }' service.yaml)"
image_repository="$(awk '$1 == "image_repository:" { print $2; exit }' service.yaml)"
test "$declared_version" = "$COMPONENT_VERSION"
test -n "$image_repository"
if [ "${GITHUB_REF_TYPE:-}" = tag ]; then
  test "$GITHUB_REF_NAME" = "$COMPONENT_VERSION"
fi

corepack enable
corepack prepare "pnpm@$PNPM_VERSION" --activate
mkdir -p target/release
set -o pipefail
task check 2>&1 | tee target/release/conformance.log

ess_bin="$PWD/target/ess/$ESS_REVISION/bin/ess"
test -x "$ess_bin"
build_path=generated/deployment/build.yaml
build_ir=generated/deployment/build.ir.json
component_ir=generated/deployment/component.ir.json
runtime_ir=generated/deployment/runtime.ir.json
image_tag="$image_repository:$COMPONENT_VERSION"

"$ess_bin" generate build execute \
  --path "$build_path" \
  --workdir . \
  --projection-out target/release/app-buildkit \
  --target app \
  --set "app.tags=$image_tag" \
  --push
image_digest="$(
  docker buildx imagetools inspect "$image_tag" \
    --format '{{json .Manifest.Digest}}' | jq -r .
)"
image_raw="$(docker buildx imagetools inspect "$image_tag" --raw)"
image_platform_digest="$(
  jq -r '
    if has("manifests") then
      first(.manifests[] | select(.platform.os == "linux" and .platform.architecture == "amd64") | .digest)
    else empty
    end
  ' <<<"$image_raw"
)"
if [ -z "$image_platform_digest" ]; then
  image_platform_digest="$image_digest"
fi

"$ess_bin" generate build execute \
  --path "$build_path" \
  --workdir . \
  --projection-out target/release/chart-buildkit \
  --target chart
chart_archive="$(find out/chart -type f -name '*-chart.tgz' -print -quit)"
test -n "$chart_archive"
chart_name="$(helm show chart "$chart_archive" | awk '$1 == "name:" { print $2; exit }')"
chart_push="$(helm push "$chart_archive" "$CHART_NAMESPACE")"
printf '%s\n' "$chart_push"
chart_digest="$(sed -n 's/^Digest: //p' <<<"$chart_push" | tail -1)"
chart_reference="${CHART_NAMESPACE#oci://}/$chart_name"
test -n "$chart_digest"

cosign sign --yes "${image_repository}@${image_digest}"
cosign sign --yes "${chart_reference}@${chart_digest}"
image_signature_reference="$(cosign triangulate "${image_repository}@${image_digest}")"
chart_signature_reference="$(cosign triangulate "${chart_reference}@${chart_digest}")"
image_signature_digest="$(
  docker buildx imagetools inspect "$image_signature_reference" \
    --format '{{json .Manifest.Digest}}' | jq -r .
)"
chart_signature_digest="$(
  docker buildx imagetools inspect "$chart_signature_reference" \
    --format '{{json .Manifest.Digest}}' | jq -r .
)"

syft scan "${image_repository}@${image_digest}" \
  -o cyclonedx-json=target/release/sbom.json
semantic_digest="$(jq -r .semantic_digest "$runtime_ir")"
build_digest="$(jq -r .build_digest "$runtime_ir")"
runtime_digest="sha256:$(sha256sum "$runtime_ir" | awk '{print $1}')"
source_commit="${GITHUB_SHA:-$(git rev-parse HEAD)}"
run_reference="${GITHUB_SERVER_URL:-local}/${GITHUB_REPOSITORY:-unknown}/actions/runs/${GITHUB_RUN_ID:-unknown}"

jq -n \
  --arg source_commit "$source_commit" \
  --arg run "$run_reference" \
  --arg image "${image_repository}@${image_digest}" \
  --arg chart "${chart_reference}@${chart_digest}" \
  --arg semantic_digest "$semantic_digest" \
  --arg build_digest "$build_digest" \
  --arg runtime_digest "$runtime_digest" \
  '{
    format: "slsa-provenance-summary/1",
    source_commit: $source_commit,
    run: $run,
    subjects: [$image, $chart],
    ess: {
      semantic_digest: $semantic_digest,
      build_digest: $build_digest,
      runtime_digest: $runtime_digest
    }
  }' > target/release/provenance.json
jq -n \
  --arg image_reference "$image_signature_reference" \
  --arg image_digest "$image_signature_digest" \
  --arg chart_reference "$chart_signature_reference" \
  --arg chart_digest "$chart_signature_digest" \
  '{
    format: "sigstore-signature-set/1",
    signatures: [
      {subject: "runtime", reference: $image_reference, digest: $image_digest},
      {subject: "chart", reference: $chart_reference, digest: $chart_digest}
    ]
  }' > target/release/signatures.json

publish_evidence() {
  local kind="$1"
  local path="$2"
  local media_type="$3"
  local destination="$EVIDENCE_REPOSITORY:$COMPONENT_VERSION-$kind"
  local digest
  digest="$(
    oras push \
      --no-tty \
      --artifact-type "application/vnd.beyond10x.ess.evidence.$kind.v1" \
      --format 'go-template={{.digest}}' \
      "$destination" \
      "$path:$media_type"
  )"
  printf '%s\t%s\n' "$destination" "$digest"
}

IFS=$'\t' read -r provenance_reference provenance_digest < <(
  publish_evidence provenance target/release/provenance.json application/json
)
IFS=$'\t' read -r sbom_reference sbom_digest < <(
  publish_evidence sbom target/release/sbom.json application/vnd.cyclonedx+json
)
IFS=$'\t' read -r signature_reference signature_digest < <(
  publish_evidence signature target/release/signatures.json application/json
)
IFS=$'\t' read -r conformance_reference conformance_digest < <(
  publish_evidence conformance target/release/conformance.log text/plain
)

evidence="$(
  jq -n \
    --arg provenance_reference "$provenance_reference" \
    --arg provenance_digest "$provenance_digest" \
    --arg sbom_reference "$sbom_reference" \
    --arg sbom_digest "$sbom_digest" \
    --arg signature_reference "$signature_reference" \
    --arg signature_digest "$signature_digest" \
    --arg conformance_reference "$conformance_reference" \
    --arg conformance_digest "$conformance_digest" \
    '{
      provenance: {reference: $provenance_reference, digest: $provenance_digest},
      sbom: {reference: $sbom_reference, digest: $sbom_digest},
      signature: {reference: $signature_reference, digest: $signature_digest},
      conformance: {reference: $conformance_reference, digest: $conformance_digest}
    }'
)"

runtime_release_unit="$COMPONENT_SERVICE-runtime"
chart_release_unit="$COMPONENT_SERVICE-chart"
jq -n \
  --arg release_unit "$runtime_release_unit" \
  --arg system "$COMPONENT_SERVICE" \
  --arg version "$COMPONENT_VERSION" \
  --arg source_commit "$source_commit" \
  --arg semantic_digest "$semantic_digest" \
  --arg build_digest "$build_digest" \
  --arg runtime_digest "$runtime_digest" \
  --arg reference "$image_repository" \
  --arg digest "$image_digest" \
  --arg platform_digest "$image_platform_digest" \
  --argjson evidence "$evidence" \
  '{
    format: "ess-release/1",
    release_unit: $release_unit,
    system: $system,
    version: $version,
    source_commit: $source_commit,
    semantic_digest: $semantic_digest,
    build_digest: $build_digest,
    runtime_digest: $runtime_digest,
    artifacts: {
      app: {
        build_output: "app",
        kind: "oci_image",
        reference: $reference,
        digest: $digest,
        platforms: {"linux/amd64": $platform_digest}
      }
    },
    evidence: $evidence
  }' > target/release/runtime-release.json
jq -n \
  --arg release_unit "$chart_release_unit" \
  --arg system "$COMPONENT_SERVICE" \
  --arg version "$COMPONENT_VERSION" \
  --arg source_commit "$source_commit" \
  --arg semantic_digest "$semantic_digest" \
  --arg build_digest "$build_digest" \
  --arg runtime_digest "$runtime_digest" \
  --arg reference "$chart_reference" \
  --arg digest "$chart_digest" \
  --argjson evidence "$evidence" \
  '{
    format: "ess-release/1",
    release_unit: $release_unit,
    system: $system,
    version: $version,
    source_commit: $source_commit,
    semantic_digest: $semantic_digest,
    build_digest: $build_digest,
    runtime_digest: $runtime_digest,
    artifacts: {
      chart: {
        build_output: "chart",
        kind: "helm_chart",
        reference: $reference,
        digest: $digest
      }
    },
    evidence: $evidence
  }' > target/release/chart-release.json

"$ess_bin" generate release verify \
  --path target/release/runtime-release.json \
  --build-ir "$build_ir" \
  --runtime-ir "$runtime_ir"
"$ess_bin" generate release verify \
  --path target/release/chart-release.json \
  --build-ir "$build_ir" \
  --runtime-ir "$runtime_ir"
"$ess_bin" generate release bundle \
  --component-ir "$component_ir" \
  --build-ir "$build_ir" \
  --runtime-ir "$runtime_ir" \
  --release target/release/runtime-release.json \
  --release target/release/chart-release.json \
  --out target/release/ess-release-bundle.json
publish_output="$(
  "$ess_bin" generate release publish \
    --path target/release/ess-release-bundle.json \
    --to "$BUNDLE_REPOSITORY:$COMPONENT_VERSION"
)"
printf '%s\n' "$publish_output"
bundle_digest="${publish_output##* }"
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  {
    printf '### %s ESS component %s\n\n' "$COMPONENT_SERVICE" "$COMPONENT_VERSION"
    printf -- '- Bundle: %s@%s\n' "$BUNDLE_REPOSITORY" "$bundle_digest"
    printf -- '- Runtime: %s@%s\n' "$image_repository" "$image_digest"
    printf -- '- Chart: %s@%s\n' "$chart_reference" "$chart_digest"
  } >> "$GITHUB_STEP_SUMMARY"
fi
