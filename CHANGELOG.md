# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

### Added

- Add a full-screen terminal workspace with account search, tag navigation,
  Markdown reading, external-editor creation and editing, pinning, trash and
  restore, adaptive layouts, and an offline demo with inspectable snapshots.
- Edit notes with `$VISUAL` or `$EDITOR`, with version checks, dry runs,
  and recoverable drafts when saving fails.

### Changed

- Align the terminal workspace with Simplenote's dark charcoal and blue palette,
  including the 256-color terminal fallback.

## [0.1.1](https://github.com/rvben/simplenote-cli/compare/v0.1.0...v0.1.1) - 2026-11-10

### Added

- Distribute the native Rust CLI on PyPI with Linux, macOS, and Windows wheels
  and a source distribution, sharing the Cargo version.
- Verify source builds, isolated wheel installs, and the public PyPI package.
- Include Windows x64 binaries in GitHub releases.

## [0.1.0] - 2026-11-10

### Added

- add scriptable Simplenote CLI
