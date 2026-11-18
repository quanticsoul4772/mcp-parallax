# Security Policy

## Reporting a vulnerability

Use GitHub's private vulnerability reporting on this repository
(Security → Report a vulnerability). Reports go directly to the maintainer;
please do not open a public issue for anything exploitable.

## Scope notes for reviewers

Parallax is an MCP stdio server that runs inside other people's agent
sessions, so its security posture is part of its design:

- **Capabilities are off by default.** Network egress (memory embeddings,
  research search/fetch) and filesystem reads (`grounded_verify`) exist only
  when their env vars are set; absent, the tools are not in the catalog.
- **SSRF guard**: research fetches refuse private, loopback, link-local, and
  metadata addresses unless `FETCH_ALLOW_PRIVATE` is explicitly set.
- **Root confinement**: `grounded_verify` resolves locators only inside
  `GROUNDED_VERIFY_ROOT`, confined at startup.
- **Secret-named files**: `grounded_verify` refuses files whose names mark them
  as likely secrets (`.env`, private keys, credential files, anything under
  `.ssh`). An exact path to one fails the call; a glob leaves them out and says
  so. The server will not start with its root inside such a folder. This
  guards against accident and misdirection; it is not a boundary, and a secret
  in an ordinarily named file is still read.
- **No `unsafe`**, no `unwrap`/`expect` on production paths
  (compiler/clippy-enforced), stdout reserved for the JSON-RPC channel.
- **Dependency gate**: a weekly `cargo audit` workflow fails on known
  advisories; Dependabot keeps the lockfile current.

## Supported versions

Pre-1.0: only the tip of `main` is supported; fixes land there.
