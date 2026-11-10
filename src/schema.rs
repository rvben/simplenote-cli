//! clispec v0.3 candidate contract. CLI arguments come from clap's runtime tree.
use crate::{cli::Cli, error::KINDS};
use clap::{ArgAction, CommandFactory};
use serde_json::{Value, json};

pub fn contract() -> Value {
    let cli = Cli::command();
    let mut commands = Vec::new();
    walk(&cli, "", &mut commands);
    json!({
        "clispec": "0.3", "name": "simplenote-cli", "version": env!("CARGO_PKG_VERSION"),
        "description": env!("CARGO_PKG_DESCRIPTION"),
        "output": {"tty": "text", "piped": "json"},
        "global_args": cli.get_arguments().filter(|a| a.is_global_set()).map(arg).collect::<Vec<_>>(),
        "commands": commands,
        "errors": KINDS.iter().map(|(kind, code, retryable)| json!({"kind": kind, "exit_code": code, "retryable": retryable})).collect::<Vec<_>>()
    })
}
fn walk(cmd: &clap::Command, prefix: &str, out: &mut Vec<Value>) {
    for sub in cmd.get_subcommands().filter(|s| s.get_name() != "help") {
        let path = format!("{prefix}{}", sub.get_name());
        if sub.has_subcommands() {
            walk(sub, &format!("{path} "), out);
            continue;
        }
        let mut entry = json!({"name": path, "description": sub.get_about().map(|s| s.to_string()).unwrap_or_default(),
            "effects": "read_only", "output_kind": "data", "cardinality": "single", "stability": "experimental",
            "args": sub.get_arguments().filter(|a| !a.is_global_set() && a.get_id() != "help").map(arg).collect::<Vec<_>>()});
        match path.as_str() {
            "tui" => {
                entry["effects"] = json!("non_idempotent");
                entry["requires_tty"] = json!(true);
                entry["output_kind"] = json!("opaque");
                entry["media_type"] = json!("text/plain; charset=utf-8");
                entry.as_object_mut().unwrap().remove("cardinality");
            }
            "schema" => {
                entry["stdout_schema"] =
                    json!({"type": "object", "required": ["clispec", "commands", "errors"]})
            }
            "completions" | "notes cat" => {
                entry.as_object_mut().unwrap().remove("cardinality");
                entry["output_kind"] = json!("opaque");
                entry["media_type"] = json!(if path == "completions" {
                    "text/x-shellscript"
                } else {
                    "text/plain; charset=utf-8"
                });
            }
            "auth login" => {
                entry["effects"] = json!("non_idempotent");
                entry["stdout_schema"] = json!({"anyOf": [
                    {"type": "object", "required": ["authenticated", "source"], "properties": {"authenticated": {"type": "boolean"}, "source": {"type": "string"}}},
                    {"type": "object", "required": ["login_url", "next_step"], "properties": {"login_url": {"type": "string"}, "next_step": {"type": "string"}}}
                ]});
            }
            "auth status" => {
                entry["output_fields"] = fields(&[
                    ("authenticated", "boolean"),
                    ("source", "string"),
                    ("verified", "boolean"),
                ])
            }
            "auth logout" => {
                entry["effects"] = json!("idempotent");
                entry["output_fields"] = fields(&[
                    ("removed", "boolean"),
                    ("environment_token_present", "boolean"),
                ]);
            }
            "notes list" | "notes search" | "notes history" => {
                entry["cardinality"] = json!("unbounded");
                entry["pagination"] = json!({"style": "cursor", "cursor_field": "next_cursor", "cursor_arg": "--cursor", "limit_arg": "--limit"});
                entry["fields_arg"] = json!("--fields");
                entry["stdout_schema"] = json!({"type": "object", "required": ["items", "next_cursor", "has_more"],
                    "properties": {"items": {"type": "array", "items": {"type": "object"}}, "next_cursor": {"type": ["string", "null"]}, "has_more": {"type": "boolean"}, "skipped_irregular": {"type": "integer", "minimum": 0}, "scan_limited": {"type": "boolean"}}});
            }
            "notes get" => entry["stdout_schema"] = note_schema(),
            "notes create" | "notes update" | "notes edit" | "notes trash" | "notes restore" => {
                // Versioned edits can merge with concurrent clients; no unsafe retry promises.
                entry["effects"] = json!("non_idempotent");
                if path == "notes edit" {
                    entry["requires_tty"] = json!(true);
                }
                entry["stdout_schema"] = json!({"anyOf": [note_schema(),
                    {"type": "object", "required": ["no_op", "note"], "properties": {"no_op": {"const": true}, "note": note_schema()}},
                    {"type": "object", "required": ["dry_run"], "properties": {"dry_run": {"const": true}, "id": {"type": "string"}, "base_version": {"type": "integer"}, "patch": {"type": "object"}, "note": note_schema()}}
                ]});
                if path == "notes trash" {
                    entry["confirmation_bypass_arg"] = json!("--yes");
                }
            }
            "export" => {
                entry["effects"] = json!("non_idempotent");
                entry["output_fields"] = fields(&[
                    ("exported", "integer"),
                    ("directory", "string"),
                    ("includes_deleted", "boolean"),
                ]);
            }
            _ => panic!("Missing command declaration: {path}"),
        }
        out.push(entry);
    }
}
fn arg(a: &clap::Arg) -> Value {
    let name = a
        .get_long()
        .map(|s| format!("--{s}"))
        .unwrap_or_else(|| a.get_id().to_string());
    let id = a.get_id().as_str();
    let kind = if matches!(a.get_action(), ArgAction::SetTrue | ArgAction::SetFalse) {
        "boolean"
    } else if matches!(
        id,
        "limit" | "base_version" | "version" | "width" | "height"
    ) {
        "integer"
    } else if matches!(a.get_action(), ArgAction::Append) {
        "string[]"
    } else {
        "string"
    };
    let mut v = json!({"name": name, "type": kind, "required": a.is_required_set(), "description": a.get_help().map(|s| s.to_string()).unwrap_or_default()});
    if let Some(short) = a.get_short() {
        v["short"] = json!(format!("-{short}"));
    }
    if let Some(values) = a.get_value_parser().possible_values() {
        v["enum"] = json!(
            values
                .filter(|v| !v.is_hide_set())
                .map(|v| v.get_name().to_owned())
                .collect::<Vec<_>>()
        );
    }
    if let Some(default) = a.get_default_values().first() {
        v["default"] = match kind {
            "integer" => json!(default.to_string_lossy().parse::<u64>().unwrap()),
            "boolean" => json!(default == "true"),
            _ => json!(default.to_string_lossy()),
        };
    }
    v
}
fn fields(pairs: &[(&str, &str)]) -> Value {
    json!(
        pairs
            .iter()
            .map(|(name, kind)| json!({"name": name, "type": kind}))
            .collect::<Vec<_>>()
    )
}
pub fn note_schema() -> Value {
    json!({"type": "object", "required": ["id", "version", "title", "content", "tags", "pinned", "markdown", "deleted", "created", "modified"],
    "properties": {
        "id": {"type": "string"}, "version": {"type": "integer", "minimum": 0}, "title": {"type": "string"}, "content": {"type": "string"},
        "tags": {"type": "array", "items": {"type": "string"}}, "pinned": {"type": "boolean"}, "markdown": {"type": "boolean"}, "deleted": {"type": "boolean"},
        "created": {"type": ["number", "string", "null"]}, "modified": {"type": ["number", "string", "null"]}
    }})
}
