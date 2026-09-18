//! The direction the runtime layer depends in.
//!
//! The runtime compiles and runs a guest and bounds its resources.
//! It does that the same way whichever ABI version the guest speaks.
//! It therefore names the versioned module only where a version has to be
//! chosen, and the list below records every such place and why.
//!
//! This is a text search, so it finds a name only where a name is written.
//! A dependency held by a type alias, by a re-export outside this directory,
//! by a macro that builds the path, or by a value whose type is never spelled
//! at the call site is invisible to it.
//! It deters an accident and it does not resist intent.
//! It stands in until the two layers are separate compilation units, which is
//! what makes the direction the compiler's job.

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    /// A file that may name the versioned module, and the reason it may.
    struct Allowed {
        file: &'static str,
        reason: &'static str,
    }

    const ALLOWED: &[Allowed] = &[
        Allowed {
            file: "runtime/engine.rs",
            reason: "the linker takes its host functions through a registrar, \
                     and the default one is this version's",
        },
        Allowed {
            file: "runtime/layering.rs",
            reason: "the check holds the text it searches for",
        },
    ];

    fn src_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
    }

    fn runtime_sources() -> Vec<PathBuf> {
        fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, found);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    found.push(path);
                }
            }
        }
        let root = src_root();
        let mut found = vec![root.join("runtime.rs")];
        walk(&root.join("runtime"), &mut found);
        found
    }

    /// The version segment, which is unique in this crate.
    ///
    /// The search is for the bare segment rather than for a whole path.
    /// A formatter cannot split an identifier, so a wrapped path still
    /// matches, and a braced import that renames the module still matches
    /// because the segment is written either way.
    fn names_the_version(source: &str) -> bool {
        source.contains("v0_2_1")
    }

    /// The path a listing and a message name a source by.
    fn listed(path: &Path) -> String {
        path.strip_prefix(src_root())
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }

    /// Every line of `sources` that names the version in a file the list does
    /// not allow.
    fn offenders(sources: &[(String, String)]) -> Vec<String> {
        sources
            .iter()
            .filter(|(name, _)| !ALLOWED.iter().any(|a| a.file == name))
            .flat_map(|(name, body)| {
                body.lines()
                    .enumerate()
                    .filter(|(_, line)| names_the_version(line))
                    .map(move |(at, line)| format!("{name}:{}: {}", at + 1, line.trim()))
            })
            .collect()
    }

    fn read_runtime_sources() -> Vec<(String, String)> {
        let sources = runtime_sources();
        assert!(
            sources.iter().any(|p| p.ends_with("runtime.rs")),
            "the walk must reach the module root, where a re-export would go"
        );
        assert!(
            sources.len() > 5,
            "a walk that found nothing would report nothing and pass"
        );
        sources
            .iter()
            .map(|p| (listed(p), std::fs::read_to_string(p).unwrap_or_default()))
            .collect()
    }

    #[test]
    fn the_runtime_names_the_version_only_where_the_list_allows() {
        // Arrange
        let sources = read_runtime_sources();

        // Act
        let found = offenders(&sources);

        // Assert
        let allowed: Vec<String> = ALLOWED
            .iter()
            .map(|a| format!("{} because {}", a.file, a.reason))
            .collect();
        assert!(
            found.is_empty(),
            "the runtime layer names the versioned ABI module here:\n  {}\n\
             Reach it through something the ABI layer owns, or add the file \
             below with the reason it has to.\nAllowed today:\n  {}",
            found.join("\n  "),
            allowed.join("\n  ")
        );
    }

    #[test]
    fn a_file_the_list_does_not_allow_is_reported_with_its_line() {
        // Arrange
        let sources = [(
            "runtime/limits.rs".to_owned(),
            "fn f() {}\nuse crate::abi::v0_2_1::AbiState;\n".to_owned(),
        )];

        // Act
        let found = offenders(&sources);

        // Assert
        assert_eq!(
            found,
            ["runtime/limits.rs:2: use crate::abi::v0_2_1::AbiState;"]
        );
    }

    #[test]
    fn a_file_the_list_allows_is_not_reported() {
        // Arrange
        let sources = [(
            "runtime/engine.rs".to_owned(),
            "crate::abi::v0_2_1::host_functions::register\n".to_owned(),
        )];

        // Act
        let found = offenders(&sources);

        // Assert
        assert!(found.is_empty());
    }

    #[test]
    fn a_nested_file_is_not_exempt_by_sharing_a_name() {
        // Arrange
        let sources = [(
            "runtime/nested/engine.rs".to_owned(),
            "use crate::abi::v0_2_1::AbiState;\n".to_owned(),
        )];

        // Act
        let found = offenders(&sources);

        // Assert
        assert_eq!(found.len(), 1, "the list names a path, not a file name");
    }

    #[test]
    fn a_spelling_that_hides_the_path_is_still_found() {
        // Arrange
        // Neither of these holds the path as one string, and both write the
        // version segment in the file.
        let hidden = [
            "use crate::abi::\n    v0_2_1\n    ::AbiState;",
            "use crate::abi::{version, v0_2_1 as v};",
        ];

        // Act
        let found = hidden.map(names_the_version);

        // Assert
        assert_eq!(found, [true; 2]);
    }

    #[test]
    fn a_file_that_names_no_version_is_not_reported() {
        // Arrange
        let plain = "use crate::runtime::HostState;";

        // Act
        let found = names_the_version(plain);

        // Assert
        assert!(!found);
    }

    #[test]
    fn every_allowed_file_exists_and_still_names_the_version() {
        // Arrange
        let sources = read_runtime_sources();

        // Act
        let stale: Vec<&str> = ALLOWED
            .iter()
            .filter(|allowed| {
                sources
                    .iter()
                    .find(|(name, _)| name == allowed.file)
                    .is_none_or(|(_, body)| !names_the_version(body))
            })
            .map(|allowed| allowed.file)
            .collect();

        // Assert
        assert!(
            stale.is_empty(),
            "these entries name a file that is gone or no longer needs one: {stale:?}"
        );
    }
}
