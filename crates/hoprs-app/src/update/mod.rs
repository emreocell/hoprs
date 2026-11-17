//! Update notice: versions, release JSON, package choice and checksums (pure logic here
//! and in `package` and `sums`), the HTTPS client (`http`) and the verified download
//! (`download`). The UI lives elsewhere.

use std::time::Duration;

use hoprs_core::{Error, ErrorKind};
use semver::Version;

pub mod download;
pub mod http;
pub mod package;
pub mod sums;

/// The latest stable release, as GitHub's API describes it.
pub const API_URL: &str = "https://api.github.com/repos/engviniciusbs/hoprs/releases/latest";
/// The release asset that lists every other asset's SHA-256.
pub const SUMS_NAME: &str = "SHA256SUMS";
/// How long after the settings load the first automatic check waits.
pub const START_DELAY: Duration = Duration::from_secs(5);
/// How often the app checks again while it runs.
pub const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// A file attached to a release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

/// A published, stable release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    /// The release's page on GitHub.
    pub page: String,
    pub assets: Vec<Asset>,
}

impl Release {
    pub fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|asset| asset.name == name)
    }
}

/// `v1.4.0` or `1.4.0`. Pre-releases and build metadata are not versions we offer.
pub fn parse_version(tag: &str) -> Option<Version> {
    let version = Version::parse(tag.strip_prefix('v').unwrap_or(tag)).ok()?;
    (version.pre.is_empty() && version.build.is_empty()).then_some(version)
}

pub fn is_newer(release: &Version, running: &Version) -> bool {
    release > running
}

fn bad(detail: impl Into<String>) -> Error {
    Error::coded(
        ErrorKind::Protocol,
        "update-bad-response",
        "The update server gave an unexpected answer",
    )
    .with_detail(detail)
}

/// An `https://github.com/...` URL: the only host a release page or asset is listed on.
fn https(url: &str) -> bool {
    url.strip_prefix("https://")
        .is_some_and(|rest| rest.split(['/', '?', '#']).next() == Some("github.com"))
}

/// The release in GitHub's `releases/latest` JSON. Anything unexpected is an error: a
/// missing field, a draft or pre-release, an unparsable tag, a URL that is not HTTPS.
pub fn parse_release(json: &[u8]) -> Result<Release, Error> {
    let value: serde_json::Value =
        serde_json::from_slice(json).map_err(|err| bad(err.to_string()))?;
    let text = |value: &serde_json::Value, key: &str| -> Result<String, Error> {
        value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| bad(format!("`{key}` missing or not a string")))
    };
    let flag = |key: &str| -> Result<bool, Error> {
        value
            .get(key)
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(|| bad(format!("`{key}` missing or not a bool")))
    };
    if flag("draft")? || flag("prerelease")? {
        return Err(bad("a draft or pre-release"));
    }
    let tag = text(&value, "tag_name")?;
    let version =
        parse_version(&tag).ok_or_else(|| bad(format!("tag `{tag}` is not a version")))?;
    let page = text(&value, "html_url")?;
    if !https(&page) {
        return Err(bad("`html_url` is not an https://github.com URL"));
    }
    let list = value
        .get("assets")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| bad("`assets` missing or not an array"))?;
    let mut assets = Vec::with_capacity(list.len());
    for item in list {
        let url = text(item, "browser_download_url")?;
        if !https(&url) {
            return Err(bad("an asset URL is not an https://github.com URL"));
        }
        assets.push(Asset {
            name: text(item, "name")?,
            url,
            size: item
                .get("size")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| bad("an asset's `size` is missing or not a number"))?,
        });
    }
    Ok(Release {
        version,
        page,
        assets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn versions_compare_by_semver() {
        assert!(is_newer(&v("1.4.0"), &v("1.3.1")));
        assert!(is_newer(&v("1.10.0"), &v("1.9.0")));
        assert!(!is_newer(&v("1.3.1"), &v("1.3.1")));
        assert!(!is_newer(&v("1.3.0"), &v("1.3.1")));
        assert_eq!(parse_version("v1.4.0"), Some(v("1.4.0")));
        assert_eq!(parse_version("1.4.0"), Some(v("1.4.0")));
        for bad in ["v1.5.0-rc.1", "1.4", "latest", "", "v1.4.0+build"] {
            assert_eq!(parse_version(bad), None, "{bad}");
        }
    }

    const FIXTURE: &str = include_str!("fixtures/latest-release.json");

    #[test]
    fn release_json_parses() {
        let release = parse_release(FIXTURE.as_bytes()).unwrap();
        assert_eq!(release.version, v("1.3.1"));
        assert_eq!(
            release.page,
            "https://github.com/engviniciusbs/hoprs/releases/tag/v1.3.1"
        );
        assert_eq!(release.assets.len(), 5);
        let deb = release.asset("hoprs_1.3.1-1_amd64.deb").unwrap();
        assert_eq!(
            deb.url,
            "https://github.com/engviniciusbs/hoprs/releases/download/v1.3.1/hoprs_1.3.1-1_amd64.deb"
        );
        assert_eq!(deb.size, 10_669_244);
        assert!(release.asset(SUMS_NAME).is_some());
        assert!(release.asset("nope").is_none());
    }

    #[test]
    fn release_json_missing_fields_fails() {
        let mut bodies: Vec<String> = Vec::new();
        let edit = |f: &dyn Fn(&mut serde_json::Value)| {
            let mut value: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
            f(&mut value);
            value.to_string()
        };
        bodies.push(edit(&|v| {
            v.as_object_mut().unwrap().remove("tag_name");
        }));
        bodies.push(edit(&|v| v["assets"] = serde_json::json!("x")));
        bodies.push(edit(&|v| v["draft"] = serde_json::json!(true)));
        bodies.push(edit(&|v| v["prerelease"] = serde_json::json!(true)));
        bodies.push(edit(&|v| v["tag_name"] = serde_json::json!("v1.5.0-rc.1")));
        bodies.push(edit(&|v| {
            v["html_url"] = serde_json::json!("http://github.com/x");
        }));
        bodies.push(edit(&|v| {
            v["assets"][0]["browser_download_url"] = serde_json::json!("http://x/y");
        }));
        bodies.push(edit(&|v| v["assets"][0]["size"] = serde_json::json!("big")));
        bodies.push(edit(&|v| {
            v["assets"][0]["browser_download_url"] =
                serde_json::json!("https://evil.example/a.deb");
        }));
        bodies.push(edit(&|v| {
            v["html_url"] = serde_json::json!("https://github.com.evil.example/x");
        }));
        bodies.push(edit(&|v| {
            v["html_url"] = serde_json::json!("https://user@github.com/x");
        }));
        bodies.push("<html>rate limited</html>".to_owned());
        bodies.push(String::new());
        for body in bodies {
            let err = parse_release(body.as_bytes()).unwrap_err();
            assert_eq!(err.code, Some("update-bad-response"), "{body}");
        }
    }
}
