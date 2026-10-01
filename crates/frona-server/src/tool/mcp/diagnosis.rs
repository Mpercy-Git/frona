//! Plain-language causes for a package manager that gave up.
//!
//! npm and uv fail at length and in code. A server whose npm package was never
//! published prints a registry error, an upgrade nag, and a path to a debug log
//! nobody outside the sandbox can open - and nowhere in all of it the sentence
//! "that package does not exist". Frona passed that transcript straight through:
//! the install error was a tail of it, and every start afterwards wrote the same
//! wall into the server's log, so the one fact that mattered was the one nobody
//! could see.
//!
//! [`explain_package_failure`] reads the machine-readable code out of that
//! output and says what it means. It answers `None` for output it does not
//! recognise, so a caller never states a cause the log does not support.

use super::models::McpRuntime;

/// One sentence naming why the package manager gave up, followed by what to do
/// about it. `None` when the output carries no code we can speak for.
///
/// `version` is the request, not what landed: "latest" for an unpinned install,
/// and empty when the invocation named no version at all.
pub fn explain_package_failure(
    runtime: &McpRuntime,
    package: &str,
    version: &str,
    log: &str,
) -> Option<String> {
    match runtime {
        McpRuntime::Npm => explain_npm(package, version, log),
        McpRuntime::Pypi => explain_pypi(package, version, log),
        McpRuntime::Binary => explain_binary(package, log),
        McpRuntime::Remote => None,
    }
}

/// How the target was asked for, as the registry was asked for it.
fn target(package: &str, version: &str, separator: &str) -> String {
    if version.is_empty() {
        package.to_string()
    } else {
        format!("{package}{separator}{version}")
    }
}

/// The last `npm error code <CODE>` in the output. Last rather than first
/// because the log accumulates across runs: the newest failure is the one being
/// explained. `npm ERR!` is npm 10 and older, still what some images ship.
fn npm_error_code(log: &str) -> Option<&str> {
    log.lines()
        .filter_map(|line| {
            let line = line.trim();
            let code = line
                .strip_prefix("npm error code ")
                .or_else(|| line.strip_prefix("npm ERR! code "))?
                .trim();
            (!code.is_empty()).then_some(code)
        })
        .next_back()
}

const NPM_NAME_ADVICE: &str = "Check the package name on the server's registry entry - it may have been renamed or \
     unpublished - or install this server from its repository or its remote URL instead.";

fn explain_npm(package: &str, version: &str, log: &str) -> Option<String> {
    let target = target(package, version, "@");
    let explanation = match npm_error_code(log)? {
        // The name itself is unknown to the registry.
        "E404" => format!("the npm registry has no package called '{package}'. {NPM_NAME_ADVICE}"),
        // The name is known but every version of it is gone - the shape an
        // unpublished package takes when something asks for it by name.
        "ENOVERSIONS" => format!(
            "the npm registry knows '{package}' but it has no published versions, which is what \
             an unpublished package looks like. {NPM_NAME_ADVICE}"
        ),
        // Same package, asked for with a version (including the implicit
        // "latest"), which is the form the install phase uses.
        "ETARGET" => {
            format!("the npm registry has no version matching '{target}'. {NPM_NAME_ADVICE}")
        }
        "ENEEDAUTH" | "E401" | "E403" => format!(
            "the npm registry refused to serve '{target}' without credentials. A private package \
             has to be installed from a source this server can authenticate to."
        ),
        "EAI_AGAIN" | "ENOTFOUND" | "ECONNREFUSED" | "ETIMEDOUT" | "ERR_SOCKET_TIMEOUT" => format!(
            "the npm registry could not be reached while resolving '{target}'. This one is \
             usually worth retrying."
        ),
        _ => return None,
    };
    Some(explanation)
}

fn explain_pypi(package: &str, version: &str, log: &str) -> Option<String> {
    let target = target(package, version, "==");
    if log.contains("was not found in the package registry")
        || log.contains("No matching distribution found")
        || log.contains("Distribution not found")
    {
        return Some(format!(
            "PyPI has no distribution matching '{target}'. Check the package name on the server's \
             registry entry, or install this server from its repository instead."
        ));
    }
    if log.contains("No solution found when resolving") {
        return Some(format!(
            "no set of dependencies satisfies '{target}' on this Python version. Pinning a \
             version the package still supports is the usual way out."
        ));
    }
    None
}

fn explain_binary(package: &str, log: &str) -> Option<String> {
    (log.contains("No such file or directory") || log.contains("command not found")).then(|| {
        format!(
            "'{package}' was not found where the sandbox looked for it. A binary server has to \
             exist inside the sandbox, at the absolute path the install recorded."
        )
    })
}

/// The tail of a log, minus the lines that only ever repeat themselves: npm's
/// nag about its own next major version, and its pointer at a debug file that
/// lives inside the sandbox. Both crowd out the error in a short tail - the nag
/// alone is five of the last ten lines of a failed npm run.
///
/// Returns at most `max_lines` lines, oldest first.
pub fn relevant_tail(log: &str, max_lines: usize) -> String {
    let mut kept: Vec<&str> = log
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .filter(|line| !is_noise(line))
        .rev()
        .take(max_lines)
        .collect();
    kept.reverse();
    kept.join("\n")
}

