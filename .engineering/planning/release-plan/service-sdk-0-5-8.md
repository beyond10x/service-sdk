---
format: aep.planning-md/1
id: release-plan:service-sdk-0-5-8
kind: release-plan
status: active
title: Release Service SDK 0.5.8
summary: Publish optional projection-field correctness for generated services.
relations:
- delivers: story:optional-projection-fields
- delivers: story:reject-null-optional-projections
- delivers: story:preserve-nested-optional-types
revision: 2
---
## Outcome

Service SDK 0.5.8 is published from the exact bot-authored main commit and downstream generated services can preserve absent optional fields without weakening required fields or accepting explicit null.

## Scope

Version and changelog alignment, the complete repository gate, bot-authored main publication, annotated tag, and immutable GitHub release coordinates.

## Qualification

`task check` passes at the release commit. The tag peels to that commit and the GitHub release exposes the same source coordinate.
