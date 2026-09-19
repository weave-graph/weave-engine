# Security boundary

The current engine is a trusted local reference implementation, not a hardened network service. Anyone controlling the CLI, SQLite database or embedding `HostContext` controls authority. `readers` is a basic principal allowlist; it is not a signed capability or revocation system. Graph writers can publish graph content and must be trusted to set classification correctly.

Plans do not contain host authority. Queries filter edge endpoints, edge readers and transitive derivation sources. Metadata resolution uses generic unavailable diagnostics and bounded traversal. The initial snapshot/revision API can reveal revision identity and graph existence to a local caller; it does not implement topology-wide noninterference. No remote endpoint should expose this API without completing the authorization design.

The audit adapter writes local logical effects only. It does not promise exactly-once external effects, sandbox arbitrary code or implement full adapter lifecycle isolation.

Report vulnerabilities without posting sensitive data. Until a private reporting channel is configured on the public repository, use a minimal public issue requesting a private reporting channel and omit exploit details or secrets.
