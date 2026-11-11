# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

## [0.2.2](https://github.com/rvben/simplenote-cli/compare/v0.2.1...v0.2.2) - 2026-11-11

### Fixed

- Enable SOCKS proxies and macOS/Windows system HTTP/HTTPS proxy discovery for
  API, login, and TUI requests. Trust system and custom PEM CAs alongside bundled
  public roots, and explain common proxy and TLS failures without leaking secrets.

## [0.2.1](https://github.com/rvben/simplenote-cli/compare/v0.2.0...v0.2.1) - 2026-11-10

### Added

- Install `simplenote` as a shorter command alongside `simplenote-cli` with
  Cargo, PyPI, and GitHub binaries; both commands share configuration and credentials.

## [0.2.0](https://github.com/rvben/simplenote-cli/compare/v0.1.1...v0.2.0) - 2026-11-10

### Added

- Add a full-screen terminal workspace with account search, tag navigation,
  Markdown reading, external-editor creation and editing, pinning, trash and
  restore, adaptive layouts, and an offline demo with inspectable snapshots.
- Edit notes with `$VISUAL` or `$EDITOR`, with version checks, dry runs,
  and recoverable drafts when saving fails.

### Changed

- Align the terminal workspace with Simplenote's dark charcoal and blue palette,
  including the 256-color terminal fallback.

### Fixed

- **tui**: redraw reliably after external editing ([5f96d09](https://github.com/rvben/simplenote-cli/commit/5f96d0966154871327686e55aa42de73b7b3249b))

## [0.1.1](https://github.com/rvben/simplenote-cli/compare/v0.1.0...v0.1.1) - 2026-11-10

### Added

- Distribute the native Rust CLI on PyPI with Linux, macOS, and Windows wheels
  and a source distribution, sharing the Cargo version.
- Verify source builds, isolated wheel installs, and the public PyPI package.
- Include Windows x64 binaries in GitHub releases.

## [0.1.0] - 2026-11-10

### Added

- add scriptable Simplenote CLI
