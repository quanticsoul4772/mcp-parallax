use super::*;

const BOTH: [Platform; 2] = [Platform::Windows, Platform::Other];

fn parts(path: &str, platform: Platform) -> Vec<String> {
    simplify(path, platform).parts
}

#[test]
fn simplify_drops_empty_and_dot_parts_and_applies_dot_dot() {
    for p in BOTH {
        assert_eq!(parts("a//./b/", p), ["a", "b"], "{p:?}");
        assert_eq!(parts("a/../b", p), ["b"], "{p:?}");
        assert_eq!(parts(".ssh/../README.md", p), ["README.md"], "{p:?}");
        assert_eq!(parts("../x", p), ["..", "x"], "{p:?}");
        assert_eq!(parts("../..", p), ["..", ".."], "{p:?}");
        assert_eq!(parts("a/b/../../../c", p), ["..", "c"], "{p:?}");
        assert_eq!(parts("Dir/File.TXT", p), ["Dir", "File.TXT"], "{p:?}");
        assert!(!simplify("a/b", p).colon_in_rest, "{p:?}");
    }
}

#[test]
fn on_windows_a_backslash_separates_and_leading_forms_are_set_aside() {
    let w = Platform::Windows;
    assert_eq!(parts(r"a\b\c.txt", w), ["a", "b", "c.txt"]);
    for path in [
        r"\\?\UNC\host\share\x.txt",
        "//?/UNC/host/share/x.txt",
        r"\\?\unc\host\share\x.txt",
    ] {
        assert_eq!(parts(path, w), ["host", "share", "x.txt"], "{path}");
        assert!(!simplify(path, w).colon_in_rest, "{path}");
    }
    for path in [
        r"\\?\C:\r\x.txt",
        "//?/C:/r/x.txt",
        r"\\.\C:\r\x.txt",
        "//./C:/r/x.txt",
        r"\??\C:\r\x.txt",
        "/??/C:/r/x.txt",
        r"C:\r\x.txt",
        "c:/r/x.txt",
    ] {
        assert_eq!(parts(path, w), ["r", "x.txt"], "{path}");
        assert!(!simplify(path, w).colon_in_rest, "{path}");
    }
    assert!(simplify("notes.txt:hidden", w).colon_in_rest);
    assert!(simplify(r"C:\r\notes.txt::$DATA", w).colon_in_rest);
    assert!(simplify(r"\\?\C:\r\a.rs:12", w).colon_in_rest);
}

#[test]
fn elsewhere_a_backslash_and_a_colon_are_ordinary_characters() {
    let o = Platform::Other;
    assert_eq!(parts(r"a\b\c.txt", o), [r"a\b\c.txt"]);
    assert_eq!(parts(r"C:\r\x.txt", o), [r"C:\r\x.txt"]);
    assert_eq!(parts("C:/r/x.txt", o), ["C:", "r", "x.txt"]);
    assert!(!simplify("notes.txt:hidden", o).colon_in_rest);
    assert!(!simplify("C:/r/x.txt", o).colon_in_rest);
}

fn rule_for(path: &str, platform: Platform) -> Option<Rule> {
    refusal(&simplify(path, platform), platform)
}

/// SC-001's third binding: typed out by hand, not derived from the
/// tables, so a wrong table is caught here.
#[test]
fn the_hand_typed_list_is_refused_by_the_expected_rule() {
    let on_both = [
        (".env", Rule::ExactName),
        ("config/.ENV", Rule::ExactName),
        (".env.local", Rule::NamePrefix),
        ("prod.env", Rule::NameEnding),
        ("id_rsa", Rule::ExactName),
        ("keys/id_ed25519", Rule::ExactName),
        ("server.pem", Rule::NameEnding),
        ("a/b.key", Rule::NameEnding),
        ("id_rsa.bak.old", Rule::ExactName),
        (".env~", Rule::ExactName),
        ("secrets.yaml.orig", Rule::ExactName),
        (".ssh/known_hosts", Rule::PartName),
        (".ssh.bak/known_hosts", Rule::PartName),
        ("x/secrets/y.txt", Rule::PartName),
        (".git/config", Rule::GitConfig),
        (".git/config.bak", Rule::GitConfig),
        ("project.git/config", Rule::GitConfig),
        (".config/gh/hosts.yml", Rule::PathSequence),
        (".config.old/gcloud/x", Rule::PathSequence),
        ("terraform.tfstate.1699999999.backup", Rule::NameEnding),
        ("client_secret_1.json", Rule::NamePrefix),
    ];
    for (path, rule) in on_both {
        for p in BOTH {
            assert_eq!(rule_for(path, p), Some(rule), "{path} {p:?}");
        }
    }
    for path in ["notes.txt:hidden", r"C:\r\notes.txt::$DATA"] {
        assert_eq!(
            rule_for(path, Platform::Windows),
            Some(Rule::Colon),
            "{path}"
        );
    }
}

