use crate::Error;
use reqwest::{
    Method, Url,
    blocking::{Client, Response},
};
use serde_json::{Value, json};
use std::{collections::HashSet, time::Duration};

pub struct Http {
    client: Client,
    api: Url,
    app: Url,
}
pub struct Api<'a> {
    pub http: &'a Http,
    pub token: String,
}
#[derive(Clone, Copy)]
pub enum Collection {
    Active,
    All,
    Pinned,
    Trash,
}

impl Http {
    pub fn new() -> Result<Self, Error> {
        Self::with_urls(
            "https://api.simperium.com/1/chalk-bump-f49/",
            "https://app.simplenote.com/",
        )
    }
    /// Explicit endpoints for isolated protocol tests; the CLI never takes endpoint overrides.
    pub fn with_urls(api: &str, app: &str) -> Result<Self, Error> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("simplenote-cli/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| Error::new("internal", "Cannot initialize HTTP client."))?;
        Ok(Self {
            client,
            api: Url::parse(api).map_err(|_| Error::usage("Invalid API URL."))?,
            app: Url::parse(app).map_err(|_| Error::usage("Invalid app URL."))?,
        })
    }
    pub fn login(&self, body: Value) -> Result<Value, Error> {
        let url = self
            .app
            .join("account/complete-login")
            .map_err(|_| Error::new("internal", "Invalid login path."))?;
        let res = self.client.post(url).json(&body).send().map_err(network)?;
        let res = checked(res, true)?;
        res.json()
            .map_err(|_| invalid("Login returned invalid JSON."))
    }
}
fn network(error: reqwest::Error) -> Error {
    let cause = if error.is_timeout() {
        "Request timed out"
    } else if error.is_connect() {
        "Could not establish a connection"
    } else {
        "Request failed"
    };
    Error::new(
        "network",
        format!("{cause}; no automatic retry was attempted."),
    )
}
fn invalid(message: &str) -> Error {
    Error::new("invalid_response", message)
}
fn checked(res: Response, login: bool) -> Result<Response, Error> {
    let status = res.status().as_u16();
    if (200..300).contains(&status) {
        return Ok(res);
    }
    let kind = match status {
        400 if login => "auth",
        401 | 403 => "auth",
        404 => "not_found",
        409 => "conflict",
        429 => "rate_limited",
        _ => "remote",
    };
    Err(Error::new(kind, format!("Service returned HTTP {status}.")))
}
pub fn validate_id(id: &str) -> Result<(), Error> {
    if id.is_empty()
        || id.len() > 256
        || id == "."
        || id == ".."
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(Error::usage(
            "Note IDs must contain only ASCII letters, digits, hyphens, or underscores (1–256 characters).",
        ));
    }
    Ok(())
}
impl Api<'_> {
    fn request(
        &self,
        method: Method,
        parts: &[&str],
        query: &[(&str, String)],
        body: Option<&Value>,
    ) -> Result<Response, Error> {
        let mut url = self.http.api.clone();
        url.path_segments_mut()
            .map_err(|_| Error::new("internal", "Invalid API base."))?
            .pop_if_empty()
            .extend(parts);
        if !query.is_empty() {
            url.query_pairs_mut()
                .extend_pairs(query.iter().map(|(k, v)| (*k, v.as_str())));
        }
        let mut request = self
            .http
            .client
            .request(method, url)
            .header("X-Simperium-Token", &self.token);
        if let Some(body) = body {
            request = request.json(body);
        }
        request.send().map_err(network)
    }
    pub fn get(&self, id: &str, version: Option<u64>) -> Result<Value, Error> {
        let (raw, v) = self.raw(id, version)?;
        normalize(id, &raw, v)
    }
    fn raw(&self, id: &str, version: Option<u64>) -> Result<(Value, u64), Error> {
        validate_id(id)?;
        let mut parts = vec!["note", "i", id];
        let v = version.map(|v| v.to_string());
        if let Some(ref v) = v {
            parts.extend(["v", v]);
        }
        let res = checked(self.request(Method::GET, &parts, &[], None)?, false)?;
        let v = response_version(&res)?;
        let raw: Value = res
            .json()
            .map_err(|_| invalid("Note returned invalid JSON."))?;
        normalize(id, &raw, v)?;
        Ok((raw, v))
    }
    pub fn list(
        &self,
        limit: usize,
        cursor: Option<&str>,
        tag: Option<&str>,
        query: Option<&str>,
        deleted: bool,
    ) -> Result<Value, Error> {
        self.list_collection(
            limit,
            cursor,
            tag,
            query,
            if deleted {
                Collection::All
            } else {
                Collection::Active
            },
        )
    }
    pub fn list_collection(
        &self,
        limit: usize,
        cursor: Option<&str>,
        tag: Option<&str>,
        query: Option<&str>,
        collection: Collection,
    ) -> Result<Value, Error> {
        let mut items = Vec::new();
        let mut mark = cursor.map(str::to_owned);
        let mut seen = HashSet::new();
        let query = query.map(str::to_lowercase);
        let tag = tag.map(str::to_lowercase);
        let mut skipped_irregular = 0;
        for _ in 0..20 {
            let body = self.index_page(limit - items.len(), mark.as_deref())?;
            let entries = body["index"].as_array().expect("validated index");
            for entry in entries {
                let id = entry["id"]
                    .as_str()
                    .ok_or_else(|| invalid("Index entry has no ID."))?;
                let v = entry["v"]
                    .as_u64()
                    .filter(|v| *v > 0)
                    .ok_or_else(|| invalid("Index entry has no valid version."))?;
                let Ok(note) = normalize(id, &entry["d"], v) else {
                    skipped_irregular += 1;
                    continue;
                };
                let included = match collection {
                    Collection::Active => note["deleted"] != true,
                    Collection::All => true,
                    Collection::Pinned => note["deleted"] != true && note["pinned"] == true,
                    Collection::Trash => note["deleted"] == true,
                };
                if !included {
                    continue;
                }
                if tag.as_ref().is_some_and(|t| {
                    !note["tags"].as_array().is_some_and(|tags| {
                        tags.iter()
                            .any(|v| v.as_str().is_some_and(|s| s.to_lowercase() == *t))
                    })
                }) {
                    continue;
                }
                if query.as_ref().is_some_and(|q| {
                    !note["content"]
                        .as_str()
                        .unwrap_or_default()
                        .to_lowercase()
                        .contains(q)
                }) {
                    continue;
                }
                items.push(note);
            }
            mark = match body.get("mark") {
                None | Some(Value::Null) => None,
                Some(Value::String(s)) if s.is_empty() => None,
                Some(Value::String(s)) => Some(s.clone()),
                _ => return Err(invalid("Index cursor is not a string.")),
            };
            if let Some(ref mark) = mark
                && (cursor == Some(mark.as_str()) || !seen.insert(mark.clone()))
            {
                return Err(invalid("Server repeated an index cursor."));
            }
            if items.len() == limit || mark.is_none() {
                return Ok(
                    json!({"items": items, "next_cursor": mark, "has_more": mark.is_some(), "skipped_irregular": skipped_irregular}),
                );
            }
        }
        Ok(
            json!({"items": items, "next_cursor": mark, "has_more": mark.is_some(), "skipped_irregular": skipped_irregular, "scan_limited": true}),
        )
    }
    /// Fetch one raw page without normalizing away unknown service fields.
    pub fn index_page(&self, limit: usize, cursor: Option<&str>) -> Result<Value, Error> {
        if !(1..=100).contains(&limit) {
            return Err(Error::usage("Page size must be 1–100."));
        }
        let mut params = vec![("data", "true".into()), ("limit", limit.to_string())];
        if let Some(cursor) = cursor {
            params.push(("mark", cursor.to_owned()));
        }
        let res = checked(
            self.request(Method::GET, &["note", "index"], &params, None)?,
            false,
        )?;
        let body: Value = res
            .json()
            .map_err(|_| invalid("Index returned invalid JSON."))?;
        let entries = body["index"]
            .as_array()
            .ok_or_else(|| invalid("Index response is missing its index array."))?;
        if entries.len() > limit {
            return Err(invalid("Server exceeded the requested page size."));
        }
        if body
            .get("mark")
            .is_some_and(|v| !v.is_null() && !v.is_string())
        {
            return Err(invalid("Index cursor is not a string."));
        }
        Ok(body)
    }
    pub fn create(&self, content: &str, tags: &[String], dry_run: bool) -> Result<Value, Error> {
        let tags = validate_tags(tags)?;
        let id = uuid::Uuid::new_v4().to_string();
        let now = now()?;
        let raw = json!({"content": content, "creationDate": now, "modificationDate": now,
            "deleted": false, "tags": tags, "systemTags": ["markdown"], "publishURL": "", "shareURL": ""});
        if dry_run {
            return Ok(
                json!({"dry_run": true, "operation": "create", "note": normalize(&id, &raw, 0)?}),
            );
        }
        eprintln!("Creating note {id}; if the request fails, check this ID before retrying.");
        self.post(&id, None, &raw)
    }
    pub fn change(&self, id: &str, base: u64, patch: Value, dry_run: bool) -> Result<Value, Error> {
        let (raw, current) = self.raw(id, None)?;
        if current != base {
            return Err(Error::new(
                "conflict",
                format!("Expected version {base}, found {current}; nothing was written."),
            ));
        }
        if bool_value(&raw["deleted"]) && patch.get("content").is_some() {
            return Err(Error::usage(
                "Restore the trashed note before updating its content.",
            ));
        }
        if patch
            .as_object()
            .is_some_and(|p| p.iter().all(|(k, v)| raw.get(k) == Some(v)))
        {
            return Ok(json!({"no_op": true, "note": normalize(id, &raw, current)?}));
        }
        let mut patch = patch;
        patch["modificationDate"] = json!(now()?);
        if dry_run {
            return Ok(json!({"dry_run": true, "id": id, "base_version": base, "patch": patch}));
        }
        self.post(id, Some(current), &patch)
    }
    /// Preserve unknown system tags when toggling the pinned flag.
    pub fn pin(&self, id: &str, base: u64, pinned: bool) -> Result<Value, Error> {
        let (raw, current) = self.raw(id, None)?;
        if current != base {
            return Err(Error::new(
                "conflict",
                "This note changed. Refresh before pinning it.",
            ));
        }
        let mut tags = raw["systemTags"].as_array().cloned().unwrap_or_default();
        tags.retain(|tag| tag != "pinned");
        if pinned {
            tags.push(json!("pinned"));
        }
        self.change(id, base, json!({"systemTags":tags}), false)
    }
    fn post(&self, id: &str, base: Option<u64>, patch: &Value) -> Result<Value, Error> {
        validate_id(id)?;
        self.post_inner(id, base, patch)
            .map_err(|e| e.with_write_context(id, if base.is_some() { "update" } else { "create" }))
    }
    fn post_inner(&self, id: &str, base: Option<u64>, patch: &Value) -> Result<Value, Error> {
        let mut parts = vec!["note", "i", id];
        let v = base.map(|v| v.to_string());
        if let Some(ref v) = v {
            parts.extend(["v", v]);
        }
        let res = self.request(
            Method::POST,
            &parts,
            &[
                ("response", "1".into()),
                ("ccid", uuid::Uuid::new_v4().to_string()),
            ],
            Some(patch),
        )?;
        if res.status().as_u16() == 412 {
            return Ok(json!({"no_op": true, "note": self.get(id, None)?}));
        }
        let res = checked(res, false)?;
        let version = response_version(&res)?;
        let raw: Value = res.json().map_err(|_| invalid("Write may have succeeded, but its response was not valid JSON; read the note before retrying."))?;
        normalize(id, &raw, version)
    }
    pub fn history(&self, id: &str, limit: usize, cursor: Option<u64>) -> Result<Value, Error> {
        let (_, current) = self.raw(id, None)?;
        let start = cursor.unwrap_or(current);
        if start > current || start == 0 {
            return Err(Error::usage(
                "History cursor must be an existing positive version.",
            ));
        }
        let end = start.saturating_sub(limit as u64).max(1);
        let mut items = Vec::new();
        let mut next = None;
        for v in (end..=start).rev().take(limit) {
            match self.get(id, Some(v)) {
                Ok(note) => items.push(note),
                Err(e) if e.kind == "not_found" => {
                    items.push(json!({"id": id, "version": v, "available": false}))
                }
                Err(e) => return Err(e),
            }
            if v > 1 {
                next = Some(v - 1);
            } else {
                next = None;
            }
        }
        Ok(
            json!({"items": items, "next_cursor": next.map(|n| n.to_string()), "has_more": next.is_some()}),
        )
    }
}
fn response_version(res: &Response) -> Result<u64, Error> {
    res.headers().get("X-Simperium-Version").and_then(|s| s.to_str().ok()).and_then(|s| s.parse::<u64>().ok()).filter(|v| *v > 0)
        .ok_or_else(|| invalid("Response has no valid X-Simperium-Version; a write may have succeeded. Read the note before retrying."))
}
fn now() -> Result<u64, Error> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| Error::new("internal", "System clock is before the Unix epoch."))
}
fn bool_value(v: &Value) -> bool {
    v == true || v == 1 || v == "1" || v.as_str().is_some_and(|s| s.eq_ignore_ascii_case("true"))
}
pub fn normalize(id: &str, raw: &Value, version: u64) -> Result<Value, Error> {
    validate_id(id).map_err(|_| invalid("Server returned an invalid note ID."))?;
    let content = raw["content"]
        .as_str()
        .ok_or_else(|| invalid("Note has no string content."))?;
    let empty = Vec::new();
    let tags = raw
        .get("tags")
        .map_or(Some(&empty), Value::as_array)
        .ok_or_else(|| invalid("Note has no tags array."))?;
    let system = raw
        .get("systemTags")
        .map_or(Some(&empty), Value::as_array)
        .ok_or_else(|| invalid("Note has no systemTags array."))?;
    if tags.iter().chain(system).any(|t| !t.is_string()) {
        return Err(invalid("Note tags must be strings."));
    }
    for field in ["creationDate", "modificationDate"] {
        if !matches!(
            &raw[field],
            Value::Null | Value::Number(_) | Value::String(_)
        ) {
            return Err(invalid("Note timestamp must be a number, string, or null."));
        }
    }
    let valid_deleted = match &raw["deleted"] {
        Value::Null | Value::Bool(_) => true,
        Value::Number(n) => n.as_u64().is_some_and(|n| n <= 1),
        Value::String(s) => ["0", "1", "true", "false"]
            .iter()
            .any(|v| s.eq_ignore_ascii_case(v)),
        _ => false,
    };
    if !valid_deleted {
        return Err(invalid("Note deletion state is invalid."));
    }
    let title = content
        .lines()
        .find(|s| !s.trim().is_empty())
        .unwrap_or("")
        .trim();
    Ok(
        json!({"id": id, "version": version, "title": title, "content": content, "tags": tags,
        "pinned": system.iter().any(|t| t == "pinned"), "markdown": system.iter().any(|t| t == "markdown"),
        "deleted": bool_value(&raw["deleted"]), "created": raw["creationDate"], "modified": raw["modificationDate"]}),
    )
}

fn validate_tags(tags: &[String]) -> Result<Vec<String>, Error> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for tag in tags {
        if tag.is_empty()
            || tag
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || c == ',' || c == '@')
        {
            return Err(Error::usage(
                "Tags must be nonempty and contain no whitespace, commas, or @. Sharing via email tags is not supported.",
            ));
        }
        if seen.insert(tag.to_lowercase()) {
            result.push(tag.clone());
        }
    }
    Ok(result)
}
