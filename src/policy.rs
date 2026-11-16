use crate::version;
use anyhow::{Context, Result, bail, ensure};
use regex::Regex;
use semver::Version;
use serde::Deserialize;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    #[serde(default)]
    tools: BTreeMap<String, ToolSpec>,
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Warn,
    #[default]
    Error,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolSpec {
    command: Option<String>,
    minimum: Option<String>,
    maximum: Option<String>,
    exact: Option<String>,
    exact_from: Option<PathBuf>,
    version_args: Option<Vec<String>>,
    version_regex: Option<String>,
    #[serde(default)]
    severity: Severity,
    #[serde(default)]
    optional: bool,
    reason: Option<String>,
    upgrade: Option<String>,
    timeout_ms: Option<u64>,
}

pub struct Tool {
    pub name: String,
    pub command: String,
    pub minimum: Option<Version>,
    pub maximum: Option<Version>,
    pub exact: Option<Version>,
    pub version_args: Vec<String>,
    pub version_regex: Option<Regex>,
    pub severity: Severity,
    pub optional: bool,
    pub reason: Option<String>,
    pub upgrade: Option<String>,
    pub timeout: Duration,
}

pub fn read_bounded(path: &Path, limit: u64) -> Result<String> {
    ensure!(
        std::fs::metadata(path)
            .with_context(|| format!("cannot inspect {}", path.display()))?
            .is_file(),
        "{} must be a regular text file",
        path.display()
    );
    let mut text = String::new();
    File::open(path)
        .with_context(|| format!("cannot open {}", path.display()))?
        .take(limit + 1)
        .read_to_string(&mut text)
        .with_context(|| format!("cannot read UTF-8 text from {}", path.display()))?;
    ensure!(
        text.len() as u64 <= limit,
        "{} exceeds {limit} bytes",
        path.display()
    );
    Ok(text)
}

pub fn load(path: &Path) -> Result<Vec<Tool>> {
    let source = read_bounded(path, 1_048_576)?;
    let manifest: Manifest =
        toml::from_str(&source).with_context(|| format!("invalid policy {}", path.display()))?;
    ensure!(manifest.schema_version == 1, "schema_version must be 1");
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    manifest
        .tools
        .into_iter()
        .map(|(name, spec)| {
            validate(name.clone(), spec, base).with_context(|| format!("tools.{name}"))
        })
        .collect()
}

fn validate(name: String, spec: ToolSpec, base: &Path) -> Result<Tool> {
    ensure!(!name.trim().is_empty(), "logical name must not be empty");
    let command = spec.command.unwrap_or_else(|| name.clone());
    ensure!(
        !command.trim().is_empty() && !command.contains('\0'),
        "command must be an executable name or path"
    );
    ensure!(
        !command.contains(char::is_whitespace) || Path::new(&command).components().count() > 1,
        "command must be a binary; put arguments in version_args"
    );
    ensure!(
        spec.version_args
            .as_ref()
            .is_none_or(|args| args.iter().all(|a| !a.contains('\0'))),
        "version_args must not contain NUL"
    );
    let timeout_ms = spec.timeout_ms.unwrap_or(2000);
    ensure!(
        (1..=30000).contains(&timeout_ms),
        "timeout_ms must be between 1 and 30000"
    );
    ensure!(
        !(spec.exact.is_some() && spec.exact_from.is_some()),
        "exact and exact_from are mutually exclusive"
    );
    let exact = if let Some(file) = spec.exact_from {
        ensure!(
            !file.as_os_str().is_empty() && file.is_relative(),
            "exact_from must be a relative plain version-file path"
        );
        let file = base.join(file);
        Some(
            version::parse(&read_bounded(&file, 4096)?).with_context(|| {
                format!(
                    "exact_from {} must contain one plain version",
                    file.display()
                )
            })?,
        )
    } else {
        spec.exact
            .as_deref()
            .map(version::parse)
            .transpose()
            .context("exact")?
    };
    let minimum = spec
        .minimum
        .as_deref()
        .map(version::parse)
        .transpose()
        .context("minimum")?;
    let maximum = spec
        .maximum
        .as_deref()
        .map(version::parse)
        .transpose()
        .context("maximum")?;
    ensure!(
        exact.is_some() || minimum.is_some() || maximum.is_some(),
        "set minimum, maximum, exact, or exact_from"
    );
    ensure!(
        exact.is_none() || (minimum.is_none() && maximum.is_none()),
        "exact/exact_from cannot be combined with minimum/maximum"
    );
    if let (Some(min), Some(max)) = (&minimum, &maximum) {
        ensure!(
            min.cmp_precedence(max) != Ordering::Greater,
            "minimum exceeds maximum"
        );
    }
    let version_regex = spec
        .version_regex
        .map(|pattern| -> Result<Regex> {
            let regex = Regex::new(&pattern).context("invalid version_regex")?;
            if regex.captures_len() < 2 {
                bail!("version_regex needs capture group 1 containing the version");
            }
            Ok(regex)
        })
        .transpose()?;
    Ok(Tool {
        name,
        command,
        minimum,
        maximum,
        exact,
        version_args: spec
            .version_args
            .unwrap_or_else(|| vec!["--version".into()]),
        version_regex,
        severity: spec.severity,
        optional: spec.optional,
        reason: spec.reason,
        upgrade: spec.upgrade,
        timeout: Duration::from_millis(timeout_ms),
    })
}

impl Tool {
    pub fn matches(&self, actual: &Version) -> bool {
        let bounds = [&self.exact, &self.minimum, &self.maximum];
        if !actual.pre.is_empty()
            && !bounds.iter().any(|bound| {
                bound.as_ref().is_some_and(|b| {
                    !b.pre.is_empty()
                        && (b.major, b.minor, b.patch) == (actual.major, actual.minor, actual.patch)
                })
            })
        {
            return false;
        }
        self.exact
            .as_ref()
            .is_none_or(|v| actual.cmp_precedence(v) == Ordering::Equal)
            && self
                .minimum
                .as_ref()
                .is_none_or(|v| actual.cmp_precedence(v) != Ordering::Less)
            && self
                .maximum
                .as_ref()
                .is_none_or(|v| actual.cmp_precedence(v) != Ordering::Greater)
    }

    pub fn required(&self) -> String {
        if let Some(exact) = &self.exact {
            return format!("={exact}");
        }
        let mut requirements = Vec::new();
        if let Some(minimum) = &self.minimum {
            requirements.push(format!(">={minimum}"));
        }
        if let Some(maximum) = &self.maximum {
            requirements.push(format!("<={maximum}"));
        }
        requirements.join(", ")
    }
}