/// SC-003: names that must stay readable.
#[test]
fn the_must_read_names_are_not_refused() {
    for path in [
        "id_rsa.pub",
        "id_ed25519.pub",
        "environment.rs",
        "env.rs",
        "keys.rs",
        "token.rs",
        "secrets.rs",
        "config.toml",
        ".gitignore",
        "README.md",
        ".github/workflows/ci.yml",
    ] {
        for p in BOTH {
            assert_eq!(rule_for(path, p), None, "{path} {p:?}");
        }
    }
    assert_eq!(
        rule_for("src/a.rs:12", Platform::Windows),
        Some(Rule::Colon)
    );
    assert_eq!(rule_for("src/a.rs:12", Platform::Other), None);
}

/// Lower case, upper case, and alternating case.
fn cases(s: &str) -> [String; 3] {
    let mixed = s
        .chars()
        .enumerate()
        .map(|(i, c)| {
            if i % 2 == 0 {
                c.to_ascii_uppercase()
            } else {
                c.to_ascii_lowercase()
            }
        })
        .collect();
    [s.to_ascii_lowercase(), s.to_ascii_uppercase(), mixed]
}

fn assert_rule(path: &str, rule: Rule) {
    for p in BOTH {
        assert_eq!(rule_for(path, p), Some(rule), "{path} {p:?}");
    }
}

/// These iterate the tables, so they cannot catch a wrong table; the
/// hand-typed list above is the check that can.
#[test]
fn every_file_name_entry_is_refused_in_any_case_and_under_a_folder() {
    for entry in EXACT_NAMES {
        for name in cases(entry) {
            assert_rule(&name, Rule::ExactName);
            assert_rule(&format!("some/dir/{name}"), Rule::ExactName);
        }
    }
    for entry in NAME_PREFIXES {
        for name in cases(&format!("{entry}x")) {
            assert_rule(&name, Rule::NamePrefix);
            assert_rule(&format!("dir/{name}"), Rule::NamePrefix);
        }
    }
    for entry in NAME_ENDINGS {
        for name in cases(&format!("x{entry}")) {
            assert_rule(&name, Rule::NameEnding);
            assert_rule(&format!("dir/{name}"), Rule::NameEnding);
        }
    }
    for entry in NAME_CONTAINS {
        for name in cases(&format!("a{entry}b")) {
            assert_rule(&name, Rule::NameEnding);
        }
    }
}

#[test]
fn every_folder_entry_is_refused_in_any_case_and_from_a_root_like_prefix() {
    for entry in PART_NAMES {
        for name in cases(entry) {
            assert_rule(&format!("{name}/readme.txt"), Rule::PartName);
            assert_rule(&format!("home/u/{name}/a/b.txt"), Rule::PartName);
            // As the file itself: refused, by rule 1 where the name is
            // also an exact name, otherwise by rule 5.
            assert!(rule_for(&format!("dir/{name}"), Platform::Other).is_some());
        }
    }
    for entry in GIT_CONFIG_NAMES {
        for name in cases(entry) {
            assert_rule(&format!(".git/{name}"), Rule::GitConfig);
            assert_rule(&format!("proj.git/sub/{name}"), Rule::GitConfig);
            assert_rule(&format!("home/u/x.GIT/{name}"), Rule::GitConfig);
            assert_eq!(rule_for(&format!("proj/{name}"), Platform::Other), None);
        }
    }
    assert_eq!(
        rule_for(r"C:\r\proj.git\config", Platform::Windows),
        Some(Rule::GitConfig)
    );
    for (first, second) in PATH_SEQUENCES {
        for a in cases(first) {
            for b in cases(second) {
                assert_rule(&format!("{a}/{b}/x.json"), Rule::PathSequence);
                assert_rule(&format!("home/u/{a}/{b}/x.json"), Rule::PathSequence);
            }
        }
        assert_eq!(
            rule_for(&format!("{second}/{first}/x"), Platform::Other),
            None
        );
    }
}

#[test]
fn backup_endings_single_and_repeated_reduce_every_part() {
    for e in BACKUP_ENDINGS {
        for twice in [(*e).to_string(), format!("{e}.old~")] {
            for suffix in cases(&twice) {
                assert_rule(&format!("id_rsa{suffix}"), Rule::ExactName);
                assert_rule(&format!(".ssh{suffix}/x"), Rule::PartName);
                assert_rule(&format!(".git{suffix}/config"), Rule::GitConfig);
                assert_rule(&format!(".git/config{suffix}"), Rule::GitConfig);
                assert_rule(&format!(".config{suffix}/gh/x"), Rule::PathSequence);
                assert_rule(&format!(".config/gh{suffix}/x"), Rule::PathSequence);
            }
        }
    }
}

