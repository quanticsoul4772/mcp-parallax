//! Secret-named files (055): the name rules, on text alone.
//!
//! A path is refused when its name marks it as a likely secret: `.env`, a
//! private key, a credentials file, anything inside `.ssh`, and the rest of
//! the tables below. The rules here never touch the disk. The reader applies
//! them to the path as written (after [`simplify`]) and to the whole path the
//! operating system resolves it to; this module only answers whether a list
//! of path parts is refused, and by which rule.
//!
//! The tables are mirrored, line for line, in
//! `specs/055-grounded-verify-secret-refusal/contracts/refusal.md`.

/// Which operating system's path rules apply. Passed in rather than read
/// from the build target, so the Windows rules are tested on every system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// `\` also separates folders, the leading `\\?\`-style forms and a drive
    /// letter are set aside, and a colon in what remains is refused.
    Windows,
    /// Only `/` separates folders; a colon is an ordinary character.
    Other,
}

/// A path reduced to its parts on text alone (FR-001's simplification).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Simplified {
    /// The parts in order, as written (letter case kept), with empty and `.`
    /// parts dropped and `..` applied where it can be.
    pub parts: Vec<String>,
    /// On Windows, whether a colon remains once the leading forms and a drive
    /// letter are set aside. Always `false` on other systems.
    pub colon_in_rest: bool,
}

/// The rule that refused a path, numbered as in FR-001.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    /// 1: the file name is one of [`EXACT_NAMES`].
    ExactName,
    /// 2: the file name starts with one of [`NAME_PREFIXES`].
    NamePrefix,
    /// 3: the file name ends with one of [`NAME_ENDINGS`] or contains one of
    /// [`NAME_CONTAINS`].
    NameEnding,
    /// 4: the backup reduction. It is applied to every part before the other
    /// rules, so it is never itself the rule reported.
    Backup,
    /// 5: some part of the path is one of [`PART_NAMES`].
    PartName,
    /// 6: the file name is one of [`GIT_CONFIG_NAMES`] and a part above it
    /// ends in `.git`.
    GitConfig,
    /// 7: two adjacent parts are one of [`PATH_SEQUENCES`].
    PathSequence,
    /// 8: on Windows, a colon remains in the path.
    Colon,
}

/// Rule 1: exact file names.
pub const EXACT_NAMES: &[&str] = &[
    ".env",
    ".envrc",
    ".netrc",
    "_netrc",
    ".npmrc",
    ".yarnrc.yml",
    ".pypirc",
    "pip.conf",
    "pip.ini",
    ".pgpass",
    "pgpass.conf",
    ".my.cnf",
    ".htpasswd",
    ".git-credentials",
    ".gitconfig",
    ".modal.toml",
    ".mcp.json",
    ".claude.json",
    "claude_desktop_config.json",
    ".credentials.json",
    "kaggle.json",
    ".vault-token",
    ".dockercfg",
    ".terraformrc",
    "terraform.rc",
    "credentials.tfrc.json",
    "kubeconfig",
    ".dev.vars",
    "local.settings.json",
    "token.json",
    "rclone.conf",
    ".boto",
    ".s3cfg",
    "application_default_credentials.json",
    ".secrets",
    "secret.json",
    "secret.yaml",
    "secret.yml",
    "secret.toml",
    "secrets.json",
    "secrets.yaml",
    "secrets.yml",
    "secrets.toml",
    "credentials",
    "credentials.json",
    "credentials.toml",
    "credentials.yaml",
    "credentials.yml",
    "credentials.xml",
    "id_rsa",
    "id_dsa",
    "id_ecdsa",
    "id_ed25519",
    "id_ecdsa_sk",
    "id_ed25519_sk",
];

/// Rule 2: file-name beginnings.
pub const NAME_PREFIXES: &[&str] = &[".env.", ".env-", ".env_", "client_secret"];

/// Rule 3: file-name endings.
pub const NAME_ENDINGS: &[&str] = &[
    ".env",
    ".pem",
    ".key",
    ".pfx",
    ".p12",
    ".p8",
    ".ppk",
    ".kdbx",
    ".jks",
    ".jceks",
    ".keystore",
    ".keytab",
    ".gpg",
    ".asc",
    ".ovpn",
    ".tfvars",
    ".tfvars.json",
    ".tfstate",
    ".tfstate.backup",
    ".kubeconfig",
];

/// Rule 3: text a file name may not contain.
pub const NAME_CONTAINS: &[&str] = &[".tfstate."];

/// Rule 4: backup endings, removed repeatedly from every part first.
pub const BACKUP_ENDINGS: &[&str] = &["~", ".bak", ".old", ".orig", ".backup", ".save"];

/// Rule 5: names no part of the path may have.
pub const PART_NAMES: &[&str] = &[
    ".ssh", ".aws", ".gnupg", ".azure", ".kube", ".kaggle", ".docker", ".envs", ".secrets",
    "secrets",
];

/// Rule 6: git configuration file names, refused under a folder ending `.git`.
pub const GIT_CONFIG_NAMES: &[&str] = &["config", "config.worktree"];

