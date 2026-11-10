use serde_json::{Value, json};

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct Error {
    pub kind: &'static str,
    pub message: String,
    pub details: Option<Value>,
}

pub const KINDS: &[(&str, u8, bool)] = &[
    ("internal", 1, false),
    ("usage", 2, false),
    ("auth", 3, false),
    ("not_found", 4, false),
    ("network", 5, false),
    ("rate_limited", 6, true),
    ("conflict", 7, false),
    ("io", 8, false),
    ("invalid_response", 9, false),
    ("remote", 10, false),
    ("confirmation_required", 11, false),
    ("tty_required", 12, false),
];

impl Error {
    pub fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            details: None,
        }
    }
    pub fn with_write_context(mut self, id: &str, operation: &str) -> Self {
        let uncertain = matches!(self.kind, "network" | "remote" | "invalid_response");
        self.details =
            Some(json!({"note_id": id, "operation": operation, "outcome_unknown": uncertain}));
        self.message.push_str(&format!(" Note ID: {id}."));
        self
    }
    pub fn usage(message: impl Into<String>) -> Self {
        Self::new("usage", message)
    }
    pub fn exit_code(&self) -> u8 {
        KINDS.iter().find(|k| k.0 == self.kind).map_or(1, |k| k.1)
    }
    pub fn envelope(&self) -> Value {
        let hint = if self
            .details
            .as_ref()
            .is_some_and(|v| v["outcome_unknown"] == true)
        {
            "The write may have succeeded. Get the note using error.details.note_id and review it before retrying."
        } else if self
            .details
            .as_ref()
            .is_some_and(|v| v.get("recovery_file").is_some())
        {
            "The draft is in error.details.recovery_file. Read the latest note and review your draft before saving with notes update --file."
        } else {
            match self.kind {
                "auth" => "Run simplenote-cli auth login, or set SIMPLENOTE_TOKEN.",
                "network" => {
                    "The outcome of a write may be unknown. Read the note before retrying; creates print their ID to stderr before sending."
                }
                "rate_limited" => "Wait before retrying. Do not repeatedly redeem a login code.",
                "conflict" => "Read the latest note and review your edit before trying again.",
                "confirmation_required" => "Review the command and pass --yes to confirm.",
                "tty_required" => {
                    "Run interactive commands in a terminal. Use notes list, notes get, or notes update --file for scripts."
                }
                _ => "Run simplenote-cli --help for usage.",
            }
        };
        let mut result = json!({"error": {"kind": self.kind, "message": self.message, "exit_code": self.exit_code(), "hint": hint}});
        if let Some(details) = &self.details {
            result["error"]["details"] = details.clone();
        }
        result
    }
}
impl From<std::io::Error> for Error {
    fn from(_: std::io::Error) -> Self {
        Self::new(
            "io",
            "Local file operation failed; check paths and permissions.",
        )
    }
}
