use path_clean::PathClean;
use serde_json::{json, Value};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use url::{Host, Url};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    Web,
    LocalResource,
}

#[derive(Debug, Clone)]
pub struct ResolvedOpenTarget {
    pub navigation_url: String,
    pub kind: TargetKind,
}

#[derive(Debug, Clone)]
pub struct OpenTargetError {
    pub code: &'static str,
    pub message: String,
    pub details: Value,
}

impl OpenTargetError {
    fn new(code: &'static str, message: impl Into<String>, details: Value) -> Self {
        Self {
            code,
            message: message.into(),
            details,
        }
    }
}

impl fmt::Display for OpenTargetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for OpenTargetError {}

pub fn resolve_open_target(
    input: &str,
    cwd: Option<&Path>,
) -> Result<ResolvedOpenTarget, OpenTargetError> {
    if input.is_empty() || input.trim().is_empty() {
        return Err(OpenTargetError::new(
            "invalid_open_target",
            "Open Target must not be empty",
            json!({ "open_target": input }),
        ));
    }
    if let Some(cwd) = cwd {
        if !cwd.is_absolute() || !cwd.is_dir() {
            return Err(OpenTargetError::new(
                "invalid_cwd",
                "cwd must be an absolute existing directory",
                json!({ "open_target": input, "cwd": cwd }),
            ));
        }
    }

    if let Some(platform) = foreign_path_platform(input) {
        return Err(OpenTargetError::new(
            "foreign_platform_path",
            format!(
                "The Open Target looks like a {platform} path, but the CLI is running on {}",
                std::env::consts::OS
            ),
            json!({
                "open_target": input,
                "host_platform": std::env::consts::OS,
                "detected_platform": platform,
            }),
        ));
    }

    if cfg!(windows) && is_explicit_path(input) {
        return resolve_path_input(input, cwd);
    }

    if let Some(scheme) = explicit_scheme(input) {
        return match scheme.as_str() {
            "http" | "https" => resolve_explicit_web(input, &scheme),
            "file" => resolve_file_url(input),
            _ if looks_like_host_port(input) => resolve_implicit_web(input),
            _ => Err(OpenTargetError::new(
                "unsupported_scheme",
                format!("Unsupported Open Target scheme: {scheme}"),
                json!({ "scheme": scheme, "supported_schemes": ["http", "https", "file"] }),
            )),
        };
    }

    if is_explicit_path(input) {
        return resolve_path_input(input, cwd);
    }

    if let Some(cwd) = cwd {
        if cwd.join(input).exists() {
            return resolve_path_input(input, Some(cwd));
        }
    }

    resolve_implicit_web(input)
}

fn resolve_explicit_web(
    input: &str,
    expected_scheme: &str,
) -> Result<ResolvedOpenTarget, OpenTargetError> {
    let parsed = Url::parse(input).map_err(|err| {
        OpenTargetError::new(
            "invalid_http_url",
            format!("Invalid {} URL: {err}", expected_scheme.to_uppercase()),
            json!({ "open_target": input, "scheme": expected_scheme }),
        )
    })?;
    if parsed.host().is_none() {
        return Err(OpenTargetError::new(
            "invalid_http_url",
            format!(
                "Invalid {} URL: host is missing",
                expected_scheme.to_uppercase()
            ),
            json!({ "open_target": input, "scheme": expected_scheme }),
        ));
    }
    Ok(ResolvedOpenTarget {
        navigation_url: input.to_string(),
        kind: TargetKind::Web,
    })
}

fn resolve_implicit_web(input: &str) -> Result<ResolvedOpenTarget, OpenTargetError> {
    let probe = Url::parse(&format!("https://{input}")).map_err(|err| {
        OpenTargetError::new(
            "invalid_open_target",
            format!("Open Target is neither a valid local path nor a valid web address: {err}"),
            json!({ "open_target": input }),
        )
    })?;
    let host = probe.host().ok_or_else(|| {
        OpenTargetError::new(
            "invalid_open_target",
            "Open Target is missing a web host",
            json!({ "open_target": input }),
        )
    })?;
    let scheme = match explicit_port(input) {
        Some(80) => "http",
        Some(443) => "https",
        _ if host_defaults_to_http(host) => "http",
        _ => "https",
    };
    Ok(ResolvedOpenTarget {
        navigation_url: format!("{scheme}://{input}"),
        kind: TargetKind::Web,
    })
}

