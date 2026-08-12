# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

### Added

- the serve loop answers `server/discover` and `ping`.

### Fixed

- tool schema `examples` were only emitted when the example list was empty;
  the condition was inverted.

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
