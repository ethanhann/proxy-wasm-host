//! The direction the runtime layer depends in.
//!
//! The runtime compiles and runs a guest and bounds its resources, and it
//! does that the same way whichever ABI version the guest speaks. It
//! therefore names the versioned module only where a version has to be
//! chosen, and the list below records every such place and why it is there.
//!
//! This is a text search, so it sees a name only where a name is written. A
//! dependency carried by a type alias, by a re-export, or by a value whose
//! type is never spelled at the call site is invisible to it. It is a
//! stand-in until the two layers are separate compilation units, which is
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
            file: "engine.rs",
            reason: "the linker takes its host functions through a registrar, \
                     and the default one is this version's",
        },
        Allowed {
            file: "services.rs",
            reason: "the services an embedder supplies carry a log level and \
                     the shared services, and the type moves to the ABI layer \
                     with its rename",
        },
        Allowed {
            file: "test_support.rs",
            reason: "the test services build a log level, and they follow the \
                     services",
        },
        Allowed {
            file: "layering.rs",
            reason: "the check holds the text it searches for",
        },
    ];

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
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found = vec![root.join("runtime.rs")];
        walk(&root.join("runtime"), &mut found);
        found
    }

    /// The versioned module, spelled so that a wrapped line still matches.
    fn names_the_version(source: &str) -> bool {
        let stripped: String = source.chars().filter(|c| !c.is_whitespace()).collect();
        stripped.contains("abi::v0_2_1") || stripped.contains("v0_2_1::")
    }

    #[test]
    fn the_walk_finds_the_runtime_sources() {
        // Arrange
        let expected = "engine.rs";

        // Act
        let found = runtime_sources();

        // Assert
        assert!(
            found.len() > 5,
            "a walk that finds nothing would pass every other test over nothing"
        );
        assert!(found.iter().any(|p| p.ends_with(expected)));
    }

    #[test]
    fn the_runtime_names_the_version_only_where_the_list_allows() {
        // Arrange
        let sources = runtime_sources();

        // Act
        let offenders: Vec<String> = sources
            .iter()
            .filter(|path| {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                !ALLOWED.iter().any(|a| a.file == name)
                    && std::fs::read_to_string(path).is_ok_and(|s| names_the_version(&s))
            })
            .map(|path| {
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into()
            })
            .collect();

        // Assert
        assert_eq!(offenders, Vec::<String>::new());
    }

    #[test]
    fn a_name_wrapped_across_two_lines_is_still_found() {
        // Arrange
        let wrapped = "use crate::abi::\n    v0_2_1::AbiState;";

        // Act
        let found = names_the_version(wrapped);

        // Assert
        assert!(
            found,
            "a formatter may wrap a long path, so the search ignores spacing"
        );
    }

    #[test]
    fn every_allowed_file_still_names_the_version() {
        // Arrange
        let sources = runtime_sources();

        // Act
        let stale: Vec<&str> = ALLOWED
            .iter()
            .filter(|allowed| {
                sources
                    .iter()
                    .find(|p| p.file_name().unwrap_or_default() == allowed.file)
                    .is_some_and(|p| {
                        std::fs::read_to_string(p).is_ok_and(|s| !names_the_version(&s))
                    })
            })
            .map(|allowed| allowed.file)
            .collect();

        // Assert
        assert_eq!(
            stale,
            Vec::<&str>::new(),
            "an entry that is no longer needed must go"
        );
    }

    #[test]
    fn every_allowed_file_carries_a_reason() {
        // Arrange
        let entries = ALLOWED;

        // Act
        let silent: Vec<&str> = entries
            .iter()
            .filter(|a| a.reason.len() < 20)
            .map(|a| a.file)
            .collect();

        // Assert
        assert_eq!(silent, Vec::<&str>::new());
    }
}
