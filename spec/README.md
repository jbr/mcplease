# MCP spec reference copies

Verbatim copies of the published Model Context Protocol schemas that
`src/types.rs` mirrors, fetched from
`https://raw.githubusercontent.com/modelcontextprotocol/modelcontextprotocol/main/schema/<revision>/schema.json`.

- `schema-2026-07-28.{json,ts}` — the newest revision the types model
  (stateless; no `initialize` handshake).
- `schema-2025-11-25.json` — the newest revision *with* the `initialize`
  handshake, which the types retain for interop with deployed servers.
- `changelog-2026-07-28.mdx` — what changed between those two revisions.

These are reference documents, excluded from the published crate
(`exclude` in `Cargo.toml`). When updating to a newer revision, fetch its
schema here first and diff.