fn is_noise(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed == "npm notice"
        || trimmed.starts_with("npm notice ")
        || trimmed.contains("A complete log of this run can be found in")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The install phase of the failure this module was written for: an
    /// unpinned package that npm resolves as `@latest`.
    const ETARGET_LOG: &str = "\
npm error code ETARGET
npm error notarget No matching version found for trek-mcp-server@latest.
npm error notarget In most cases you or one of your dependencies are requesting a package version that doesn't exist.
npm notice
npm notice New major version of npm available! 11.17.0 -> 12.0.2
npm notice Changelog: https://github.com/npm/cli/releases/tag/v12.0.2
npm notice To update run: npm install -g npm@12.0.2
npm notice
npm error A complete log of this run can be found in: /app/data/users/mpercy/mcps/trek/.npm-cache/_logs/2026-08-07T12_19_26_953Z-debug-0.log
";

    /// The start phase of the same failure: the same package asked for by name.
    const ENOVERSIONS_LOG: &str = "\
npm error code ENOVERSIONS
npm error No versions available for trek-mcp-server
npm notice
npm notice New major version of npm available! 11.19.0 -> 12.0.2
";

    #[test]
    fn names_the_missing_version_behind_etarget() {
        let explanation =
            explain_package_failure(&McpRuntime::Npm, "trek-mcp-server", "latest", ETARGET_LOG)
                .expect("ETARGET is a code we speak for");
        assert!(
            explanation.contains("no version matching 'trek-mcp-server@latest'"),
            "{explanation}"
        );
        assert!(explanation.contains("registry entry"), "{explanation}");
    }

    #[test]
    fn names_the_unpublished_package_behind_enoversions() {
        let explanation =
            explain_package_failure(&McpRuntime::Npm, "trek-mcp-server", "", ENOVERSIONS_LOG)
                .expect("ENOVERSIONS is a code we speak for");
        assert!(explanation.contains("unpublished"), "{explanation}");
        assert!(explanation.contains("'trek-mcp-server'"), "{explanation}");
    }

    #[test]
    fn a_missing_name_reads_differently_from_a_missing_version() {
        let missing_name = explain_package_failure(
            &McpRuntime::Npm,
            "trek-mcp-server",
            "latest",
            "npm error code E404\nnpm error 404 Not Found - GET https://registry.npmjs.org/trek-mcp-server\n",
        )
        .unwrap();
        assert!(missing_name.contains("no package called"), "{missing_name}");
    }

    /// The log is appended to across runs, so an old failure must not out-shout
    /// the one being explained.
    #[test]
    fn the_newest_code_in_an_appended_log_wins() {
        let log = format!("{ETARGET_LOG}{ENOVERSIONS_LOG}");
        let explanation =
            explain_package_failure(&McpRuntime::Npm, "trek-mcp-server", "", &log).unwrap();
        assert!(explanation.contains("unpublished"), "{explanation}");
    }

    #[test]
    fn network_failures_are_told_apart_from_missing_packages() {
        let explanation = explain_package_failure(
            &McpRuntime::Npm,
            "@example/thing",
            "1.2.3",
            "npm error code EAI_AGAIN\nnpm error request to https://registry.npmjs.org failed\n",
        )
        .unwrap();
        assert!(
            explanation.contains("could not be reached"),
            "{explanation}"
        );
        assert!(explanation.contains("retrying"), "{explanation}");
    }

    #[test]
    fn an_unrecognised_code_is_left_unexplained() {
        assert!(
            explain_package_failure(
                &McpRuntime::Npm,
                "thing",
                "1.0.0",
                "npm error code EJACKALOPE\nnpm error something new\n",
            )
            .is_none()
        );
        assert!(
            explain_package_failure(&McpRuntime::Npm, "thing", "1.0.0", "it just died\n").is_none()
        );
    }

    #[test]
    fn legacy_npm_error_prefixes_are_still_read() {
        let explanation = explain_package_failure(
            &McpRuntime::Npm,
            "thing",
            "1.0.0",
            "npm ERR! code E404\nnpm ERR! 404 Not Found\n",
        )
        .unwrap();
        assert!(explanation.contains("no package called"), "{explanation}");
    }

    #[test]
    fn a_missing_pypi_distribution_is_explained() {
        let explanation = explain_package_failure(
            &McpRuntime::Pypi,
            "trek-mcp",
            "1.0.0",
            "error: Distribution not found at: file:///nope\n",
        )
        .unwrap();
        assert!(explanation.contains("'trek-mcp==1.0.0'"), "{explanation}");
    }

    #[test]
    fn a_remote_server_has_no_package_to_explain() {
        assert!(
            explain_package_failure(&McpRuntime::Remote, "", "", "npm error code E404").is_none()
        );
    }

    #[test]
    fn the_tail_drops_npm_nags_and_keeps_the_error() {
        let tail = relevant_tail(ETARGET_LOG, 5);
        assert!(tail.contains("npm error code ETARGET"), "{tail}");
        assert!(tail.contains("No matching version found"), "{tail}");
        assert!(!tail.contains("npm notice"), "{tail}");
        assert!(!tail.contains("A complete log of this run"), "{tail}");
    }

    #[test]
    fn the_tail_is_the_end_of_the_log_in_order() {
        let tail = relevant_tail("one\ntwo\nthree\nfour\n", 2);
        assert_eq!(tail, "three\nfour");
    }
}
