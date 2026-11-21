use crate::error::{OmvixError, Result};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fmt;
use std::path::PathBuf;

pub const GITHUB_OWNER: &str = "Borab55";
pub const GITHUB_REPO: &str = "hp-omen-hub-lite";
pub const GITHUB_RELEASES_LATEST_API: &str =
    "https://api.github.com/repos/Borab55/hp-omen-hub-lite/releases/latest";
pub const GITHUB_RELEASES_PAGE_URL: &str =
    "https://github.com/Borab55/hp-omen-hub-lite/releases";

pub const MAX_API_JSON_BYTES: usize = 1024 * 1024;
pub const MAX_CHECKSUM_BYTES: usize = 256 * 1024;
pub const MAX_UPDATE_BINARY_BYTES: u64 = 100 * 1024 * 1024;

/// Optional future Authenticode signer Subject pin. When `Some`, any `Valid` Authenticode
/// certificate whose `Subject` differs is rejected.
pub const PINNED_SIGNER_SUBJECT: Option<&str> = None;

/// Pure-Rust FIPS 180-4 SHA-256 digest (lowercase hex), avoiding external dependencies or DLLs.
pub fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity(data.len() + 64);
    padded.extend_from_slice(data);
    padded.push(0x80);
    while (padded.len() % 64) != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in padded.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        let delta = [a, b, c, d, e, f, g, hh];
        for (slot, val) in h.iter_mut().zip(delta) {
            *slot = slot.wrapping_add(val);
        }
    }

    let mut hex = String::with_capacity(64);
    for word in h {
        use std::fmt::Write;
        let _ = write!(hex, "{word:08x}");
    }
    hex
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemVer {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub prerelease: Option<String>,
}

impl SemVer {
    pub fn parse(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        let no_prefix = trimmed
            .strip_prefix('v')
            .or_else(|| trimmed.strip_prefix('V'))
            .unwrap_or(trimmed);
        let no_build = no_prefix.split('+').next()?.trim();
        if no_build.is_empty() {
            return None;
        }
        let (core, prerelease) = match no_build.split_once('-') {
            Some((c, p)) => {
                let p = p.trim();
                if p.is_empty() || p.split('.').any(|seg| seg.is_empty()) {
                    return None;
                }
                (c, Some(p.to_string()))
            }
            None => (no_build, None),
        };
        let mut parts = core.split('.');
        let major = parts.next()?.parse::<u64>().ok()?;
        let minor = parts.next()?.parse::<u64>().ok()?;
        let patch = parts.next()?.parse::<u64>().ok()?;
        if parts.next().is_some() {
            return None;
        }
        Some(Self {
            major,
            minor,
            patch,
            prerelease,
        })
    }
}

impl fmt::Display for SemVer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.prerelease {
            Some(pre) => write!(f, "{}.{}.{}-{pre}", self.major, self.minor, self.patch),
            None => write!(f, "{}.{}.{}", self.major, self.minor, self.patch),
        }
    }
}

fn compare_prerelease(a: &str, b: &str) -> Ordering {
    let mut it_a = a.split('.');
    let mut it_b = b.split('.');
    loop {
        match (it_a.next(), it_b.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(sa), Some(sb)) => {
                let ord = match (sa.parse::<u64>(), sb.parse::<u64>()) {
                    (Ok(na), Ok(nb)) => na.cmp(&nb),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => sa.cmp(sb),
                };
                if ord != Ordering::Equal {
                    return ord;
                }
            }
        }
    }
}

impl PartialOrd for SemVer {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SemVer {
    fn cmp(&self, other: &Self) -> Ordering {
        let base = (self.major, self.minor, self.patch).cmp(&(
            other.major,
            other.minor,
            other.patch,
        ));
        if base != Ordering::Equal {
            return base;
        }
        match (&self.prerelease, &other.prerelease) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Greater,
            (Some(_), None) => Ordering::Less,
            (Some(a), Some(b)) => compare_prerelease(a, b),
        }
    }
}

