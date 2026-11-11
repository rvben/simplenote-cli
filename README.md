# simplenote-cli

A Rust command-line client for existing Simplenote accounts. Text on a terminal,
JSON when piped, a discoverable [clispec](https://clispec.dev) v0.3 candidate
contract, and no background daemon. This is an independent, unofficial client.

Live smoke tests cover browser login, listing, pagination, field selection,
fetching, exact text output, history, search, creation, dry-run, updates, stale
edit refusal, trash/restore, and export. Automated tests use isolated fixtures.

## Install

Cargo and PyPI packages install both `simplenote` and `simplenote-cli`. Both
commands run the same native Rust CLI and share credentials and configuration.
Maturin builds
the PyPI package, which has no Python runtime dependencies.

```sh
uv tool install simplenote-cli
# Or: pipx install simplenote-cli
cargo install simplenote-cli --locked
```

PyPI support starts with 0.1.1. For 0.1.0, use Cargo or the
[GitHub binaries](https://github.com/rvben/simplenote-cli/releases).
Wheels cover Linux x64 and ARM64 (glibc 2.28+), macOS x64 and ARM64, and Windows
x64. Installing a source distribution requires Rust 1.90 or newer.

## Build

Requires Rust 1.90 or newer:

```sh
cargo build --release --locked
cargo install --path . --locked
```

## Login

```sh
simplenote auth login
simplenote auth status
```

On a terminal, login prints the browser URL and prompts for the `email:CODE`
credential, showing asterisks as you type or paste. Press `Ctrl+R` to show or hide
the input, Backspace to edit, or `Ctrl+U` to clear it. Press Enter to sign in, or
Esc / `Ctrl+C` / `Ctrl+D` to cancel without redeeming the code. The input line is
cleared before submission or cancellation. The browser supports the sign-in
methods offered by Simplenote. Codes expire after five minutes and are single-use.
The CLI does not request or store your password and does not automatically retry
redemption.

Without a terminal, `auth login` returns a login URL and instructions. Redeem a
code through stdin using `auth login --code-stdin`; avoid putting credentials in
arguments or shell history. Headless use can instead supply `SIMPLENOTE_TOKEN`.
Tokens are never printed. Codes are shown only when you explicitly reveal them
in the interactive terminal; they are never included in structured output or errors.

Credentials are stored in `auth.json` under the platform configuration directory
(`~/Library/Application Support/simplenote-cli` on macOS,
`$XDG_CONFIG_HOME/simplenote-cli` or `~/.config/simplenote-cli` on Linux).
`SIMPLENOTE_CONFIG_DIR` overrides that directory. On Unix, files are written with
mode 0600, and symlinks, special files, and group/world-readable credential files
are rejected. Windows uses platform filesystem permissions; Unix mode guarantees
do not apply there. No credentials are borrowed from another application.

`auth status` checks local availability only; it does not verify a token with the
service. `auth logout` removes this CLI's file; it does not revoke the server's
token, sign out other devices, or clear `SIMPLENOTE_TOKEN`.

## Proxies and certificate trust

All API requests, login-code redemption, and TUI requests use the same proxy
configuration. Set proxy environment variables in the shell that launches the CLI:

```sh
# HTTPS through an HTTP proxy (CONNECT tunneling)
HTTPS_PROXY=http://127.0.0.1:8080 simplenote notes list

# SOCKS5, with destination DNS resolved by the proxy
ALL_PROXY=socks5h://127.0.0.1:1080 simplenote notes list

# Bypass proxies for selected hosts, or use NO_PROXY='*' to bypass all
NO_PROXY=api.simperium.com,app.simplenote.com simplenote notes list
```

`HTTPS_PROXY` applies to HTTPS requests; `HTTP_PROXY` applies only to HTTP, so it
alone does not cover Simplenote's endpoints. `ALL_PROXY` supplies a fallback for
both. Lowercase variants (`https_proxy`, `http_proxy`, `all_proxy`, `no_proxy`)
are also supported; uppercase takes precedence when both are set. `NO_PROXY`
accepts a comma-separated list of hosts, domains, or IP addresses. On macOS and
Windows, manual system HTTP/HTTPS proxy settings are also discovered. Set the
protocol-specific environment variable to override its system setting. Automatic
proxy configuration scripts (PAC/WPAD) are not evaluated.

HTTP/HTTPS and SOCKS proxies are supported. For SOCKS5, `socks5://` resolves
destination names locally; `socks5h://` resolves them through the proxy. Proxy URLs
can include URL-encoded credentials (`scheme://user:password@host:port`). HTTP
proxies support Basic authentication; integrated NTLM/Kerberos authentication is
not provided. Keep proxy credentials out of shell history and shared diagnostics.

TLS verification uses bundled public roots plus certificates from the system
trust store. If a corporate proxy inspects HTTPS, install its CA in that store,
or supply a PEM CA bundle:

```sh
SSL_CERT_FILE=/path/to/company-ca.pem simplenote notes list
```

`SSL_CERT_FILE` and `SSL_CERT_DIR` override the system certificate locations;
bundled public roots remain available. Certificate and hostname verification stay
enabled. Connection errors identify common proxy, authentication, DNS, and TLS
failures without printing raw errors or credentials. Requests are not retried
automatically; after a failed write, check the note before retrying.

## Interactive workspace

```sh
simplenote tui
simplenote tui --demo
simplenote tui --demo --snapshot --width 132 --height 38
```

The full-screen workspace combines a tag rail, note index, and a Markdown
reading pane. Small terminals switch to a single-pane layout; Enter opens a
note and Esc returns to the index. Mouse clicks and the wheel work alongside
the keyboard. Set `NO_COLOR=1` to use your terminal's own colors.

| Key | Action |
| --- | --- |
| `↑` / `↓`, `j` / `k` | Select notes or scroll the focused pane |
| `Tab`, `Enter`, `Esc` | Change focus, read, return |
| `/`, then `Enter` | Search the account; typing previews loaded matches |
| `t`, `1` / `2` / `3` | Tags, all notes, pinned, trash |
| `m`, `r` | Load more, refresh |
| `n`, `e` | Create or edit in `$VISUAL` / `$EDITOR` |
| `p` | Pin or unpin |
| `d`, `u` | Move to Trash with confirmation, restore |
| `?`, `q` | Help, quit |

Network reads and pin/trash/restore writes run in the background. The index
shows pinned notes first, then recently modified notes, within the loaded
results. Pages contain up to 50 notes; `m` continues traversal. Tags come from
loaded notes. Pinned and trash views can have empty pages with more results
available. Search scans account pages using the same bounded pagination as
the CLI; keep loading while a next page is available.

Creating and editing temporarily restore the ordinary terminal for your editor.
An empty new draft cancels creation. Failed saves keep a private recovery draft,
display its path, and print retained paths again when you quit. Writes use the
same version checks and uncertain-write handling as CLI updates; nothing is
retried automatically. New notes start without tags. The TUI keeps notes in
memory only and does not create a persistent offline cache.

Recovery drafts use the operating system's temporary directory. Copy them to
a permanent location if you intend to keep them beyond the current session.
Quitting waits for an in-flight pin, trash, or restore request to finish so its
outcome can be reported. Trash confirmation requires `y`.

`--demo` uses sample notes, disables writes, and never loads credentials or
contacts Simplenote. `--demo --snapshot` prints a plain-text frame from the same
renderer without requiring a terminal. Normal interactive mode requires
terminal stdin, stdout, and stderr; use the CLI commands below in scripts.

## Read and search

```sh
simplenote-cli notes list --limit 20
simplenote-cli notes list --tag work --fields id,title,version
simplenote-cli notes search 'release checklist'
simplenote-cli notes get NOTE_ID
simplenote-cli notes get NOTE_ID --version 42
simplenote-cli notes cat NOTE_ID > note.md
simplenote-cli notes history NOTE_ID --limit 10
```

List and search use server index order, exclude trash by default, and omit note
content from their results. `--include-deleted` includes trash. Search is a
case-insensitive substring match over note content; filtering may scan multiple
server pages. Each invocation scans at most 20 server pages; a selective query
may return fewer than `--limit` matches, including an empty page, with a
`next_cursor`. Continue until that cursor is null. Results are fresh network reads; this version has no offline cache.

Tag matching is case-insensitive. Collections return `items`, `has_more`,
`next_cursor`, and `skipped_irregular` (the count of malformed records omitted
during this request; export preserves those records). Continue with
`--cursor TOKEN` and the same filters. `--limit` is 1–100, default 20. The server's
index is not a transactional snapshot: concurrent edits can affect traversal.
History cursors are version numbers; unavailable versions are marked explicitly.
Timestamps are the service's Unix seconds (numbers or numeric strings).

`notes get` includes the full content and version. When piped, `notes cat`
emits exact UTF-8 content with no added newline, even with `-o json`. On a
terminal, control and direction characters are escaped unless `--raw` is passed.
It is declared as an opaque text command in the schema.

## Write

```sh
simplenote-cli notes create --file note.md --tag work --dry-run
simplenote-cli notes create --file note.md --tag work
printf 'A new note\n' | simplenote-cli notes create --file -

simplenote-cli notes get NOTE_ID
simplenote-cli notes update NOTE_ID --base-version 42 --file edited.md --dry-run
simplenote-cli notes update NOTE_ID --base-version 42 --file edited.md
simplenote-cli notes edit NOTE_ID
simplenote-cli notes edit NOTE_ID --dry-run
simplenote-cli notes trash NOTE_ID --base-version 43 --yes
simplenote-cli notes restore NOTE_ID --base-version 44
```

Content must be nonblank UTF-8 and at most 1 MiB. New notes enable Markdown.
Tags must contain no whitespace, commas, or `@`; case-insensitive duplicates
are removed. Email tags can share notes in Simplenote, so this CLI rejects them.
Updates change content only; other fields are preserved by sending a partial
patch. Trash is recoverable; permanent deletion is not exposed.

`--base-version` is required for edits, trash, and restore. The CLI first reads
the current version and rejects an edit that is already stale. It then sends a
versioned patch to Simperium. **This is not an atomic compare-and-swap:** Simperium
can merge edits made concurrently after that read. The CLI returns the server's
actual resulting content and version. Review the result when concurrent editing
is possible. It never claims the server enforces a strict version precondition.

`--dry-run` performs validation (including a fresh read for edits) without sending
a write. Creates and writes are never automatically retried. Repeating a create
can create a duplicate; its chosen ID is printed to stderr before the request so
you can inspect that ID after an ambiguous failure. Failed writes also include
`error.details.note_id`, `operation`, and `outcome_unknown` in the JSON error.
Check the note after a network error, server error, or invalid write response
before deciding to retry.
A repeated edit with its original base version normally refuses as stale.

`notes edit` fetches the note and its version, opens a private Markdown draft in
the first nonempty `$VISUAL` or `$EDITOR`, and waits for that program to exit.
The default is `vi` on Unix. Windows requires a configured editor that waits
for editing to finish. Editor settings support quoted arguments, for example
`VISUAL='code --wait'` (`code.cmd --wait` on Windows); the CLI does not expand
shell expressions. Use a wait option for GUI editors.
Quote executable paths containing spaces; on Windows, use forward slashes or
single quotes around backslash paths.

Unchanged content is a no-op. Changed content uses the fetched version with the
same preflight check and concurrent-merge limitations as `notes update`.
Failures retain the draft and return `error.details.recovery_file` and
`base_version`; JSON errors preserve any uncertain-write information. Inspect
`recovery_directory` if the editor removed or renamed the draft; that directory
also retains any backup files left by the editor. Inspect
the latest note before retrying with `notes update --file`. A changed dry-run
also retains its draft, as does a successful save whose server result differs
from the submitted text; those results include `recovery_file` and `base_version`.
Successful exact saves and unchanged edits clean up their drafts.

On Unix the draft directory is mode 0700 and the initial file mode 0600. Editor
output goes to stderr, keeping stdout available for a structured result. This
command requires terminal stdin and stderr, declares `requires_tty` in its
clispec contract, and refuses noninteractive use with `tty_required` (exit 12).
Use `notes update --file` in scripts and agents.

## Export

```sh
simplenote-cli export --directory ./simplenote-export
```

Creates a **new** directory and one private JSON file per note, named
`note-<SHA256 of exact note ID>.json`. Hashing avoids case-insensitive filename
collisions and platform-reserved names; the exact ID stays in the JSON.
Includes trash, full content, tags, flags, versions, and timestamps. Every file
also contains `raw`, preserving all service fields, including publication and
sharing metadata and unknown system tags. Irregular legacy records that cannot
be normalized are still exported with `id`, `version`, `normalized: false`, and
`raw`. Historical revisions are not included. There is no bulk import command
in this version.

Existing directories and files are never overwritten. Failed exports retain
partial files for inspection; retry into a different new directory. Export is
paginated internally and is not an atomic account snapshot.

## Output and errors

```sh
simplenote-cli -o json notes list | jq '.items[].id'
simplenote-cli -o text notes list
simplenote-cli schema
simplenote-cli completions zsh
```

`--output auto|json|text` / `-o` is global. Auto uses readable text on a terminal
and JSON otherwise. Terminal output uses compact note cards, clear result headings,
readable note bodies, and dates in UTC. Empty results and pagination include next
steps; previews and retained recovery drafts are clearly marked. Color is limited
to interactive terminals; set `NO_COLOR` or `TERM=dumb` to disable it. Use
`--fields` to choose the metadata shown in lists, search, and history.

Data goes to stdout; diagnostics go to stderr. JSON and exact `notes cat` output
keep their existing formats. In JSON mode,
the last stderr line on failure is an `error` object with `kind`, `message`,
`exit_code`, and `hint`. No telemetry is sent by this client.

| Exit | Kind |
|---|---|
| 1 | internal |
| 2 | usage |
| 3 | auth |
| 4 | not_found |
| 5 | network |
| 6 | rate_limited |
| 7 | conflict |
| 8 | io |
| 9 | invalid_response |
| 10 | remote |
| 11 | confirmation_required |
| 12 | tty_required |

## Development

```sh
make check
cargo test --locked
cargo build --locked
clispec score ./target/debug/simplenote-cli auth status
```

Tests use isolated HTTP fixtures and temporary credential directories; they do
not contact your account or create remote notes. The contract derives arguments
from clap and validates against the vendored v0.3 candidate schema. The candidate
spec can still change.

API behavior follows [Simperium's HTTP reference](https://simperium.com/docs/reference/http/).
The browser code flow and service app ID were verified against
[Automattic's Simplenote MCP implementation](https://github.com/Automattic/simplenote-mcp).
Simplenote's [developer page](https://simplenote.com/developers/) does not offer a
supported third-party integration contract; service compatibility may change.
Publishing and releases require a separate explicit approval and use `vership`.

Release preparation and publishing prerequisites are documented in [docs/releases.md](docs/releases.md).
