# Security policy

Service SDK handles authenticated authority, durable service state, content references, and
external-effect plans. Do not put a suspected vulnerability, credential, token, private source,
production payload, or transcript in a public issue.

## Report privately

Use this repository's **Security** tab and choose **Report a vulnerability**. Include the affected
release or commit, the boundary involved, reproduction steps, and impact. Use synthetic credentials
and fixtures wherever possible.

If GitHub does not offer the private reporting form, open a public issue containing no sensitive
details and ask a maintainer to establish a private channel.

## Supported versions

Service SDK is pre-v1. Security fixes target the current `main` branch and latest tagged release.
Previously released schemas and runtime IR remain immutable compatibility evidence, not supported
runtime branches.

## Public source and secrets

The repository and its history are publicly readable under Apache-2.0. Credentials, key files,
private source, production data, and transcripts must never be committed, including in generated
output, fixtures, or history.
