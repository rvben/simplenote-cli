# Releases

Vership prepares the version and changelog, commits, tags, and pushes. The
release workflow builds Linux and macOS binaries for x64 and ARM64, publishes
the Rust crate, and creates a GitHub release with archives and SHA-256 files.
Windows is tested in CI; prebuilt Windows binaries are not part of this release.

## First publication prerequisites

The repository must exist at `rvben/simplenote-cli`. Keep pull-request and
required-check protections enabled for bots and external contributors; use
`All checks passed` as the aggregate CI check. This project has no Homebrew
publication configured yet.

The first crates.io publication requires a short-lived, appropriately scoped
`CARGO_REGISTRY_TOKEN` repository secret, because Trusted Publishing cannot
bootstrap an unpublished crate name. After the first publication, configure
Trusted Publishing for `rvben/simplenote-cli`, workflow `release.yml`, and remove
the bootstrap secret. Subsequent workflow runs use GitHub OIDC automatically.
Never place a publishing token in tracked files or command-line arguments.

Repository creation, changing secrets or registry settings, pushing, tagging,
and publishing all require explicit user approval. Prepare everything locally
before requesting that approval.

## Prepare

```sh
vership status
vership config show
vership bump --help
vership bump minor --dry-run
vership bump minor --prepare
```

The initial source commit starts at 0.0.0; the initial minor preparation creates
0.1.0. Later releases use the appropriate patch, minor, or major level.
Preparation runs formatting, Clippy, and tests, updates the version and changelog,
and creates a local release commit. It neither tags nor pushes. Do not bump
again after preparation.

Validate the exact prepared version:

```sh
make check
make conformance
actionlint
cargo publish --dry-run --locked
vership release --dry-run
```

The score uses `auth status`, an offline probe; protocol and output correctness
are additionally covered by fixture tests. Live smoke tests use a disposable
note, check create/read/update, stale-edit refusal, trash/restore, and leave the
note in trash. They require separate explicit authorization for account writes.
Tests and CI do not borrow local credentials or run live account mutations.

## Publish after approval

Once repository and first-publication authentication are ready:

```sh
vership release
```

This tags and pushes the prepared version. The tag starts the release workflow.
Wait for every build, the crates job, and the GitHub release job to finish, then:

```sh
vership verify 0.1.0 --targets tag,release,crates
```

Use the prepared version in place of 0.1.0 for later releases. Verify origin/main
matches the tested release commit and reconcile the primary checkout before
reporting completion.

## Recovery

Inspect local and remote tags and published assets before retrying. Preview
recovery with `vership release --dry-run`. A brief tag may be deleted and recreated
only if no release or artifact was published anywhere. Once anything was
published, preserve the tag and use a patch release for content corrections.
A failed publication job may be retried for the same unchanged content. A manual
workflow can skip an already published crate; it must run from the existing
version tag when `dry_run` is false. A branch-based manual run is dry-run only.
