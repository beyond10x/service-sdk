---
format: aep.planning-md/1
id: release-plan:service-sdk-0-5-8
kind: release-plan
status: implemented
title: Release Service SDK 0.5.8
summary: Publish optional projection-field correctness for generated services.
relations:
- delivers: story:optional-projection-fields
- delivers: story:reject-null-optional-projections
- delivers: story:preserve-nested-optional-types
revision: 4
---
## Outcome

Service SDK 0.5.8 is published from the exact bot-authored main commit and downstream generated services can preserve absent optional fields without weakening required fields or accepting explicit null.

## Scope

Version and changelog alignment, the complete repository gate, bot-authored main publication, annotated tag, and immutable GitHub release coordinates.

## Qualification

`task check` passes at the release commit. The tag peels to that commit and the GitHub release exposes the same source coordinate.

## Release evidence

The complete repository gate passed at bot-authored commit `f57255a2886cae3ace2a3a35935e8f1fd91a5fd4`. Annotated tag `0.5.8` peels to that commit and the immutable GitHub release is https://github.com/beyond10x/service-sdk/releases/tag/0.5.8.
