use crate::{Error, api::Http};
mod prompt;
pub(crate) use prompt::read_credential;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub const LOGIN_URL: &str = "https://app.simplenote.com/login/?source=mcp";
#[derive(Serialize, Deserialize)]
pub struct Credentials {
    pub token: String,
}

pub fn config_dir() -> Result<PathBuf, Error> {
    if let Some(path) = std::env::var_os("SIMPLENOTE_CONFIG_DIR") {
        return Ok(path.into());
    }
    dirs::config_dir()
        .map(|p| p.join("simplenote-cli"))
        .ok_or_else(|| {
            Error::new(
                "io",
                "Cannot determine configuration directory; set SIMPLENOTE_CONFIG_DIR.",
            )
        })
}

pub fn load(dir: &Path) -> Result<String, Error> {
    if let Ok(token) = std::env::var("SIMPLENOTE_TOKEN")
        && !token.trim().is_empty()
    {
        return Ok(token.trim().to_owned());
    }
    read(dir)
}
pub fn read(dir: &Path) -> Result<String, Error> {
    let mut opts = OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = opts
        .open(dir.join("auth.json"))
        .map_err(|_| Error::new("auth", "No readable credentials found."))?;
    if !file.metadata()?.is_file() {
        return Err(Error::new("auth", "Credentials must be a regular file."));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if file.metadata()?.permissions().mode() & 0o077 != 0 {
            return Err(Error::new(
                "auth",
                "Credentials are readable by others; set auth.json permissions to 0600.",
            ));
        }
    }
    let mut raw = String::new();
    Read::by_ref(&mut file)
        .take(65537)
        .read_to_string(&mut raw)?;
    if raw.len() > 65536 {
        return Err(Error::new("auth", "Credentials file exceeds 64 KiB."));
    }
    let credentials: Credentials =
        serde_json::from_str(&raw).map_err(|_| Error::new("auth", "Invalid credentials file."))?;
    if credentials.token.trim().is_empty() {
        return Err(Error::new("auth", "Credentials contain an empty token."));
    }
    Ok(credentials.token)
}
pub fn save(dir: &Path, token: String) -> Result<(), Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700).create(dir)?;
    }
    #[cfg(not(unix))]
    fs::create_dir_all(dir)?;
    let mut temp = tempfile::NamedTempFile::new_in(dir)?;
    serde_json::to_writer(&mut temp, &Credentials { token })
        .map_err(|_| Error::new("io", "Cannot serialize credentials."))?;
    temp.flush()?;
    temp.as_file().sync_all()?;
    temp.persist(dir.join("auth.json"))
        .map_err(|_| Error::new("io", "Cannot save credentials."))?;
    Ok(())
}
pub fn logout(dir: &Path) -> Result<bool, Error> {
    match fs::remove_file(dir.join("auth.json")) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}
pub fn redeem(http: &Http, credential: &str) -> Result<String, Error> {
    let (username, code) = credential_parts(credential)?;
    let body =
        http.login(serde_json::json!({"username": username, "auth_code": code.to_uppercase()}))?;
    ["sync_token", "syncToken", "token"]
        .iter()
        .find_map(|k| {
            body[*k]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        })
        .ok_or_else(|| Error::new("invalid_response", "Login did not return a sync token."))
}

fn credential_parts(credential: &str) -> Result<(&str, &str), Error> {
    let (username, code) = credential
        .trim()
        .rsplit_once(':')
        .ok_or_else(|| Error::usage("Expected the browser credential in email:CODE form."))?;
    if username.trim().is_empty() || code.trim().is_empty() {
        return Err(Error::usage(
            "Expected the browser credential in email:CODE form.",
        ));
    }
    Ok((username.trim(), code.trim()))
}