fn host_defaults_to_http(host: Host<&str>) -> bool {
    match host {
        Host::Domain(domain) => {
            let lower = domain.to_ascii_lowercase();
            lower == "localhost"
                || lower.ends_with(".localhost")
                || lower.ends_with(".local")
                || !lower.contains('.')
        }
        Host::Ipv4(ip) => {
            ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified()
        }
        Host::Ipv6(ip) => {
            ip.is_loopback()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || ip.is_unspecified()
        }
    }
}

fn resolve_file_url(input: &str) -> Result<ResolvedOpenTarget, OpenTargetError> {
    if !input.to_ascii_lowercase().starts_with("file://") {
        return Err(OpenTargetError::new(
            "invalid_file_url",
            "Relative file URLs are not supported; pass a relative filesystem path instead",
            json!({ "open_target": input }),
        ));
    }
    let mut parsed = Url::parse(input).map_err(|err| {
        OpenTargetError::new(
            "invalid_file_url",
            format!("Invalid file URL: {err}"),
            json!({ "open_target": input }),
        )
    })?;
    if !parsed.username().is_empty() || parsed.password().is_some() || parsed.port().is_some() {
        return Err(OpenTargetError::new(
            "invalid_file_url",
            "File URL must not contain credentials or a port",
            json!({ "open_target": input }),
        ));
    }

    let host = parsed.host_str().unwrap_or_default().to_string();
    #[cfg(not(windows))]
    if !host.is_empty() && !host.eq_ignore_ascii_case("localhost") {
        return Err(OpenTargetError::new(
            "unsupported_file_host",
            format!("File URL host is not supported on this platform: {host}"),
            json!({ "open_target": input, "host": host, "host_platform": std::env::consts::OS }),
        ));
    }
    if host.eq_ignore_ascii_case("localhost") {
        parsed.set_host(None).map_err(|_| {
            OpenTargetError::new(
                "invalid_file_url",
                "Could not normalize localhost file URL",
                json!({ "open_target": input }),
            )
        })?;
    }

    let query = parsed.query().map(str::to_string);
    let fragment = parsed.fragment().map(str::to_string);
    parsed.set_query(None);
    parsed.set_fragment(None);
    let path = parsed.to_file_path().map_err(|_| {
        OpenTargetError::new(
            "invalid_file_url",
            "File URL must be absolute and map to a path on this platform",
            json!({ "open_target": input }),
        )
    })?;
    if !path.is_absolute() {
        return Err(OpenTargetError::new(
            "invalid_file_url",
            "Relative file URLs are not supported; pass a relative filesystem path instead",
            json!({ "open_target": input }),
        ));
    }
    resolve_local_path(path, input, query.as_deref(), fragment.as_deref())
}

fn resolve_path_input(
    input: &str,
    cwd: Option<&Path>,
) -> Result<ResolvedOpenTarget, OpenTargetError> {
    let path = expand_current_user_home(input)?;
    let absolute = if path.is_absolute() {
        path
    } else {
        let cwd = validate_cwd(cwd, input)?;
        cwd.join(path)
    }
    .clean();
    resolve_local_path(absolute, input, None, None)
}

fn validate_cwd<'a>(cwd: Option<&'a Path>, input: &str) -> Result<&'a Path, OpenTargetError> {
    let cwd = cwd.ok_or_else(|| {
        OpenTargetError::new(
            "missing_cwd",
            "A working directory is required for a relative Open Target",
            json!({ "open_target": input }),
        )
    })?;
    if !cwd.is_absolute() || !cwd.is_dir() {
        return Err(OpenTargetError::new(
            "invalid_cwd",
            "cwd must be an absolute existing directory",
            json!({ "open_target": input, "cwd": cwd }),
        ));
    }
    Ok(cwd)
}

