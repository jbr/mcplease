# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0](https://github.com/jbr/mcplease/compare/mcplease-v0.2.3...mcplease-v0.3.0) - 2026-08-12

### Added

- [**breaking**] adopt further affordances from 2026-07-28
- [**breaking**] rewrite protocol types against the MCP 2026-07-28 schema revision

### Other

- use trusted-publishers
- run fmt on nightly

### Changed

- **Breaking:** protocol types rewritten against the MCP `2026-07-28` schema
  revision (reference copies in `spec/`), keeping the `initialize` handshake
  types from `2025-11-25` for interop with deployed servers. `ToolSchema` is
  now `Tool` with a raw JSON `input_schema` (the lossy `InputSchema` enum is
  gone), `Info` is `Implementation`, and the JSON-RPC envelope is
  `JsonRpcRequest`/`JsonRpcNotification`/`JsonRpcResponse`/`JsonRpcMessage`.
  New types cover content blocks, capabilities, `server/discover`, and
  cacheable-result fields.
- `initialize` negotiates the protocol version (echo a supported requested
  version, otherwise answer with the newest handshake revision) instead of
  always claiming `2024-11-05`.
- a tool execution failure is now reported as a `CallToolResult` with
  `isError: true` rather than a JSON-RPC protocol error, per spec.
- **Breaking:** `Tool::execute` gained an associated `Output` type and a
  `&RequestContext` parameter: `fn execute(self, state: &mut State, context:
  &RequestContext) -> Result<Self::Output>`. Existing tools add
  `type Output = String;` and an ignored context parameter.
- **Breaking:** `WithExamples` is now `ToolMeta`, which also carries a tool's
  annotations, title, and icons. The old name remains as a deprecated alias.
- **Breaking:** `run` takes a `ServerConfig` in place of the `server_info` and
  `instructions` arguments.
- **Breaking:** `result_type` fields are a `ResultType` enum rather than
  `Option<String>`. Unrecognized values from a future revision are preserved
  rather than failing to parse.
- `tools/list` and `server/discover` advertise an hour-long `ttlMs` instead of
  `0`, since the tool list is fixed at compile time; both are configurable on
  `ServerConfig`. `cacheScope` stays `private` by default.
- every result carries `io.modelcontextprotocol/serverInfo` in `_meta`, not
  just `server/discover`. A client can use its `version` to invalidate a cached
  tool list when the server binary changes.

### Added

- the serve loop answers `server/discover` and `ping`.
- structured tool output: a tool whose `Output` implements `ToolOutput` sends
  both model-facing content and a machine-readable `structuredContent`, and
  advertises its JSON Schema as the tool's `outputSchema`. The
  `structured_output!` macro writes the impl using the spec's recommended
  lossless shape; implement it by hand to send the model a summary instead.
  `String` and `Vec<ContentBlock>` outputs are supported directly, the latter
  for images, audio, embedded resources, and audience-annotated blocks.
- `ToolMeta::annotations`, `title`, and `icons` reach a tool's advertised
  definition. The protocol's annotation defaults are pessimistic — an
  undeclared tool is presumed destructive and open-world — so declaring
  `read_only_hint` and friends is worthwhile.
- `RequestContext` exposes the calling client's protocol version, identity, and
  per-request capabilities to a tool.
- `InputRequiredResult` and `ToolCallOutcome` model the multi round-trip
  request pattern from the client side. The serve loop never produces an
  interim result, but a client must discriminate on `resultType`: a server may
  return `input_required` carrying only `requestState` (load shedding) without
  the client having declared any capability, and reading that as an ordinary
  result would report a successful empty tool call.
- `meta_keys` names the `_meta` keys the specification reserves.

### Fixed

- tool schema `examples` were only emitted when the example list was empty;
  the condition was inverted.
- the CLI did not compile against `syn` 3.0, and its generated-project test
  silently checked against the published crate rather than the local one, so
  drift between the generator and the library went undetected.

## [0.2.3](https://github.com/jbr/mcplease/compare/mcplease-v0.2.2...mcplease-v0.2.3) - 2025-07-18

### Other

- tweaks to cli interface, code tidying

## [0.2.1](https://github.com/jbr/mcplease/compare/mcplease-v0.2.0...mcplease-v0.2.1) - 2025-07-18

### Added

- add mcplease-cli

### Other

- Merge pull request #4 from jbr/cli

## [0.2.0](https://github.com/jbr/mcplease/compare/v0.1.0...v0.2.0) - 2025-07-12

### Added

- print to stderr
- require Debug from tools explicitly
- improve clap and debugging
- add cli interface
- sessions are reloaded when needed

### Fixed

- input schema parsing

### Other

- don't run coverage currently

## [0.1.0](https://github.com/jbr/mcplease/releases/tag/v0.1.0) - 2025-06-29

### Added

- initial commit
