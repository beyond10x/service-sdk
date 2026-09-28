---
format: aep.planning-md/3
id: story:service-catalog-console
kind: story
status: implemented
title: Generate service catalogs and a reusable console
summary: Generate exact catalogs, an external discovery factory, reusable Vue UI, and standalone service docs.
relations:
- derived_from: epic:builder-runtime
- serves: vision:composable-services
revision: 4
transitions:
- {from: "draft", to: "proposed", at: "2026-09-02T12:30:11Z", actor: "human:timo", revision: 2, imported: true}
- {from: "proposed", to: "active", at: "2026-09-02T12:30:11Z", actor: "human:timo", revision: 3, imported: true}
- {from: "active", to: "implemented", at: "2026-09-02T12:32:39Z", actor: "human:timo", revision: 4, decided_on: {"recorded":{"test_result":1}}, imported: true}
---
# Generate service catalogs and a reusable console

## Acceptance

- ESS `ess-browser-catalog/1` remains the exact semantic source and service-sdk emits deterministic `service-catalog/1` operation bindings from the same schemas as the domain Connector manifest.
- The SDK supplies a separate read-only `ServiceCatalogFactory` through the existing external Connector factory and deployment-overlay seam; Connectors has no SDK-specific fields or code.
- A shared Vue widget renders generated operation forms, lifecycle, views and activity, confirms writes, and refreshes prior queries after successful mutations.
- Every generated service contains a standalone docs app using an explicit non-persistent demo binding; composed products inject a session-authenticated live binding.
- Tenant, user, authority, executor and realm remain absent from routes, widget binding arguments and operation inputs; optional realm absence remains distinct from literal `default`.
- Rust, TypeScript, generated-service and AEP gates pass.