fn resolve_local_path(
    path: PathBuf,
    input: &str,
    query: Option<&str>,
    fragment: Option<&str>,
) -> Result<ResolvedOpenTarget, OpenTargetError> {
    if !path.exists() {
        return Err(OpenTargetError::new(
            "local_resource_not_found",
            format!("Local Resource does not exist: {}", path.display()),
            json!({ "open_target": input, "resolved_path": path }),
        ));
    }
    let canonical = dunce::canonicalize(&path).map_err(|err| {
        OpenTargetError::new(
            "local_resource_not_found",
            format!("Could not resolve Local Resource {}: {err}", path.display()),
            json!({ "open_target": input, "resolved_path": path }),
        )
    })?;
    if canonical.to_str().is_none() {
        return Err(OpenTargetError::new(
            "non_utf8_local_resource",
            "Local Resource path cannot be represented as UTF-8",
            json!({ "open_target": input }),
        ));
    }
    let metadata = fs::metadata(&canonical).map_err(|err| {
        OpenTargetError::new(
            "local_resource_not_found",
            format!(
                "Could not inspect Local Resource {}: {err}",
                canonical.display()
            ),
            json!({ "open_target": input, "resolved_path": canonical }),
        )
    })?;
    if !metadata.is_file() {
        return Err(OpenTargetError::new(
            "local_resource_not_file",
            format!(
                "Local Resource must be a regular file: {}",
                canonical.display()
            ),
            json!({
                "open_target": input,
                "resolved_path": canonical,
                "actual_type": if metadata.is_dir() { "directory" } else { "non_regular" },
            }),
        ));
    }
    let mut url = Url::from_file_path(&canonical).map_err(|_| {
        OpenTargetError::new(
            "invalid_file_url",
            "Could not convert Local Resource to a file URL",
            json!({ "open_target": input, "resolved_path": canonical }),
        )
    })?;
    url.set_query(query);
    url.set_fragment(fragment);
    Ok(ResolvedOpenTarget {
        navigation_url: url.to_string(),
        kind: TargetKind::LocalResource,
    })
}

fn expand_current_user_home(input: &str) -> Result<PathBuf, OpenTargetError> {
    if input == "~" || input.starts_with("~/") || (cfg!(windows) && input.starts_with("~\\")) {
        let home = dirs::home_dir().ok_or_else(|| {
            OpenTargetError::new(
                "home_directory_unavailable",
                "Could not determine the current user's home directory",
                json!({ "open_target": input }),
            )
        })?;
        if input == "~" {
            return Ok(home);
        }
        return Ok(home.join(&input[2..]));
    }
    Ok(PathBuf::from(input))
}

fn explicit_scheme(input: &str) -> Option<String> {
    let (scheme, _) = input.split_once(':')?;
    let mut chars = scheme.chars();
    if !chars.next()?.is_ascii_alphabetic()
        || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return None;
    }
    Some(scheme.to_ascii_lowercase())
}

fn looks_like_host_port(input: &str) -> bool {
    explicit_port(input).is_some()
}

fn explicit_port(input: &str) -> Option<u16> {
    let authority = input.split(['/', '?', '#']).next()?;
    if authority.starts_with('[') {
        let end = authority.find(']')?;
        return authority.get(end + 1..)?.strip_prefix(':')?.parse().ok();
    }
    let (host, port) = authority.rsplit_once(':')?;
    (!host.is_empty()).then(|| port.parse().ok()).flatten()
}

fn is_explicit_path(input: &str) -> bool {
    input == "~"
        || input.starts_with("~/")
        || input.starts_with("./")
        || input.starts_with("../")
        || input.starts_with(".\\")
        || input.starts_with("..\\")
        || Path::new(input).is_absolute()
}

#[cfg(not(windows))]
fn foreign_path_platform(input: &str) -> Option<&'static str> {
    let bytes = input.as_bytes();
    let drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/');
    let unc = input.starts_with("\\\\");
    let relative = input.starts_with(".\\") || input.starts_with("..\\");
    let home = input.starts_with("~\\");
    (drive || unc || relative || home).then_some("windows")
}

