# MCP spec reference copies

Verbatim copies of the published Model Context Protocol specification that
`src/types.rs` mirrors, fetched from
`https://raw.githubusercontent.com/modelcontextprotocol/modelcontextprotocol/main/`.

These are reference documents, excluded from the published crate
(`exclude` in `Cargo.toml`).

## Prose — normative

`prose/specification/2026-07-28/` is the specification proper, the source of
every MUST/SHOULD. **This is the authority.** The JSON Schema is generated from
it and does not carry its requirements: `structuredContent`'s
backwards-compatibility rule, the `ttlMs` freshness semantics, the deterministic
tool ordering SHOULD, and the `requestState` integrity requirements all exist
only here.

`prose/extensions/` holds the extension specs, which live outside the core
schema — currently the overview plus `tasks/`.

Fetched from `docs/specification/<revision>/**.mdx` and `docs/extensions/**.mdx`;
the `prose/` layout drops the leading `docs/`. The generated `schema.mdx`
reference page is intentionally not vendored — it is the schema below, rendered.

## Schemas — generated

- `schema-2026-07-28.{json,ts}` — the newest revision the types model
  (stateless; no `initialize` handshake).
- `schema-2025-11-25.json` — the newest revision *with* the `initialize`
  handshake, which the types retain for interop with deployed servers.

The `.ts` is the more readable of the two: it carries the doc comments.

## Updating

When moving to a newer revision, fetch its schema *and* its prose here first,
and diff both. `prose/specification/<revision>/changelog.mdx` summarizes what
changed from the prior revision.
