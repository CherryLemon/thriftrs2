# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- JSON protocol round-trip support for top-level structs and RPC messages, including buffered JSON RPC framing.
- Broader IDL parsing for `include`, `namespace`, `typedef`, `enum`, `const`, `exception`, `throws`, `union`, annotations, field defaults, and `struct`/`service extends` inheritance.
- Python loader support for `include_dirs`, enum constants, schema defaults, and qualified include references such as `common.User`.
- Declared RPC exception encoding/decoding for handlers that raise Python exceptions matching a `throws` struct name.
- Read compatibility for thriftpy2 `utils` JSON envelopes (`metadata`/`payload` with a 4-byte length prefix) when deserializing JSON structs.
- Comprehensive benchmark script covering JSON ser/de, RPC protocol/transport/concurrency matrices, thriftpy2 comparisons, multi-run averages, and CI smoke mode.
- Python and Rust regression coverage for parser compatibility, protocol interop, JSON behavior, and RPC matrices.

### Changed

- Optimized synchronous RPC handler dispatch by avoiding per-call coroutine introspection and unnecessary blocking-task handoff.
- Plain dict serialization now writes schema default values for omitted fields, matching `ThriftStruct()` instance behavior.

### Fixed

- Optional fields with `None` are omitted during serialization, while required fields with `None` now raise `TypeError`.
- Lexer keyword handling no longer splits identifiers like `optional_note` or `include_path`.
- Compact protocol nested struct field-id state is reset correctly.
- JSON numeric map keys are restored to their declared numeric types when deserializing.

## [0.1.0] - 2026-04-19

### Added

- Initial public release: Thrift IDL loading, struct serialization (binary, compact, JSON), and RPC client/server helpers backed by Rust/PyO3.

[Unreleased]: https://github.com/CherryLemon/thriftrs2/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/CherryLemon/thriftrs2/releases/tag/v0.1.0
