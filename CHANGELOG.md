# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.5.0] - 2026-10-22

### Changed

- **Dispatch is async and concurrent.** `Tool::execute` and `Dispatch::call` return `impl
  Future<Output = ...> + Send` and take `State` by value; `handle_request` and `answer_tool_call`
  are `async fn`. `State` is now `Clone + Send + Sync + 'static`, so a server holds shared handles —
  a pool, a client, a channel sender — and clones one per call rather than lending `&mut`.

  This is what `2026-07-28` asks for. It removed protocol-level sessions and made every request
  self-describing; it requires request ids and progress tokens to be unique across *active*
  requests; cancellation exists to stop one call while others continue; and `subscriptions/listen`
  is a request that never returns, which a server running requests one at a time to completion would
  deadlock on. `Clone` rather than `Arc<State>` because the handles people actually hold are already
  internally shared.

  `+ Send` is declared on the returned futures rather than left to inference: an opaque future with
  no declared `Send` cannot be proven `Send` by a generic caller, only by a concrete one, so a
  transport handing calls to a runtime could never do it. The cost is deliberate — a state that is
  not `Send` no longer serves.

  **The binary brings the runtime.** Nothing in this crate names or starts one. The stdio transport
  is `calls` (below), over async I/O; the binary spawns each call it yields onto whatever executor
  it runs. Tools share that executor, so a tool with a synchronous stretch moves it to a blocking
  pool (`smol::unblock`, `tokio::task::spawn_blocking`).

- **The stdio transport is async on both ends, and the drivers are gone.**  `calls(reader, writer,
  state, &config)` takes any `futures-lite` `AsyncRead` and `AsyncWrite` — smol's `Unblock` over
  stdin and stdout, or tokio's through `async-compat` — and returns `Calls`, a `Stream<Item =
  anyhow::Result<Call>>`. Polling it *is* the transport: it reads, answers every request that needs
  no tool state, records cancellations, admits calls, and drives the one writer. The whole driver is

  ```rust
  while let Some(call) = calls.next().await {
      executor.spawn(call?).detach();
  }
  ```

  The stream ends only once the input has ended *and* every call it handed out has been answered and
  written, so a client that half-closes still gets every response, and there is no separate drain
  step to forget. A failed write is yielded once as `Err`; dropping `Calls` cancels the calls still
  running.  `Call` resolves to `()`.

  There is no reader thread any more, so nothing outlives `Calls`. Output is queued, unbounded
  because a `Notifier` may not wait, and backpressure is applied at the input: while the output is
  backed up, no more lines are read.  `calls` is generic only in `Tools` and `State`, so a caller
  writes `calls::<Tools, _>(..)`.

- **A tool that panics is answered** with an internal error (`-32603`) rather than left for the
  client to time out, however the call is run.

- **`cli` parses and nothing more.** `Cli<Tools>` is a clap `Parser`: `Cli::Serve` or
  `Cli::Tool(tools)`, with `serve` now a real subcommand that appears in `--help`. The binary runs
  whichever it gets. Logging setup is no longer the library's: `MCP_LOG_LOCATION` is gone, and
  generated servers call `env_logger::init()`, which logs to stderr — the one place the stdio
  transport lets a server write freely.

- **`tools!` no longer follows the `cli` feature.** It derives clap's `Subcommand` for `Tools`
  unless called as `tools!(cli = false, State, ...)`.  Previously the derive appeared whenever
  mcplease was built with `cli`, and cargo builds one mcplease per build with the union of every
  dependent's features — so a server with no command line broke as soon as another crate in the same
  build asked for `cli`. The feature now only decides whether the derive is *available*; without it,
  the default form is a compile error naming the fix.

- **`mcplease create --runtime smol|tokio`** (default `smol`) writes a `main` for that runtime.

- `ServerConfig::with_tool_queue_depth` / `tool_queue_depth` and `DEFAULT_TOOL_QUEUE_DEPTH` become
  `with_max_concurrent_calls` / `max_concurrent_calls` and `DEFAULT_MAX_CONCURRENT_CALLS` (still
  16). The number now bounds calls *running* rather than calls *waiting*; a call past it is still
  refused immediately with `JsonRpcError::server_busy`. `1` restores single-file execution for a
  server whose tools are not yet safe to run alongside each other.

- `RequestContext` is a capability handle rather than a plain struct. Its fields are private with
  accessors, it is built with `from_request` (which is the only way to get the request id) plus
  `with_notifier` / `with_cancellation`, and it carries the progress token, the notification sink,
  and a `swansong::Swansong` the transport shuts down on cancellation. `Debug` is hand-written;
  `Default` still gives the command-line path a context with no client, no sink, and nothing to
  cancel.

- `answer_tool_call` and `handle_request` return `Option<JsonRpcResponse>`.  `None` means the call
  was cancelled, and the spec directs a server not to answer a request the client withdrew. The
  stdio contract is correspondingly "every accepted call is answered **or cancelled** before `Calls`
  ends".

- `client::Reaction` is `#[non_exhaustive]`, since the protocol keeps adding server-initiated
  traffic.

### Added