/// Folder rules only (5 and 7, after rule 4's reduction), naming the folder
/// as written: what the startup check and a glob's starting folder use.
#[test]
fn only_the_folder_rules_refuse_a_folder_and_it_is_named_as_written() {
    let refused = |path: &str| refused_folder(&simplify(path, Platform::Other).parts);
    assert_eq!(refused(".ssh/proj").as_deref(), Some(".ssh"));
    assert_eq!(refused("home/Secrets").as_deref(), Some("Secrets"));
    assert_eq!(refused(".ssh.bak/proj").as_deref(), Some(".ssh.bak"));
    assert_eq!(
        refused("home/.config/gh/proj").as_deref(),
        Some(".config/gh")
    );
    assert_eq!(
        refused(".CONFIG.old/GCloud").as_deref(),
        Some(".CONFIG.old/GCloud")
    );
    for path in [
        "credentials",
        "proj/.env",
        ".git/config",
        "x.git",
        ".config",
        "gh/.config",
    ] {
        assert_eq!(refused(path), None, "{path}");
    }
}

/// 055 FR-013: the rule block on the contract page and the tables here are
/// the same lists, in both directions, with the stated number of each kind.
#[test]
fn the_contract_page_lists_exactly_the_rules_in_the_code() {
    use std::collections::{BTreeMap, BTreeSet};
    let page =
        include_str!("../../../specs/055-grounded-verify-secret-refusal/contracts/refusal.md");
    let open = page
        .find("```text")
        .expect("the rule block's opening fence");
    let body = &page[open..];
    let body = &body[body.find('\n').expect("a line after the fence") + 1..];
    let body = &body[..body.find("```").expect("the rule block's closing fence")];

    let mut block: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for line in body.lines() {
        let split = line.trim_end_matches('\r').split_once(' ');
        assert!(
            split.is_some(),
            "unreadable line in the rule block: {line:?}"
        );
        let (kind, value) = split.unwrap();
        assert!(
            matches!(
                kind,
                "exact"
                    | "prefix"
                    | "ending"
                    | "contains"
                    | "backup"
                    | "part"
                    | "gitconfig"
                    | "sequence"
            ),
            "unknown kind in the rule block: {line:?}"
        );
        block.entry(kind).or_default().push(value);
    }
    let count = |kind: &str| block.get(kind).map_or(0, Vec::len);
    for (kind, expected) in [
        ("exact", 55),
        ("prefix", 4),
        ("ending", 20),
        ("contains", 1),
        ("backup", 6),
        ("part", 10),
        ("gitconfig", 2),
        ("sequence", 2),
    ] {
        assert_eq!(count(kind), expected, "number of {kind} lines");
    }
    let page_set =
        |kind: &str| -> BTreeSet<String> { block[kind].iter().map(|v| (*v).to_string()).collect() };
    let code_set =
        |table: &[&str]| -> BTreeSet<String> { table.iter().map(|v| (*v).to_string()).collect() };
    assert_eq!(page_set("exact"), code_set(EXACT_NAMES));
    assert_eq!(page_set("prefix"), code_set(NAME_PREFIXES));
    assert_eq!(page_set("ending"), code_set(NAME_ENDINGS));
    assert_eq!(page_set("contains"), code_set(NAME_CONTAINS));
    assert_eq!(page_set("backup"), code_set(BACKUP_ENDINGS));
    assert_eq!(page_set("part"), code_set(PART_NAMES));
    assert_eq!(page_set("gitconfig"), code_set(GIT_CONFIG_NAMES));
    let code_pairs: BTreeSet<String> = PATH_SEQUENCES
        .iter()
        .map(|(first, second)| format!("{first} {second}"))
        .collect();
    assert_eq!(page_set("sequence"), code_pairs);
    // Each table has no repeats, so equal sets mean equal lists.
    for (name, table) in [
        ("EXACT_NAMES", EXACT_NAMES),
        ("NAME_PREFIXES", NAME_PREFIXES),
        ("NAME_ENDINGS", NAME_ENDINGS),
        ("NAME_CONTAINS", NAME_CONTAINS),
        ("BACKUP_ENDINGS", BACKUP_ENDINGS),
        ("PART_NAMES", PART_NAMES),
        ("GIT_CONFIG_NAMES", GIT_CONFIG_NAMES),
    ] {
        assert_eq!(
            code_set(table).len(),
            table.len(),
            "{name} repeats an entry"
        );
    }
    assert_eq!(
        code_pairs.len(),
        PATH_SEQUENCES.len(),
        "PATH_SEQUENCES repeats an entry"
    );
}
