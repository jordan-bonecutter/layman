use anyhow::{Context, Result, ensure};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct Patch {
    pub path: PathBuf,
    pub before: String,
    pub after: String,
}
impl Patch {
    pub fn check(path: &Path, include: &Path) -> Result<Self> {
        let path = fs::canonicalize(path).context("Cannot locate the running Sway config")?;
        let before = fs::read_to_string(&path).context("Cannot read Sway config")?;
        let line = format!(
            "include {}",
            crate::model::quote(include.to_str().context("Non-UTF-8 include path")?)?
        );
        let matches = |s: &str| {
            let Some(value) = s
                .trim()
                .strip_prefix("include")
                .filter(|v| v.starts_with(char::is_whitespace))
            else {
                return false;
            };
            let value = value.trim();
            let (value, tail) = if value.starts_with(['\"', '\'']) {
                let quote = value.as_bytes()[0] as char;
                let Some(end) = value[1..].find(quote).map(|i| i + 1) else {
                    return false;
                };
                (&value[1..end], &value[end + 1..])
            } else {
                let end = value.find(char::is_whitespace).unwrap_or(value.len());
                (&value[..end], &value[end..])
            };
            if !tail.trim().is_empty() && !tail.trim().starts_with('#') {
                return false;
            }
            let expanded = if let Some(rest) = value
                .strip_prefix("~/")
                .or_else(|| value.strip_prefix("$HOME/"))
                .or_else(|| value.strip_prefix("${HOME}/"))
            {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(rest))
            } else if let Some(rest) = value
                .strip_prefix("$XDG_CONFIG_HOME/")
                .or_else(|| value.strip_prefix("${XDG_CONFIG_HOME}/"))
            {
                std::env::var_os("XDG_CONFIG_HOME").map(|root| PathBuf::from(root).join(rest))
            } else {
                let candidate = PathBuf::from(value);
                Some(if candidate.is_absolute() {
                    candidate
                } else {
                    path.parent().unwrap().join(candidate)
                })
            };
            expanded.is_some_and(|p| {
                fs::canonicalize(&p).unwrap_or(p)
                    == fs::canonicalize(include).unwrap_or_else(|_| include.to_path_buf())
            })
        };
        // Existing includes are user configuration: never remove or relocate them.
        let after = if before.lines().any(matches) {
            before.clone()
        } else {
            let mut after = before.clone();
            if !after.is_empty() && !after.ends_with('\n') {
                after.push('\n');
            }
            if !after.is_empty() && !after.ends_with("\n\n") {
                after.push('\n');
            }
            after.push_str(&line);
            after.push('\n');
            after
        };
        Ok(Self {
            path,
            before,
            after,
        })
    }
    pub fn installed(&self) -> bool {
        self.before == self.after
    }
    pub fn diff(&self) -> Vec<(char, String)> {
        if self.installed() {
            return Vec::new();
        }
        let a: Vec<_> = self.before.lines().collect();
        let b: Vec<_> = self.after.lines().collect();
        let common = a.iter().zip(&b).take_while(|(a, b)| a == b).count();
        let mut result = vec![
            (' ', format!("--- {}", self.path.display())),
            (' ', format!("+++ {}", self.path.display())),
            (
                ' ',
                format!(
                    "@@ -{},{} +{},{} @@",
                    common.saturating_sub(3) + 1,
                    a.len().saturating_sub(common.saturating_sub(3)),
                    common.saturating_sub(3) + 1,
                    b.len().saturating_sub(common.saturating_sub(3))
                ),
            ),
        ];
        for l in a.iter().take(common).skip(common.saturating_sub(3)) {
            result.push((' ', (*l).into()));
        }
        // Preserve common trailing lines as context rather than marking everything changed.
        let mut i = common;
        let mut j = common;
        while i < a.len() || j < b.len() {
            if i < a.len() && j < b.len() && a[i] == b[j] {
                result.push((' ', a[i].into()));
                i += 1;
                j += 1;
            } else if j < b.len() && (i == a.len() || !a[i..].contains(&b[j])) {
                result.push(('+', b[j].into()));
                j += 1;
            } else if i < a.len() {
                result.push(('-', a[i].into()));
                i += 1;
            } else {
                result.push(('+', b[j].into()));
                j += 1;
            }
        }
        result
    }
    pub fn apply(&self) -> Result<()> {
        ensure!(!self.installed(), "The include is already installed");
        let original = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.path)?;
        original.try_lock().context("Sway config is being edited")?;
        ensure!(
            fs::read_to_string(&self.path)? == self.before,
            "Sway config changed since the diff. Check it again."
        );
        let parent = self.path.parent().context("Config parent directory")?;
        // Keep an exact backup with a unique filename; preserve config permissions.
        let mut backup = tempfile::Builder::new()
            .prefix("config.layman-backup-")
            .tempfile_in(parent)?;
        backup.write_all(self.before.as_bytes())?;
        backup.as_file().sync_all()?;
        backup.keep()?;
        let mut replacement = tempfile::NamedTempFile::new_in(parent)?;
        replacement.write_all(self.after.as_bytes())?;
        replacement
            .as_file()
            .set_permissions(original.metadata()?.permissions())?;
        replacement.as_file().sync_all()?;
        // Recheck after staging to avoid overwriting an intervening external edit.
        ensure!(
            fs::read_to_string(&self.path)? == self.before,
            "Sway config changed since the diff. Check it again."
        );
        replacement.persist(&self.path)?;
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    }
}
pub fn check() -> Result<Patch> {
    Patch::check(
        &crate::ipc::config_path()?,
        &crate::store::directory()?.join("sway.conf"),
    )
}
