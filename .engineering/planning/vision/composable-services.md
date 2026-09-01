---
format: aep.planning-md/1
id: vision:composable-services
kind: vision
status: approved
title: Composable standalone services
summary: Define once, synthesize safe standalone event-sourced services, and compose them without monolith coupling.
revision: 3
---
# Composable standalone services

## Outcome

Humans and agents define service semantics once and obtain independently deployable, event-sourced services that compose into products without coupling domain code to a monolith.

## Measures

- One definition deterministically produces runtime contracts, clients, Connector contributions, and conformance evidence.
- Authentication context is authoritative and realm never becomes an application route coordinate.
- Generated output is reproducible and compiles without ESS compiler internals.