- **`tools!(…, modules = false, …)`** declares no module per tool and re-exports nothing: tools are
  written `(Type, "name")` and must be in scope, so one tool type can be listed in several sets.

- **Progress.** `RequestContext::progress(progress, total, message)` emits `notifications/progress`
  with the caller's `progressToken`, and is a no-op when the caller sent none or the transport has
  no sink — the spec's own fallback, and why there is no server capability to declare. It returns
  `Result<(), Cancelled>`, so a tool that reports progress gets cancellation checks at the points it
  already has and `?` does the right thing. Rate limiting is the tool's business; this crate does
  not coalesce or throttle.

- **Cancellation.** The stdio reader now acts on `notifications/cancelled` instead of ignoring
  it. Each call runs inside `Swansong::interrupt` on its own child swansong, so cancelling drops the
  call's future at its next await point. Code with no await in it sees the cancellation through
  `RequestContext::is_cancelled`, `progress` returning `Err(Cancelled)`, or the
  `RequestContext::cancelled` future; a tool that never looks and never awaits runs to completion
  and has its result discarded. Cancelling an unknown or already-finished id is ignored, as the spec
  directs. On streamable HTTP a closed response stream is the cancellation signal instead, which is
  why the swansong is supplied by the transport rather than this crate's core. It is part of the
  public interface on purpose: a host can pass a child of its own shutdown swansong, and shutting
  that down cancels every call under it.

- A `tools/call` reusing the id of a call still running is refused with `invalid request`
  (`-32600`); the spec forbids reusing an id before its response. A call reusing the id of one
  already *cancelled* is admitted and is itself cancellable — the old call's cleanup no longer
  removes the new call's registration.

- A failed write to stdout now cancels every running call, since none of their answers can be
  delivered, and `Calls` yields `Err` whether the reader or a call hit it. Previously a failure seen
  by the reader ended `serve` with `Ok(())`.

- `types::Notifier`: where a server-initiated message goes. Object-safe, sync, and infallible,
  taking a whole `JsonRpcNotification` — so the same sink carries progress today and
  `notifications/subscriptions/*` later. Sync because the obvious implementations push onto a queue
  one writer drains, which is not an await point, so a tool never `.await`s telemetry.

- `types::ProgressToken` (`string | integer`) and `types::Cancelled`.  `meta_keys::PROGRESS_TOKEN`,
  which unlike the `io.modelcontextprotocol/*` keys is spelled bare.

- `RequestContext::request_id`, `progress_token`, and `notify`. Cancellation names the id, and a
  future `subscriptions/listen` id *is* the subscription id.

- Client-side progress and cancellation: `ClientProtocol::request_with_progress` and
  `tools_call_with_progress` stamp a token, `cancel` builds a `notifications/cancelled`, and
  `Reaction` gains `Progress` and `SubscriptionEnded`. The token *is* the request id — the spec
  requires only uniqueness across active requests, which is the id's rule in different words, so a
  separate namespace would buy nothing but a token-to-id table to evict.

- `JsonRpcError::invalid_request`. `mcplease::swansong` re-exports the crate.

- **Notifications tied to no request.** `Calls::notifier()` returns a `ConnectionNotifier`, and
  every call's `RequestContext::connection_notifier()` returns the same sink, which a tool may keep
  past its call. Revisions through `2025-11-25` allow a server to speak unprompted once the
  handshake is done, and `2026-07-28` does not. So it delivers only after an `initialize` answered
  with a handshake revision and the client's `notifications/initialized`, and otherwise drops what
  it is given rather than buffering it.

- `ServerConfig::with_experimental_capability(name, value)` advertises an entry under
  `capabilities.experimental` in both `initialize` and `server/discover`.

- `claude_channel`: Claude Code's channels, a vendor extension that pushes events into a running
  session. It provides `CAPABILITY`, `METHOD`, and `ChannelEvent`, which drops attribute keys Claude
  Code would drop silently (anything but letters, digits and `_`) and logs a warning on this
  side. The contract is vendored under `spec/claude-code/`.

### Removed

- **`SessionStore` and the `session` feature.** It was the only I/O in a crate whose claim is that
  it does none, and the only stateful thing in a crate tracking a revision that deleted
  sessions. Nothing else in the crate depended on it. A server that still wants it can vendor the
  module verbatim — it was 286 self-contained lines — and pick its own filesystem story; the shape
  that survives a concurrent, session-free server is a handle in state plus a token the client
  carries. This drops the `notify` dependency entirely.

- `run`, `serve`, and `serve_with`, and the `pollster`, `dirs`, `shellexpand`, and `env_logger`
  dependencies, with the `mcplease::dirs` and `mcplease::shellexpand` re-exports. `run` drove calls
  without a runtime, which only works for tools that never really await — a promise the async `Tool`
  signature does not make. `serve` and `serve_with` polled every call on one task, where one
  blocking tool stalls the rest, which the protocol's concurrency rules do not allow. Both were
  easier to reach than the right answer, which is to spawn.

- The guarantee that a non-`Send` `State` can serve. Declaring `Send` on the dispatch futures is
  what makes them usable by a generic caller, and that is the trade.

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
