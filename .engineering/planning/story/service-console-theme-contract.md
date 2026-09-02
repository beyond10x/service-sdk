---
format: aep.planning-md/1
id: story:service-console-theme-contract
kind: story
status: implemented
title: Make generated consoles themeable
summary: Let generated service consoles inherit documented semantic host tokens while retaining accessible standalone defaults.
relations:
- derived_from: epic:builder-runtime
- serves: vision:composable-services
revision: 4
---
## Outcome

Every generated service console inherits a host product's semantic theme without knowing that product's preference model, while standalone generated documentation remains accessible by default.

## Acceptance

- The Vue console consumes a documented optional `--b10x-*` CSS token contract for canvas, surfaces, text, borders, accent, status, focus, code, overlays, and elevation.
- Every token has a neutral system-aware fallback, so standalone generated documentation needs no host theme dependency.
- The console accepts no Devcenter-specific theme prop, reads no host storage, and branches on no named product theme.
- Host token overrides style operation navigation, forms, lifecycle/views, activity results, focus, and native controls without leaking outside the widget.
- TypeScript, widget, generated-docs, Rust, and AEP gates pass.
