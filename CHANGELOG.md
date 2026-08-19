# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.4.2] - 2026-08-19

### Fixed

- `headers::validate` requires the standard headers only of a request whose
  `MCP-Protocol-Version` header declares `2026-07-28` or later (revisions are
  ISO dates, so the comparison is textual). The headers are new in that
  revision: a client speaking an earlier one — or declaring no version at all,
  which implies a legacy revision — cannot be expected to send headers its
  protocol does not define, and requiring them unconditionally rejected every
  such client, the published conformance suite among them. The version gates
  requiredness, not agreement: a header that is present must match the body in
  any era. `headers::MCP_PROTOCOL_VERSION` names the header it reads, through
  the same lookup `validate` already takes.
- the `tools!`-generated `Deserialize` no longer requires `params.arguments`
  on `tools/call`. The schema makes it optional and absent means the same as
  `{}`, so a tool that takes no arguments is callable without the field;
  previously such a call was rejected with `-32602: missing field
  'arguments'`.

## [0.4.1] - 2026-08-19

### Added

- the `headers` module, unconditional like `types`: the standard HTTP request
  headers `2026-07-28` requires (`Mcp-Method` on every request, `Mcp-Name` on
  `tools/call`/`resources/read`/`prompts/get`), their base64 sentinel value
  encoding, and `validate` for the server side.

## [0.4.0] - 2026-08-12

### Added

- feature flags, so a consumer can take part of the crate. `types` is
  unconditional; `server` is the transport-agnostic tool-authoring and dispatch
  surface (`traits`, `tools!`, `handle_request`); `stdio` is the stdin/stdout
  serve loop; `cli` is the clap argv path; `client` is the client half of the
  protocol; `session` is `SessionStore`. The default is `["cli", "client",
  "session"]`, which is everything, so an existing consumer is unaffected.
- `handle_request` and `serve` are public. `handle_request` answers a decoded
  `JsonRpcRequest` with a `JsonRpcResponse` and performs no I/O, which is the
  whole surface a transport this crate does not implement — an HTTP endpoint,
  say — needs from it.
- the `client` module, behind the `client` feature: the client half of the
  protocol, split the same way as the server half and performing no I/O either,
  so a transport is responsible for framing alone. `ClientProtocol` allocates
  request ids, builds requests, and classifies each arriving message as a
  `Reaction` — a result to correlate by id, a reply to send (`ping` answered,
  everything else declined as method-not-found), or nothing. `Negotiation` is
  the `server/discover` probe with its documented fallback to the `initialize`
  handshake, as a state machine: `Step::Send` a request, feed the result back,
  `Step::Done` with what was negotiated. There is no transport trait and no
  sync/async split, because what differs between transports is the read loop
  itself, which a transport writes. The feature pulls no dependency the crate
  does not already have unconditionally.
- `RequestContext` can now be produced as well as parsed: `ClientProtocol`
  stamps the `_meta` that `2026-07-28` requires on every request — protocol
  version, client info, capabilities — which is what a server rebuilds with
  `RequestContext::from_params`. Previously only the reading half existed, so
  neither side's handling of the reserved `_meta` keys was checkable; a test
  now drives a client-stamped request through `handle_request` into a tool that
  reads the caller back out.

### Changed

- `tools!` derives clap's `Subcommand` for the generated `Tools` enum only
  under the `cli` feature. A tool struct's own `clap::Args` derive is likewise
  needed only there; `#[cfg_attr(feature = "cli", derive(clap::Args))]` keeps a
  tool usable in both configurations.
- `initialize` reads `protocolVersion` directly from the request params rather
  than by deserializing all of `InitializeRequestParams`. Version negotiation is
  precisely where the two sides have not yet agreed on the message shape, and a
  `clientInfo` that failed to parse previously cost the client its requested
  version and silently returned the fallback.

## [0.3.0] - 2026-08-12


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
