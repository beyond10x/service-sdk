---
format: aep.planning-md/1
id: story:live-parent-authority-conformance
kind: story
status: implemented
title: Version live parent authority and executable service scenarios
summary: Add a v2 inherited-authority obligation, injected auth facts, and generated Connector scenario execution.
relations:
- derived_from: epic:builder-runtime
- serves: vision:composable-services
revision: 4
---
# Version live parent authority and executable service scenarios

## Acceptance

- A new `sdk.derive.inherit-parent-authority/v2` provider preserves the frozen v1 meaning and derives child projection owner/scopes from the current folded parent.
- Parent transfer cannot leave stale child visibility; engine tests prove the former owner value is replaced.
- Generated Connector factories accept a deployment-injected authenticated authority-facts resolver while retaining a safe principal/group default.
- The resolver receives only receiver-verified principal context; tenant, realm, authority, user, and executor remain absent from operation inputs.
- service-builder emits generated Rust conformance tests for every declared scenario fixture.
- The conformance runner invokes intents and queries through the generated Connector factory and proves exact `None` versus `Some("default")` partition authority.
- Existing deterministic generation, cross-artifact conformance, runtime, documentation, and AEP gates pass.