#[cfg(windows)]
fn foreign_path_platform(input: &str) -> Option<&'static str> {
    (input.starts_with("/Users/") || input.starts_with("/home/") || input.starts_with("/tmp/"))
        .then_some("unix")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "agent-browser-open-target-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn existing_bare_path_wins_over_web() {
        let dir = temp_dir();
        fs::write(dir.join("example.com"), "ok").unwrap();
        let resolved = resolve_open_target("example.com", Some(&dir)).unwrap();
        assert_eq!(resolved.kind, TargetKind::LocalResource);
        assert!(resolved.navigation_url.starts_with("file://"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_bare_file_shape_falls_back_to_web() {
        let dir = temp_dir();
        let resolved = resolve_open_target("demo.html", Some(&dir)).unwrap();
        assert_eq!(resolved.navigation_url, "https://demo.html");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn explicit_missing_path_is_an_error() {
        let dir = temp_dir();
        let err = resolve_open_target("./missing.html", Some(&dir)).unwrap_err();
        assert_eq!(err.code, "local_resource_not_found");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn directories_are_rejected() {
        let dir = temp_dir();
        let err = resolve_open_target(dir.to_str().unwrap(), None).unwrap_err();
        assert_eq!(err.code, "local_resource_not_file");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn explicit_web_url_is_preserved() {
        let input = "HTTPS://EXAMPLE.com:443/a/../b?q=a%2Fb";
        let resolved = resolve_open_target(input, None).unwrap();
        assert_eq!(resolved.navigation_url, input);
    }

    #[test]
    fn local_hosts_default_to_http_and_public_hosts_to_https() {
        assert_eq!(
            resolve_open_target("localhost:3000", None)
                .unwrap()
                .navigation_url,
            "http://localhost:3000"
        );
        assert_eq!(
            resolve_open_target("printer.local", None)
                .unwrap()
                .navigation_url,
            "http://printer.local"
        );
        assert_eq!(
            resolve_open_target("example.com", None)
                .unwrap()
                .navigation_url,
            "https://example.com"
        );
    }

    #[test]
    fn standard_ports_override_host_defaults() {
        assert_eq!(
            resolve_open_target("example.com:80", None)
                .unwrap()
                .navigation_url,
            "http://example.com:80"
        );
        assert_eq!(
            resolve_open_target("localhost:443", None)
                .unwrap()
                .navigation_url,
            "https://localhost:443"
        );
    }

    #[test]
    fn unsupported_schemes_are_rejected() {
        let err = resolve_open_target("mailto:user@example.com", None).unwrap_err();
        assert_eq!(err.code, "unsupported_scheme");
    }

    #[cfg(unix)]
    #[test]
    fn path_hash_and_question_mark_are_filename_characters() {
        let dir = temp_dir();
        let filename = "report#1?draft.html";
        fs::write(dir.join(filename), "ok").unwrap();
        let resolved = resolve_open_target(&format!("./{filename}"), Some(&dir)).unwrap();
        assert!(resolved.navigation_url.contains("report%231%3Fdraft.html"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn relative_file_urls_are_rejected() {
        let err = resolve_open_target("file:demo.html", None).unwrap_err();
        assert_eq!(err.code, "invalid_file_url");
    }

    #[test]
    fn relative_paths_require_a_valid_absolute_cwd() {
        let err = resolve_open_target("./demo.html", None).unwrap_err();
        assert_eq!(err.code, "missing_cwd");
        let err = resolve_open_target("./demo.html", Some(Path::new("relative"))).unwrap_err();
        assert_eq!(err.code, "invalid_cwd");
        let err =
            resolve_open_target("https://example.com", Some(Path::new("relative"))).unwrap_err();
        assert_eq!(err.code, "invalid_cwd");
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_links_resolve_to_the_real_file() {
        use std::os::unix::fs::symlink;

        let dir = temp_dir();
        let real = dir.join("real.html");
        let link = dir.join("current.html");
        fs::write(&real, "ok").unwrap();
        symlink(&real, &link).unwrap();
        let resolved = resolve_open_target(link.to_str().unwrap(), None).unwrap();
        assert_eq!(
            Url::parse(&resolved.navigation_url)
                .unwrap()
                .to_file_path()
                .unwrap(),
            dunce::canonicalize(real).unwrap()
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn file_url_preserves_query_and_fragment() {
        let dir = temp_dir();
        let file = dir.join("demo.html");
        fs::write(&file, "ok").unwrap();
        let mut url = Url::from_file_path(&file).unwrap();
        url.set_query(Some("theme=dark"));
        url.set_fragment(Some("intro"));
        let resolved = resolve_open_target(url.as_str(), None).unwrap();
        assert!(resolved.navigation_url.ends_with("?theme=dark#intro"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(not(windows))]
    #[test]
    fn windows_paths_are_rejected_on_unix() {
        for input in [
            r"C:\Users\Alice\demo.html",
            r"\\server\share\demo.html",
            r".\demo.html",
            r"~\demo.html",
        ] {
            let err = resolve_open_target(input, None).unwrap_err();
            assert_eq!(err.code, "foreign_platform_path", "input: {input}");
        }
    }
}
