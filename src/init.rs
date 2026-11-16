use crate::{policy, version};
use anyhow::{Context, Result, bail, ensure};
use semver::Version;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

const PINS: &[(&str, &str)] = &[
    ("bazel", ".bazelversion"),
    ("node", ".node-version"),
    ("node", ".nvmrc"),
    ("python", ".python-version"),
    ("ruby", ".ruby-version"),
    ("terraform", ".terraform-version"),
];

const EXAMPLES: &str = r#"
# Examples only: uncomment and choose your team's actual requirements.
# Any executable works; command defaults to the logical tool name.
# [tools.dekit]
# minimum = "0.10.3"
# reason = "Required for Kitty graphics passthrough"
# upgrade = "Follow your team's dekit installation guide"

# [tools.opendhu]
# command = "aa"
# minimum = "1.2.3" # Illustrative example, not a verified openDHU floor
# version_args = ["--version"]
# severity = "warn"
# optional = true
# reason = "Used for Android Auto development"
# upgrade = "Follow your team's openDHU installation guide"
"#;

#[derive(Serialize)]
pub struct DetectedTool {
    name: &'static str,
    exact_from: &'static str,
}

#[derive(Serialize)]
pub struct Report {
    schema_version: u32,
    status: &'static str,
    config: String,
    tools: Vec<DetectedTool>,
    warnings: Vec<String>,
}

pub fn create(path: &Path) -> Result<Report> {
    match fs::symlink_metadata(path) {
        Ok(_) => bail!(
            "{} already exists; refusing to overwrite it",
            path.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("cannot inspect {}", path.display()));
        }
    }
    let base = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    ensure!(
        base.is_dir(),
        "manifest directory {} must already exist",
        base.display()
    );
    let mut candidates: BTreeMap<&str, Vec<(&str, Version)>> = BTreeMap::new();
    let mut warnings = Vec::new();
    for &(name, pin) in PINS {
        let pin_path = base.join(pin);
        match fs::symlink_metadata(&pin_path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            _ => {}
        }
        match policy::read_bounded(&pin_path, 4096).and_then(|text| version::parse(&text)) {
            Ok(version) => candidates.entry(name).or_default().push((pin, version)),
            Err(error) => warnings.push(format!(
                "Skipped {pin}: needs one readable plain SemVer version ({error:#})"
            )),
        }
    }
    let mut tools = Vec::new();
    for (name, pins) in candidates {
        if pins
            .iter()
            .skip(1)
            .any(|(_, version)| version.cmp_precedence(&pins[0].1).is_ne())
        {
            warnings.push(format!(
                "Skipped {name}: conflicting pins {}; choose one manually",
                pins.iter()
                    .map(|(pin, _)| *pin)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        } else {
            tools.push(DetectedTool {
                name,
                exact_from: pins[0].0,
            });
        }
    }
    let mut source = String::from(
        "# Team tool policy. Review detected pins and customize the examples below.\nschema_version = 1\n",
    );
    for tool in &tools {
        source.push_str(&format!(
            "\n[tools.{}]\nexact_from = {:?}\n",
            tool.name, tool.exact_from
        ));
    }
    if tools.is_empty() {
        source.push_str(
            "\n# No usable version pins detected. Uncomment examples before checking tools.\n",
        );
    }
    for warning in &warnings {
        for line in warning.lines() {
            source.push_str(&format!("# {line}\n"));
        }
    }
    source.push_str(EXAMPLES);
    // create_new also refuses dangling symlinks and guards against a concurrent init.
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| {
            format!(
                "cannot create {}; existing files are never overwritten",
                path.display()
            )
        })?;
    file.write_all(source.as_bytes())
        .with_context(|| format!("cannot write {}", path.display()))?;
    Ok(Report {
        schema_version: 1,
        status: "created",
        config: path.to_string_lossy().into_owned(),
        tools,
        warnings,
    })
}

pub fn human(report: &Report) -> String {
    let mut output = format!(
        "Created {} ({} detected tools).\n",
        report.config,
        report.tools.len()
    );
    for tool in &report.tools {
        output.push_str(&format!(
            "  {}: exact_from = {:?}\n",
            tool.name, tool.exact_from
        ));
    }
    for warning in &report.warnings {
        output.push_str(&format!("  Warning: {warning}\n"));
    }
    output.push_str(
        "Review the pins and uncomment examples for other tools before running toolcheck check.\n",
    );
    output
}
