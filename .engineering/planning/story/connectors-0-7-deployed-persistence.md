---
format: aep.planning-md/1
id: story:connectors-0-7-deployed-persistence
kind: story
status: active
title: Consume Connectors 0.7 with the deployed persistence contract
relations:
- decomposes: epic:builder-runtime
- serves: vision:composable-services
scope:
- confidence: cited
  path: Cargo.lock
- confidence: cited
  path: crates/service-catalog
- confidence: cited
  path: crates/service-conformance/Cargo.toml
- confidence: cited
  path: crates/service-connectors
revision: 7
---
## Outcome

Apply the upstream 0.7 protocol adaptation to the SDK revision already consumed by generated application deployments. Keep the existing persistence contract at this compatibility boundary.

## Evidence and decision

The upstream correction at 2cc5d56c694b9081907c37d7c41cc404cbbe4bdd passed the full gate and merged. A real local Devcenter composition using it then failed startup because the newer Eventlog constructor admits only isolated loopback databases; that revision also requires separate hosted migrations, DML-only roles and connection budgets. These persistence changes are independent of the requested Connector protocol release and were not implemented by this consumer migration.

Use baseline 0118bd3f9d63ead5d525fb39324b1e5e13c4ab1a, already consumed by deployed generated services, and apply the same reviewed rate-advice constructor and Connector pin changes. No test bypass, fake model, insecure verifier or new persistence exception is introduced. Keep this published compatibility revision separate from SDK main; main retains its correct newer hosted persistence contract.

## Acceptance

The three direct protocol consumers pin the exact Connectors 0.7.0 release, both generated operation descriptions supply absent rate advice truthfully, and the complete baseline SDK gate passes before publication. The baseline host has no direct Connector protocol dependency and needs no source edit. Devcenter must resolve this published Git source without temporary path patches, build and run the actual k3d Connector, then prove real Files, PTY and user-authorized model replies before promotion.

## Authorization

Continue the operator-authorized Connector 0.7 adoption and correction of deployment failures. This single bounded story proceeds through draft, proposed and active under the existing implementation authorization.

## Verification

The complete baseline SDK gate passed after the 0.7 protocol adaptation: formatting, all-target clippy, workspace tests, generated fixture freshness, documentation, web checks, release action contract and AEP validation. Eventlog remains exactly at b7e8f0d87b01c403415546d311952cb155caf16f. No persistence implementation or transport admission was modified. Real container composition verification belongs to the downstream acceptance run.