/// Validates that `url` strictly uses HTTPS and points only to pinned GitHub Release endpoints
/// or official GitHub asset CDN hosts.
pub fn validate_github_release_url(url: &str) -> Result<()> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err(OmvixError::Config("Empty update URL".to_string()));
    }
    if trimmed.chars().any(|c| {
        c.is_whitespace()
            || c.is_control()
            || matches!(c, '"' | '\'' | '`' | ';' | '|' | '&' | '<' | '>')
    }) {
        return Err(OmvixError::Config(format!(
            "Unsafe characters in update URL: {trimmed}"
        )));
    }
    let Some(rest) = trimmed.strip_prefix("https://") else {
        return Err(OmvixError::Config(format!(
            "Update URL must use https:// scheme: {trimmed}"
        )));
    };
    let (authority, path) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, ""),
    };
    if authority.is_empty()
        || authority.contains('@')
        || authority.contains('?')
        || authority.contains('#')
        || authority.contains(':')
    {
        return Err(OmvixError::Config(format!(
            "Invalid host in update URL: {trimmed}"
        )));
    }
    if path.split('/').any(|seg| seg == "..") {
        return Err(OmvixError::Config(format!(
            "Path traversal not allowed in update URL: {trimmed}"
        )));
    }

    let host_lower = authority.to_ascii_lowercase();
    let path_lower = path.to_ascii_lowercase();
    let repo_prefix = format!(
        "/{}/{}/",
        GITHUB_OWNER.to_ascii_lowercase(),
        GITHUB_REPO.to_ascii_lowercase()
    );

    let allowed = match host_lower.as_str() {
        "api.github.com" => {
            path_lower.starts_with(&format!("/repos{repo_prefix}"))
        }
        "github.com" => {
            let releases_exact = format!("{repo_prefix}releases");
            let releases_prefix = format!("{repo_prefix}releases/");
            path_lower == releases_exact || path_lower.starts_with(&releases_prefix)
        }
        "objects.githubusercontent.com"
        | "release-assets.githubusercontent.com"
        | "github-releases.githubusercontent.com" => path.len() > 1,
        _ => false,
    };

    if allowed {
        Ok(())
    } else {
        Err(OmvixError::Config(format!(
            "Update URL outside pinned GitHub repository allowlist: {trimmed}"
        )))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    pub download_url: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseInfo {
    pub tag_name: String,
    pub version: String,
    pub name: String,
    pub published_at: String,
    pub html_url: String,
    pub notes: String,
    pub binary_asset: ReleaseAsset,
    pub checksum_asset: Option<ReleaseAsset>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthenticodeInfo {
    pub status: String,
    pub subject: Option<String>,
    pub issuer: Option<String>,
    pub thumbprint: Option<String>,
    pub not_after: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpdateSignatureStatus {
    AuthenticodeValid {
        subject: String,
        issuer: String,
        thumbprint: String,
    },
    UnsignedSha256Only,
    Rejected(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StagedUpdate {
    pub version: String,
    pub binary_path: PathBuf,
    pub expected_sha256: String,
    pub verified_sha256: String,
    pub signature: UpdateSignatureStatus,
    pub release_url: String,
}

impl StagedUpdate {
    pub fn can_install(&self, allow_unsigned_sha256: bool) -> bool {
        let valid_hash = self.verified_sha256.len() == 64
            && self.expected_sha256.len() == 64
            && self
                .verified_sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
            && self
                .verified_sha256
                .eq_ignore_ascii_case(&self.expected_sha256);
        if !valid_hash {
            return false;
        }
        match &self.signature {
            UpdateSignatureStatus::AuthenticodeValid { .. } => true,
            UpdateSignatureStatus::UnsignedSha256Only => allow_unsigned_sha256,
            UpdateSignatureStatus::Rejected(_) => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum UpdateUiStatus {
    #[default]
    Idle,
    Checking,
    UpToDate {
        checked_version: String,
        checked_at_utc: String,
    },
    NoReleasesPublished,
    UpdateAvailable(ReleaseInfo),
    Downloading {
        version: String,
    },
    ReadyToInstall(StagedUpdate),
    Applying {
        version: String,
    },
    Error(String),
}

#[derive(Debug, Deserialize)]
struct RawRelease {
    #[serde(default)]
    tag_name: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<RawAsset>,
}

#[derive(Debug, Deserialize)]
struct RawAsset {
    #[serde(default)]
    name: String,
    #[serde(default)]
    browser_download_url: String,
    #[serde(default)]
    size: u64,
}

fn is_hub_exe_asset(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("omvix-hub")
        && lower.ends_with(".exe")
        && !lower.contains("cli")
        && !lower.contains("service")
        && !lower.contains("svc")
}

fn is_hub_zip_asset(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("omvix-hub") && lower.ends_with(".zip")
}

pub fn parse_github_release_json(json: &str) -> Result<ReleaseInfo> {
    if json.len() > MAX_API_JSON_BYTES {
        return Err(OmvixError::Config(format!(
            "GitHub release payload exceeds {MAX_API_JSON_BYTES} bytes"
        )));
    }
    let raw: RawRelease = serde_json::from_str(json)
        .map_err(|e| OmvixError::Config(format!("Invalid GitHub release JSON: {e}")))?;

    if raw.draft {
        return Err(OmvixError::Config(
            "Ignoring draft GitHub release".to_string(),
        ));
    }

    let ver = SemVer::parse(&raw.tag_name).ok_or_else(|| {
        OmvixError::Config(format!(
            "Invalid SemVer release tag `{}`",
            raw.tag_name
        ))
    })?;
    let version = ver.to_string();

    let html_url_raw = raw.html_url.unwrap_or_default();
    let html_url = if html_url_raw.trim().is_empty() {
        GITHUB_RELEASES_PAGE_URL.to_string()
    } else {
        validate_github_release_url(html_url_raw.trim())?;
        html_url_raw.trim().to_string()
    };

    let selected_bin = raw
        .assets
        .iter()
        .find(|a| is_hub_exe_asset(&a.name))
        .or_else(|| {
            raw.assets.iter().find(|a| {
                let lower = a.name.to_ascii_lowercase();
                is_hub_zip_asset(&a.name) && (lower.contains("win") || lower.contains("x64"))
            })
        })
        .or_else(|| raw.assets.iter().find(|a| is_hub_zip_asset(&a.name)))
        .ok_or_else(|| {
            OmvixError::Config(
                "Release contains no matching omvix-hub .exe or .zip asset".to_string(),
            )
        })?;

    if selected_bin.size == 0 || selected_bin.size > MAX_UPDATE_BINARY_BYTES {
        return Err(OmvixError::Config(format!(
            "Release binary size {} bytes is out of allowed bounds",
            selected_bin.size
        )));
    }
    validate_github_release_url(&selected_bin.browser_download_url)?;

    let binary_asset = ReleaseAsset {
        name: selected_bin.name.clone(),
        download_url: selected_bin.browser_download_url.clone(),
        size_bytes: selected_bin.size,
    };

    let selected_checksum = raw
        .assets
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case("sha256sums.txt"))
        .or_else(|| raw.assets.iter().find(|a| a.name.eq_ignore_ascii_case("sha256sums")))
        .or_else(|| raw.assets.iter().find(|a| a.name.eq_ignore_ascii_case("checksums.txt")))
        .or_else(|| {
            raw.assets.iter().find(|a| {
                let lower = a.name.to_ascii_lowercase();
                (lower.ends_with(".sha256") || lower.ends_with(".sha256.txt"))
                    && (lower.contains("omvix-hub") || lower.contains("sha256"))
            })
        });

    let checksum_asset = if let Some(cs) = selected_checksum {
        if cs.size > MAX_CHECKSUM_BYTES as u64 {
            return Err(OmvixError::Config(format!(
                "Checksum manifest size {} exceeds limit",
                cs.size
            )));
        }
        validate_github_release_url(&cs.browser_download_url)?;
        Some(ReleaseAsset {
            name: cs.name.clone(),
            download_url: cs.browser_download_url.clone(),
            size_bytes: cs.size,
        })
    } else {
        None
    };

    let display_name = raw
        .name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| format!("v{version}"));
    let published_at = raw
        .published_at
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let notes: String = raw.body.unwrap_or_default().chars().take(4000).collect();

    Ok(ReleaseInfo {
        tag_name: raw.tag_name.trim().to_string(),
        version,
        name: display_name,
        published_at,
        html_url,
        notes,
        binary_asset,
        checksum_asset,
    })
}

fn is_valid_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Parses a `SHA256SUMS.txt` manifest or single-hash `.sha256` file for `target_filename`.
pub fn parse_sha256_manifest(manifest: &str, target_filename: &str) -> Option<String> {
    let target_base = target_filename
        .trim()
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(target_filename)
        .trim();
    if target_base.is_empty() {
        return None;
    }

    let lines: Vec<&str> = manifest
        .lines()
        .map(|l| l.trim_start_matches('\u{FEFF}').trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();

    if lines.len() == 1 {
        let single = lines[0];
        if !single.contains(char::is_whitespace) && is_valid_sha256_hex(single) {
            return Some(single.to_ascii_lowercase());
        }
    }

    for line in lines {
        let Some((hash_part, rest_part)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let hash_clean = hash_part.trim();
        if !is_valid_sha256_hex(hash_clean) {
            continue;
        }
        let mut file_part = rest_part.trim_start();
        if let Some(after_star) = file_part.strip_prefix('*') {
            file_part = after_star.trim_start();
        }
        let file_clean = file_part
            .trim()
            .trim_matches('"')
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(file_part)
            .trim();
        if !file_clean.is_empty() && file_clean.eq_ignore_ascii_case(target_base) {
            return Some(hash_clean.to_ascii_lowercase());
        }
    }

    None
}

pub fn evaluate_authenticode_signature(
    info: &AuthenticodeInfo,
    pinned_subject: Option<&str>,
) -> UpdateSignatureStatus {
    let status = info.status.trim();
    if status.eq_ignore_ascii_case("Valid") {
        let subject = info.subject.as_deref().map(str::trim).unwrap_or("");
        let issuer = info.issuer.as_deref().map(str::trim).unwrap_or("");
        let thumbprint = info.thumbprint.as_deref().map(str::trim).unwrap_or("");
        if subject.is_empty() || issuer.is_empty() {
            return UpdateSignatureStatus::Rejected(
                "Valid Authenticode signature is missing Subject or Issuer metadata".to_string(),
            );
        }
        if let Some(expected_subj) = pinned_subject {
            if subject != expected_subj.trim() {
                return UpdateSignatureStatus::Rejected(format!(
                    "Signer Subject mismatch: expected `{expected_subj}`, got `{subject}`"
                ));
            }
        }
        UpdateSignatureStatus::AuthenticodeValid {
            subject: subject.to_string(),
            issuer: issuer.to_string(),
            thumbprint: thumbprint.to_string(),
        }
    } else if status.eq_ignore_ascii_case("NotSigned") {
        UpdateSignatureStatus::UnsignedSha256Only
    } else {
        UpdateSignatureStatus::Rejected(format!(
            "Authenticode signature rejected: {status}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_nist_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn semver_parsing_and_ordering_work() {
        let v045 = SemVer::parse("0.4.5").unwrap();
        let v050_rc1 = SemVer::parse("v0.5.0-rc.1").unwrap();
        let v050_rc2 = SemVer::parse("V0.5.0-rc.2+build99").unwrap();
        let v050 = SemVer::parse("v0.5.0").unwrap();
        let v051 = SemVer::parse("0.5.1").unwrap();

        assert!(v045 < v050_rc1);
        assert!(v050_rc1 < v050_rc2);
        assert!(v050_rc2 < v050);
        assert!(v050 < v051);
        assert_eq!(v050.to_string(), "0.5.0");
        assert_eq!(v050_rc2.to_string(), "0.5.0-rc.2");

        assert!(SemVer::parse("0.5").is_none());
        assert!(SemVer::parse("v0.5.0-").is_none());
        assert!(SemVer::parse("not-a-version").is_none());
    }

    #[test]
    fn github_release_url_allowlist_enforces_pinning() {
        assert!(validate_github_release_url(GITHUB_RELEASES_LATEST_API).is_ok());
        assert!(validate_github_release_url(GITHUB_RELEASES_PAGE_URL).is_ok());
        assert!(validate_github_release_url(
            "https://github.com/Borab55/hp-omen-hub-lite/releases/download/v0.5.0/omvix-hub.exe"
        )
        .is_ok());
        assert!(validate_github_release_url(
            "https://objects.githubusercontent.com/github-production-release-asset-123/omvix-hub.exe"
        )
        .is_ok());
        assert!(validate_github_release_url(
            "https://release-assets.githubusercontent.com/file.exe"
        )
        .is_ok());

        assert!(validate_github_release_url(
            "http://github.com/Borab55/hp-omen-hub-lite/releases"
        )
        .is_err());
        assert!(validate_github_release_url(
            "https://github.com/OtherUser/hp-omen-hub-lite/releases"
        )
        .is_err());
        assert!(validate_github_release_url("https://evil.example.com/omvix-hub.exe").is_err());
        assert!(validate_github_release_url(
            "https://github.com/Borab55/hp-omen-hub-lite/releases;calc.exe"
        )
        .is_err());
        assert!(validate_github_release_url(
            "https://user@github.com/Borab55/hp-omen-hub-lite/releases"
        )
        .is_err());
    }

    #[test]
    fn parses_github_release_json_for_exe_and_zip_and_rejects_draft_or_bad_urls() {
        let exe_json = r#"{
            "tag_name": "v0.5.0",
            "name": "Omvix Hub v0.5.0",
            "draft": false,
            "published_at": "2026-11-22T00:00:00Z",
            "html_url": "https://github.com/Borab55/hp-omen-hub-lite/releases/tag/v0.5.0",
            "body": "Release notes here",
            "assets": [
                {
                    "name": "omvix-cli.exe",
                    "browser_download_url": "https://github.com/Borab55/hp-omen-hub-lite/releases/download/v0.5.0/omvix-cli.exe",
                    "size": 100000
                },
                {
                    "name": "omvix-hub.exe",
                    "browser_download_url": "https://github.com/Borab55/hp-omen-hub-lite/releases/download/v0.5.0/omvix-hub.exe",
                    "size": 4500000
                },
                {
                    "name": "SHA256SUMS.txt",
                    "browser_download_url": "https://github.com/Borab55/hp-omen-hub-lite/releases/download/v0.5.0/SHA256SUMS.txt",
                    "size": 256
                }
            ]
        }"#;
        let parsed = parse_github_release_json(exe_json).expect("exe release");
        assert_eq!(parsed.version, "0.5.0");
        assert_eq!(parsed.binary_asset.name, "omvix-hub.exe");
        assert_eq!(parsed.checksum_asset.unwrap().name, "SHA256SUMS.txt");

        let zip_json = r#"{
            "tag_name": "v0.5.1",
            "draft": false,
            "assets": [
                {
                    "name": "omvix-hub-win-x64.zip",
                    "browser_download_url": "https://objects.githubusercontent.com/assets/omvix-hub-win-x64.zip",
                    "size": 3000000
                }
            ]
        }"#;
        let parsed_zip = parse_github_release_json(zip_json).expect("zip release");
        assert_eq!(parsed_zip.binary_asset.name, "omvix-hub-win-x64.zip");
        assert!(parsed_zip.checksum_asset.is_none());

        let draft_json = r#"{"tag_name":"v0.5.0","draft":true,"assets":[]}"#;
        assert!(parse_github_release_json(draft_json).is_err());

        let bad_url_json = r#"{
            "tag_name": "v0.5.0",
            "draft": false,
            "assets": [{
                "name": "omvix-hub.exe",
                "browser_download_url": "https://attacker.tld/omvix-hub.exe",
                "size": 1000
            }]
        }"#;
        assert!(parse_github_release_json(bad_url_json).is_err());
    }

    #[test]
    fn parses_sha256_manifest_formats_and_rejects_invalid() {
        let h1 = "A".repeat(64);
        let h2 = "b".repeat(64);
        let manifest = format!(
            "# comment\n{h1}  omvix-cli.exe\n{h2} *dist/omvix-hub.exe\nnotahash omvix-hub.exe\n"
        );
        assert_eq!(
            parse_sha256_manifest(&manifest, "omvix-hub.exe"),
            Some("b".repeat(64))
        );
        assert_eq!(
            parse_sha256_manifest(&format!("  {}\n", "C".repeat(64)), "omvix-hub.exe"),
            Some("c".repeat(64))
        );
        assert!(parse_sha256_manifest(&manifest, "missing.exe").is_none());
        assert!(parse_sha256_manifest("short-hash omvix-hub.exe", "omvix-hub.exe").is_none());
    }

    #[test]
    fn authenticode_policy_and_install_gate_fail_closed() {
        let valid_sig = AuthenticodeInfo {
            status: "Valid".into(),
            subject: Some("CN=Omvix Studio".into()),
            issuer: Some("CN=Trusted CA".into()),
            thumbprint: Some("DEADBEEF".into()),
            not_after: None,
        };
        let eval_ok = evaluate_authenticode_signature(&valid_sig, Some("CN=Omvix Studio"));
        assert!(matches!(
            eval_ok,
            UpdateSignatureStatus::AuthenticodeValid { .. }
        ));
        assert!(matches!(
            evaluate_authenticode_signature(&valid_sig, Some("CN=Other")),
            UpdateSignatureStatus::Rejected(_)
        ));

        let unsigned_sig = AuthenticodeInfo {
            status: "NotSigned".into(),
            subject: None,
            issuer: None,
            thumbprint: None,
            not_after: None,
        };
        let eval_unsigned = evaluate_authenticode_signature(&unsigned_sig, None);
        assert_eq!(eval_unsigned, UpdateSignatureStatus::UnsignedSha256Only);

        let tampered_sig = AuthenticodeInfo {
            status: "HashMismatch".into(),
            subject: Some("CN=Omvix Studio".into()),
            issuer: Some("CN=Trusted CA".into()),
            thumbprint: None,
            not_after: None,
        };
        assert!(matches!(
            evaluate_authenticode_signature(&tampered_sig, None),
            UpdateSignatureStatus::Rejected(_)
        ));

        let hash = "a".repeat(64);
        let staged_signed = StagedUpdate {
            version: "0.5.0".into(),
            binary_path: PathBuf::from("omvix-hub.exe"),
            expected_sha256: hash.clone(),
            verified_sha256: hash.clone(),
            signature: eval_ok,
            release_url: GITHUB_RELEASES_PAGE_URL.into(),
        };
        assert!(staged_signed.can_install(false));
        assert!(staged_signed.can_install(true));

        let mut staged_unsigned = staged_signed.clone();
        staged_unsigned.signature = UpdateSignatureStatus::UnsignedSha256Only;
        assert!(!staged_unsigned.can_install(false));
        assert!(staged_unsigned.can_install(true));

        let mut staged_bad_hash = staged_signed.clone();
        staged_bad_hash.verified_sha256 = "f".repeat(64);
        assert!(!staged_bad_hash.can_install(true));

        let mut staged_rejected = staged_signed;
        staged_rejected.signature = UpdateSignatureStatus::Rejected("HashMismatch".into());
        assert!(!staged_rejected.can_install(true));
    }
}