/// Rule 7: adjacent folder pairs.
pub const PATH_SEQUENCES: &[(&str, &str)] = &[(".config", "gcloud"), (".config", "gh")];

/// Simplify `path` on text alone, as FR-001 describes.
#[must_use]
pub fn simplify(path: &str, platform: Platform) -> Simplified {
    let (rest, colon_in_rest) = match platform {
        Platform::Windows => {
            let rest = strip_windows_leading_forms(path);
            (rest, rest.contains(':'))
        }
        Platform::Other => (path, false),
    };
    let mut parts: Vec<String> = Vec::new();
    let separators: &[char] = match platform {
        Platform::Windows => &['/', '\\'],
        Platform::Other => &['/'],
    };
    for part in rest.split(separators) {
        match part {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|last| last != "..") {
                    parts.pop();
                } else {
                    parts.push(part.to_string());
                }
            }
            _ => parts.push(part.to_string()),
        }
    }
    Simplified {
        parts,
        colon_in_rest,
    }
}

/// On Windows, set aside a leading `\\?\UNC\`, `\\?\`, `\\.\` or `\??\`
/// (either slash, any letter case), then a leading drive letter and its colon.
fn strip_windows_leading_forms(path: &str) -> &str {
    let is_sep = |c: u8| c == b'/' || c == b'\\';
    let b = path.as_bytes();
    let mut start = 0;
    // `\\?\` and `\\.\`: two separators, `?` or `.`, a separator.
    if b.len() >= 4
        && is_sep(b[0])
        && is_sep(b[1])
        && (b[2] == b'?' || b[2] == b'.')
        && is_sep(b[3])
    {
        start = 4;
        // `\\?\UNC\`: the share follows, with no drive letter.
        if b[2] == b'?' && b.len() >= 8 && b[4..7].eq_ignore_ascii_case(b"UNC") && is_sep(b[7]) {
            return &path[8..];
        }
    } else if b.len() >= 4 && is_sep(b[0]) && b[1] == b'?' && b[2] == b'?' && is_sep(b[3]) {
        // `\??\`.
        start = 4;
    }
    let rest = &b[start..];
    if rest.len() >= 2 && rest[0].is_ascii_alphabetic() && rest[1] == b':' {
        start += 2;
    }
    &path[start..]
}

/// The lowest-numbered rule that refuses `simplified`, if any.
#[must_use]
pub fn refusal(simplified: &Simplified, platform: Platform) -> Option<Rule> {
    let reduced: Vec<String> = simplified.parts.iter().map(|p| reduce(p)).collect();
    if let Some((file, above)) = reduced.split_last() {
        let file = file.as_str();
        if EXACT_NAMES.contains(&file) {
            return Some(Rule::ExactName);
        }
        if NAME_PREFIXES.iter().any(|p| file.starts_with(p)) {
            return Some(Rule::NamePrefix);
        }
        if NAME_ENDINGS.iter().any(|e| file.ends_with(e))
            || NAME_CONTAINS.iter().any(|c| file.contains(c))
        {
            return Some(Rule::NameEnding);
        }
        if reduced
            .iter()
            .any(|part| PART_NAMES.contains(&part.as_str()))
        {
            return Some(Rule::PartName);
        }
        if GIT_CONFIG_NAMES.contains(&file)
            && above
                .iter()
                .any(|part| part.rsplit_once('.').is_some_and(|(_, ext)| ext == "git"))
        {
            return Some(Rule::GitConfig);
        }
        if reduced.windows(2).any(|pair| {
            PATH_SEQUENCES
                .iter()
                .any(|(first, second)| pair[0] == *first && pair[1] == *second)
        }) {
            return Some(Rule::PathSequence);
        }
    }
    if platform == Platform::Windows && simplified.colon_in_rest {
        return Some(Rule::Colon);
    }
    None
}

/// The first folder, or adjacent pair of folders, refused by the folder rules.
///
/// The folder rules are 5 and 7, after rule 4's reduction. The folder is
/// named as written; a pair is joined with `/`. The file-name rules are not
/// applied: a folder named `credentials` is an ordinary folder.
#[must_use]
pub fn refused_folder(parts: &[String]) -> Option<String> {
    let reduced: Vec<String> = parts.iter().map(|p| reduce(p)).collect();
    for (i, part) in reduced.iter().enumerate() {
        if PART_NAMES.contains(&part.as_str()) {
            return Some(parts[i].clone());
        }
        if i > 0
            && PATH_SEQUENCES
                .iter()
                .any(|(first, second)| reduced[i - 1] == *first && part == second)
        {
            return Some(format!("{}/{}", parts[i - 1], parts[i]));
        }
    }
    None
}

/// A part lowercased, with every trailing backup ending removed (rule 4).
fn reduce(part: &str) -> String {
    let mut name = part.to_ascii_lowercase();
    while let Some(ending) = BACKUP_ENDINGS.iter().find(|e| name.ends_with(*e)) {
        name.truncate(name.len() - ending.len());
    }
    name
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;
