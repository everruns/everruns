//! Package registry fetcher
//!
//! Handles PyPI, crates.io, and npm URLs, returning structured package
//! metadata optimized for dependency evaluation by agents.

use crate::client::FetchOptions;
use crate::error::FetchError;
use crate::fetchers::default::{
    read_body_with_timeout, send_request_following_redirects, BODY_TIMEOUT, DEFAULT_MAX_BODY_SIZE,
};
use crate::fetchers::Fetcher;
use crate::types::{FetchRequest, FetchResponse};
use crate::DEFAULT_USER_AGENT;
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT};
use serde::Deserialize;
use std::time::Duration;
use url::Url;

const API_TIMEOUT: Duration = Duration::from_secs(10);

/// Which registry a URL belongs to
#[derive(Debug, Clone, PartialEq)]
enum Registry {
    PyPI {
        name: String,
        version: Option<String>,
    },
    CratesIo {
        name: String,
    },
    Npm {
        name: String,
    },
}

/// Package registry fetcher
///
/// Matches `pypi.org/project/{name}`, `crates.io/crates/{name}`,
/// and `npmjs.com/package/{name}`.
pub struct PackageRegistryFetcher;

impl PackageRegistryFetcher {
    pub fn new() -> Self {
        Self
    }

    fn parse_url(url: &Url) -> Option<Registry> {
        let host = url.host_str()?;
        let segments: Vec<&str> = url.path_segments().map(|s| s.collect()).unwrap_or_default();

        match host {
            "pypi.org" | "www.pypi.org" => {
                // /project/{name} or /project/{name}/{version}
                if segments.len() >= 2 && segments[0] == "project" && !segments[1].is_empty() {
                    let name = segments[1].to_string();
                    let version = segments
                        .get(2)
                        .filter(|v| !v.is_empty())
                        .map(|v| v.to_string());
                    Some(Registry::PyPI { name, version })
                } else {
                    None
                }
            }
            "crates.io" | "www.crates.io" => {
                // /crates/{name}
                if segments.len() >= 2 && segments[0] == "crates" && !segments[1].is_empty() {
                    Some(Registry::CratesIo {
                        name: segments[1].to_string(),
                    })
                } else {
                    None
                }
            }
            "www.npmjs.com" | "npmjs.com" => {
                // /package/{name} or /package/@scope/name
                if segments.len() >= 2 && segments[0] == "package" && !segments[1].is_empty() {
                    let name = if segments[1].starts_with('@') && segments.len() >= 3 {
                        format!("{}/{}", segments[1], segments[2])
                    } else {
                        segments[1].to_string()
                    };
                    Some(Registry::Npm { name })
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

impl Default for PackageRegistryFetcher {
    fn default() -> Self {
        Self::new()
    }
}

// --- PyPI API types ---

#[derive(Debug, Deserialize)]
struct PyPIResponse {
    info: PyPIInfo,
}

#[derive(Debug, Deserialize)]
struct PyPIInfo {
    name: String,
    version: String,
    summary: Option<String>,
    license: Option<String>,
    author: Option<String>,
    requires_python: Option<String>,
    requires_dist: Option<Vec<String>>,
    home_page: Option<String>,
}

// --- crates.io API types ---

#[derive(Debug, Deserialize)]
struct CratesIoResponse {
    #[serde(rename = "crate")]
    krate: CratesIoCrate,
}

#[derive(Debug, Deserialize)]
struct CratesIoCrate {
    name: String,
    max_version: String,
    description: Option<String>,
    license: Option<String>,
    downloads: u64,
    repository: Option<String>,
    categories: Option<Vec<String>>,
    keywords: Option<Vec<String>>,
}

// --- npm API types ---

#[derive(Debug, Deserialize)]
struct NpmResponse {
    name: String,
    description: Option<String>,
    #[serde(rename = "dist-tags")]
    dist_tags: Option<NpmDistTags>,
    license: Option<serde_json::Value>,
    repository: Option<NpmRepository>,
}

#[derive(Debug, Deserialize)]
struct NpmDistTags {
    latest: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NpmRepository {
    url: Option<String>,
}

#[async_trait]
impl Fetcher for PackageRegistryFetcher {
    fn name(&self) -> &'static str {
        "package_registry"
    }

    fn matches(&self, url: &Url) -> bool {
        Self::parse_url(url).is_some()
    }

    async fn fetch(
        &self,
        request: &FetchRequest,
        options: &FetchOptions,
    ) -> Result<FetchResponse, FetchError> {
        let url = Url::parse(&request.url).map_err(|_| FetchError::InvalidUrlScheme)?;

        let registry = Self::parse_url(&url).ok_or_else(|| {
            FetchError::FetcherError("Not a valid package registry URL".to_string())
        })?;

        let user_agent = options.user_agent.as_deref().unwrap_or(DEFAULT_USER_AGENT);
        let ua_header = HeaderValue::from_str(user_agent)
            .unwrap_or_else(|_| HeaderValue::from_static(DEFAULT_USER_AGENT));
        let max_body_size = options.max_body_size.unwrap_or(DEFAULT_MAX_BODY_SIZE);

        let content = match registry {
            Registry::PyPI { name, version } => {
                fetch_pypi(
                    options,
                    &ua_header,
                    &name,
                    version.as_deref(),
                    max_body_size,
                )
                .await?
            }
            Registry::CratesIo { name } => {
                fetch_crates_io(options, &ua_header, &name, max_body_size).await?
            }
            Registry::Npm { name } => fetch_npm(options, &ua_header, &name, max_body_size).await?,
        };

        Ok(FetchResponse {
            url: request.url.clone(),
            status_code: 200,
            content_type: Some("text/markdown".to_string()),
            format: Some("package_registry".to_string()),
            content: Some(content),
            ..Default::default()
        })
    }
}

/// Issue a single registry API GET through the configured transport, with manual
/// per-hop redirect validation (TM-SSRF-010). Returns the capped body bytes.
async fn registry_get(
    options: &FetchOptions,
    ua: &HeaderValue,
    accept: Option<HeaderValue>,
    api_url: &str,
    max_body_size: usize,
    api_name: &str,
) -> Result<bytes::Bytes, FetchError> {
    let parsed = Url::parse(api_url).map_err(|_| FetchError::InvalidUrlScheme)?;
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, ua.clone());
    if let Some(accept) = accept {
        headers.insert(ACCEPT, accept);
    }

    let (resp, _redirect_chain) = send_request_following_redirects(
        parsed,
        reqwest::Method::GET,
        headers,
        options,
        API_TIMEOUT,
    )
    .await?;

    if !(200..300).contains(&resp.status) {
        return Err(FetchError::FetcherError(format!(
            "{} API error: HTTP {}",
            api_name, resp.status
        )));
    }

    let (body, _) = read_body_with_timeout(resp, BODY_TIMEOUT, max_body_size).await?;
    Ok(body)
}

async fn fetch_pypi(
    options: &FetchOptions,
    ua: &HeaderValue,
    name: &str,
    version: Option<&str>,
    max_body_size: usize,
) -> Result<String, FetchError> {
    let api_url = match version {
        Some(v) => format!("https://pypi.org/pypi/{}/{}/json", name, v),
        None => format!("https://pypi.org/pypi/{}/json", name),
    };

    let body = registry_get(options, ua, None, &api_url, max_body_size, "PyPI").await?;
    let data: PyPIResponse = serde_json::from_slice(&body)
        .map_err(|e| FetchError::FetcherError(format!("Failed to parse PyPI data: {}", e)))?;

    let info = &data.info;
    let mut out = String::new();
    out.push_str(&format!("# {} (PyPI)\n\n", info.name));
    out.push_str(&format!("- **Version:** {}\n", info.version));

    if let Some(license) = &info.license {
        if !license.is_empty() {
            out.push_str(&format!("- **License:** {}\n", license));
        }
    }
    if let Some(summary) = &info.summary {
        out.push_str(&format!("- **Summary:** {}\n", summary));
    }
    if let Some(author) = &info.author {
        if !author.is_empty() {
            out.push_str(&format!("- **Author:** {}\n", author));
        }
    }
    if let Some(python) = &info.requires_python {
        out.push_str(&format!("- **Python:** {}\n", python));
    }
    if let Some(home) = &info.home_page {
        if !home.is_empty() {
            out.push_str(&format!("- **Homepage:** {}\n", home));
        }
    }
    out.push_str(&format!(
        "- **URL:** https://pypi.org/project/{}/\n",
        info.name
    ));

    if let Some(deps) = &info.requires_dist {
        if !deps.is_empty() {
            out.push_str(&format!("\n## Dependencies ({})\n\n", deps.len()));
            for dep in deps.iter().take(50) {
                out.push_str(&format!("- {}\n", dep));
            }
            if deps.len() > 50 {
                out.push_str(&format!("\n...and {} more\n", deps.len() - 50));
            }
        }
    }

    Ok(out)
}

async fn fetch_crates_io(
    options: &FetchOptions,
    ua: &HeaderValue,
    name: &str,
    max_body_size: usize,
) -> Result<String, FetchError> {
    let api_url = format!("https://crates.io/api/v1/crates/{}", name);

    let body = registry_get(
        options,
        ua,
        Some(HeaderValue::from_static("application/json")),
        &api_url,
        max_body_size,
        "crates.io",
    )
    .await?;
    let data: CratesIoResponse = serde_json::from_slice(&body)
        .map_err(|e| FetchError::FetcherError(format!("Failed to parse crates.io data: {}", e)))?;

    let krate = &data.krate;
    let mut out = String::new();
    out.push_str(&format!("# {} (crates.io)\n\n", krate.name));
    out.push_str(&format!("- **Version:** {}\n", krate.max_version));

    if let Some(license) = &krate.license {
        out.push_str(&format!("- **License:** {}\n", license));
    }
    if let Some(desc) = &krate.description {
        out.push_str(&format!("- **Description:** {}\n", desc));
    }
    out.push_str(&format!("- **Downloads:** {}\n", krate.downloads));
    if let Some(repo) = &krate.repository {
        out.push_str(&format!("- **Repository:** {}\n", repo));
    }
    out.push_str(&format!(
        "- **URL:** https://crates.io/crates/{}\n",
        krate.name
    ));

    if let Some(keywords) = &krate.keywords {
        if !keywords.is_empty() {
            out.push_str(&format!("- **Keywords:** {}\n", keywords.join(", ")));
        }
    }
    if let Some(categories) = &krate.categories {
        if !categories.is_empty() {
            out.push_str(&format!("- **Categories:** {}\n", categories.join(", ")));
        }
    }

    Ok(out)
}

async fn fetch_npm(
    options: &FetchOptions,
    ua: &HeaderValue,
    name: &str,
    max_body_size: usize,
) -> Result<String, FetchError> {
    let api_url = format!("https://registry.npmjs.org/{}", name);

    let body = registry_get(
        options,
        ua,
        Some(HeaderValue::from_static("application/json")),
        &api_url,
        max_body_size,
        "npm",
    )
    .await?;
    let data: NpmResponse = serde_json::from_slice(&body)
        .map_err(|e| FetchError::FetcherError(format!("Failed to parse npm data: {}", e)))?;

    let mut out = String::new();
    out.push_str(&format!("# {} (npm)\n\n", data.name));

    if let Some(tags) = &data.dist_tags {
        if let Some(latest) = &tags.latest {
            out.push_str(&format!("- **Version:** {}\n", latest));
        }
    }

    // License can be a string or object
    if let Some(license) = &data.license {
        let license_str = match license {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Object(o) => o
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
            _ => String::new(),
        };
        if !license_str.is_empty() {
            out.push_str(&format!("- **License:** {}\n", license_str));
        }
    }

    if let Some(desc) = &data.description {
        out.push_str(&format!("- **Description:** {}\n", desc));
    }
    if let Some(repo) = &data.repository {
        if let Some(url) = &repo.url {
            out.push_str(&format!("- **Repository:** {}\n", url));
        }
    }
    out.push_str(&format!(
        "- **URL:** https://www.npmjs.com/package/{}\n",
        data.name
    ));

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_pypi_url() {
        let url = Url::parse("https://pypi.org/project/requests").unwrap();
        assert_eq!(
            PackageRegistryFetcher::parse_url(&url),
            Some(Registry::PyPI {
                name: "requests".to_string(),
                version: None
            })
        );
    }

    #[test]
    fn test_parse_pypi_url_with_version() {
        let url = Url::parse("https://pypi.org/project/requests/2.31.0").unwrap();
        assert_eq!(
            PackageRegistryFetcher::parse_url(&url),
            Some(Registry::PyPI {
                name: "requests".to_string(),
                version: Some("2.31.0".to_string())
            })
        );
    }

    #[test]
    fn test_parse_crates_io_url() {
        let url = Url::parse("https://crates.io/crates/serde").unwrap();
        assert_eq!(
            PackageRegistryFetcher::parse_url(&url),
            Some(Registry::CratesIo {
                name: "serde".to_string()
            })
        );
    }

    #[test]
    fn test_parse_npm_url() {
        let url = Url::parse("https://www.npmjs.com/package/express").unwrap();
        assert_eq!(
            PackageRegistryFetcher::parse_url(&url),
            Some(Registry::Npm {
                name: "express".to_string()
            })
        );
    }

    #[test]
    fn test_parse_npm_scoped_url() {
        let url = Url::parse("https://www.npmjs.com/package/@types/node").unwrap();
        assert_eq!(
            PackageRegistryFetcher::parse_url(&url),
            Some(Registry::Npm {
                name: "@types/node".to_string()
            })
        );
    }

    #[test]
    fn test_rejects_non_registry() {
        let url = Url::parse("https://example.com/project/foo").unwrap();
        assert_eq!(PackageRegistryFetcher::parse_url(&url), None);
    }

    #[test]
    fn test_rejects_non_package_paths() {
        let url = Url::parse("https://pypi.org/search/").unwrap();
        assert_eq!(PackageRegistryFetcher::parse_url(&url), None);

        let url = Url::parse("https://crates.io/categories").unwrap();
        assert_eq!(PackageRegistryFetcher::parse_url(&url), None);

        let url = Url::parse("https://www.npmjs.com/settings").unwrap();
        assert_eq!(PackageRegistryFetcher::parse_url(&url), None);
    }

    #[test]
    fn test_fetcher_matches() {
        let fetcher = PackageRegistryFetcher::new();

        let url = Url::parse("https://pypi.org/project/requests").unwrap();
        assert!(fetcher.matches(&url));

        let url = Url::parse("https://crates.io/crates/serde").unwrap();
        assert!(fetcher.matches(&url));

        let url = Url::parse("https://www.npmjs.com/package/express").unwrap();
        assert!(fetcher.matches(&url));

        let url = Url::parse("https://github.com/owner/repo").unwrap();
        assert!(!fetcher.matches(&url));
    }
}
