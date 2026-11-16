use anyhow::{Context, Result, bail};
use regex::Regex;
use semver::Version;
use std::collections::BTreeSet;

pub fn parse(input: &str) -> Result<Version> {
    let raw = input.trim();
    let raw = raw.strip_prefix('v').unwrap_or(raw);
    let suffix_start = raw.find(['-', '+']).unwrap_or(raw.len());
    let (core, suffix) = raw.split_at(suffix_start);
    let parts: Vec<_> = core.split('.').collect();
    if !(1..=3).contains(&parts.len())
        || parts
            .iter()
            .any(|p| p.is_empty() || !p.bytes().all(|c| c.is_ascii_digit()))
    {
        bail!("expected a numeric version such as 1.2.3, got {input:?}");
    }
    let mut normalized = parts.join(".");
    for _ in parts.len()..3 {
        normalized.push_str(".0");
    }
    normalized.push_str(suffix);
    Version::parse(&normalized).with_context(|| format!("invalid version {input:?}"))
}

pub fn extract(stdout: &str, stderr: &str, pattern: Option<&Regex>) -> Result<Version> {
    let mut versions = BTreeSet::new();
    for output in [stdout, stderr] {
        if let Some(pattern) = pattern {
            for capture in pattern.captures_iter(output) {
                if let Some(value) = capture.get(1) {
                    versions.insert(parse(value.as_str())?);
                }
            }
        } else {
            let tokens: Vec<_> = output
                .split_whitespace()
                .map(|token| {
                    token.trim_matches(['(', ')', '[', ']', ',', ';', ':', '=', '\'', '"'])
                })
                .collect();
            // Prefer dotted versions, so Ruby's dates and revision numbers are not versions.
            let dotted: Vec<_> = tokens.iter().filter(|t| t.contains('.')).collect();
            if dotted.is_empty() {
                for token in tokens {
                    let number = token.strip_prefix('v').unwrap_or(token);
                    if !number.is_empty() && number.bytes().all(|c| c.is_ascii_digit()) {
                        versions.insert(parse(token)?);
                    }
                }
            } else {
                for token in dotted {
                    let number = token.strip_prefix('v').unwrap_or(token);
                    if number.starts_with(|c: char| c.is_ascii_digit()) {
                        versions.insert(parse(token)?);
                    }
                }
            }
        }
    }
    if versions.len() != 1 {
        bail!(
            "expected one unambiguous version; set version_regex with capture group 1 (found {})",
            versions.len()
        );
    }
    Ok(versions.into_iter().next().expect("one version"))
}
