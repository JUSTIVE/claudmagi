//! The macOS shim: the one thing a bundle has to fix about how it was
//! launched.

/// Where a command-line tool lands when a shell's profile put it on `PATH`
/// and launchd's environment did not. Homebrew first on Apple silicon, then
/// the Intel/`/usr/local` prefix, then the two user-level `bin` directories.
const SHELL_PREFIXES: [&str; 4] =
    ["/opt/homebrew/bin", "/opt/homebrew/sbin", "/usr/local/bin", "/usr/local/sbin"];
const HOME_PREFIXES: [&str; 2] = [".local/bin", ".cargo/bin"];

/// Puts back the `PATH` entries a login shell would have added (#66).
///
/// An app launched from Finder, Spotlight or `open` inherits launchd's
/// environment, not a shell's, so `PATH` comes up as
/// `/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin` however the terminal that
/// built it was set up. `git`, `ps`, `open` and `orca` all live inside that
/// and kept working; `gh` lives in Homebrew's `/opt/homebrew/bin` and did
/// not, so in the installed app every pull request resolved to "no `gh`, no
/// connector" while `--prs` from a terminal listed them all.
///
/// # Safety
///
/// Call it as the first thing `main` does. `set_var` is a data race against
/// any other thread reading the environment, and at that point there are none
/// — the `gh` and `orca` workers are spawned much later.
pub unsafe fn widen_path() {
    let widened = widen(&std::env::var("PATH").unwrap_or_default(), &home_prefixes());
    unsafe { std::env::set_var("PATH", widened) }
}

fn home_prefixes() -> Vec<String> {
    let Some(home) = dirs::home_dir() else { return Vec::new() };
    HOME_PREFIXES.iter().map(|p| home.join(p).to_string_lossy().into_owned()).collect()
}

/// Appends the prefixes that are missing, keeping whatever order the
/// environment already had — a shell that set `PATH` deliberately keeps its
/// own precedence, and a launchd one simply gains the entries it lacks.
fn widen(current: &str, home: &[String]) -> String {
    let mut dirs: Vec<&str> = current.split(':').filter(|s| !s.is_empty()).collect();
    let extras = SHELL_PREFIXES.iter().copied().chain(home.iter().map(String::as_str));
    for extra in extras {
        if !dirs.contains(&extra) {
            dirs.push(extra);
        }
    }
    dirs.join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launchd_path_gains_the_shell_prefixes() {
        let home = vec!["/Users/x/.local/bin".to_string()];
        let got = widen("/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin", &home);
        assert_eq!(
            got,
            "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/sbin:/Users/x/.local/bin",
            "the missing ones are appended and /usr/local/bin is not repeated"
        );
    }

    #[test]
    fn a_shell_path_keeps_its_own_precedence() {
        let got = widen("/my/tools:/opt/homebrew/bin:/usr/bin", &[]);
        assert!(got.starts_with("/my/tools:/opt/homebrew/bin:/usr/bin"), "{got}");
        assert_eq!(got.matches("/opt/homebrew/bin").count(), 1, "no duplicates");
    }

    #[test]
    fn an_empty_path_is_still_usable() {
        assert_eq!(widen("", &[]), SHELL_PREFIXES.join(":"));
    }
}
