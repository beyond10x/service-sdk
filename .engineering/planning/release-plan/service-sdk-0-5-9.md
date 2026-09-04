---
format: aep.planning-md/1
id: release-plan:service-sdk-0-5-9
kind: release-plan
status: active
title: Release Service SDK 0.5.9
summary: Publish the Connectors 0.6.2 factory identity for generated services.
relations:
- delivers: story:align-current-connectors-service-identity
revision: 2
---
## Outcome

Publish Service SDK 0.5.9 with every generated Connector factory and catalog bound to the exact Connectors 0.6.2 service identity.

## Qualification

The complete SDK gate passes, the annotated tag peels to the bot-authored main commit, and downstream generators consume that exact revision.
