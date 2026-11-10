# Releases

Vership prepares the version and changelog, commits, tags, and pushes. The
release workflow builds Linux and macOS binaries for x64 and ARM64, publishes
the Rust crate and PyPI package, and creates a GitHub release with archives and
SHA-256 files. Windows x64 also has a wheel and release binary. Maturin uses
`bindings = "bin"` and reads the version from Cargo.toml; Python does not maintain
a second implementation or independent version.

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

## PyPI publication

Configure a PyPI Trusted Publisher (a pending publisher for the first upload):

- Project: `simplenote-cli`
- GitHub owner: `rvben`
- Repository: `simplenote-cli`
- Workflow: `release.yml`
- GitHub environment: `pypi`

The `pypi` job exchanges GitHub OIDC credentials for a short-lived PyPI token.
For bootstrap publication, it also supports an encrypted `PYPI_API_TOKEN`
repository secret, matching the other Rust CLI release workflows. The token
must permit creating this project. Configure Trusted Publishing through the
owner account and remove the bootstrap secret afterward. PyPI never reuses the
crates.io credential. Missing publication credentials fail the job rather than
silently omit PyPI from a release.

Release builds produce wheels for Linux x64 and ARM64 (manylinux 2.28), macOS
x64 and ARM64, and Windows x64, plus an sdist. Each wheel's metadata is checked;
native wheels are installed without an index into isolated environments and
smoke tested without credentials. CI also builds a wheel from the source
distribution and checks the installed executable.

## Prepare

```sh
vership status
vership config show
vership bump --help
vership bump patch --dry-run
vership bump patch --prepare
```

Use a patch bump for 0.1.1; later releases use the appropriate bump level.
Preparation runs formatting, Clippy, and tests, updates the version and changelog,
and creates a local release commit. It neither tags nor pushes. Do not bump
again after preparation.

Validate the exact prepared version:

```sh
make check
make conformance
actionlint
make package-check
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
Wait for every build, both registry jobs, and the GitHub release job to finish, then:

```sh
vership verify 0.1.1 --targets tag,release,crates,pypi
```

Use the prepared version in place of 0.1.1 for later releases. Verify origin/main
matches the tested release commit and reconcile the primary checkout before
reporting completion.

## Recovery

Inspect local and remote tags and published assets before retrying. Preview
recovery with `vership release --dry-run`. A brief tag may be deleted and recreated
only if no release or artifact was published anywhere. Once anything was
published, preserve the tag and use a patch release for content corrections.
A failed publication job may be retried for the same unchanged content. A manual
workflow can skip an already published crate; it must run from the existing
version tag when `dry_run` is false. A branch-based manual run is dry-run only. `skip_pypi` is available when the
unchanged PyPI version was already published. Never replace the published 0.1.0
tag to add packaging; prepare 0.1.1 with `vership bump patch --prepare`.

Prefer rerunning failed jobs so the original build artifacts are reused. PyPI
uploads skip existing filenames when completing a partial upload; they never
replace published files. Use `skip_github_release` for a full recovery dispatch
after GitHub publication. The release job refuses to overwrite existing assets.
