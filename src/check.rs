use crate::policy::{Severity, Tool};
use crate::process;
use serde::Serialize;
use std::path::PathBuf;
use std::time::Instant;

// Shadow versions are advisory; at most three extra commands share the tool deadline.
const MAX_SHADOW_PROBES: usize = 3;

#[derive(Serialize)]
pub struct Shadow {
    #[serde(serialize_with = "serialize_path")]
    pub path: PathBuf,
    pub actual: Option<String>,
    pub compatible: Option<bool>,
    pub issue: Option<&'static str>,
    pub message: String,
}

#[derive(Serialize)]
pub struct ToolResult {
    pub name: String,
    pub command: String,
    pub status: &'static str,
    pub issue: Option<&'static str>,
    pub actual: Option<String>,
    pub required: String,
    #[serde(serialize_with = "serialize_optional_path")]
    pub selected_path: Option<PathBuf>,
    pub shadowed: Vec<Shadow>,
    pub reason: Option<String>,
    pub remediation: Option<String>,
    pub message: String,
}

#[derive(Default, Serialize)]
pub struct Summary {
    pub pass: usize,
    pub warn: usize,
    pub error: usize,
    pub skipped: usize,
    pub acceptable: bool,
}

#[derive(Serialize)]
pub struct Report {
    pub schema_version: u32,
    pub tools: Vec<ToolResult>,
    pub summary: Summary,
}

pub fn run(tools: &[Tool], strict: bool) -> Report {
    let tools: Vec<_> = tools.iter().map(check).collect();
    let mut summary = Summary::default();
    for tool in &tools {
        match tool.status {
            "pass" => summary.pass += 1,
            "warn" => summary.warn += 1,
            "skipped" => summary.skipped += 1,
            _ => summary.error += 1,
        }
    }
    summary.acceptable = summary.error == 0 && (!strict || summary.warn == 0);
    Report {
        schema_version: 1,
        tools,
        summary,
    }
}

fn check(tool: &Tool) -> ToolResult {
    let paths = process::resolve(&tool.command);
    let selected_path = paths.first().cloned();
    let mut result = ToolResult {
        name: tool.name.clone(),
        command: tool.command.clone(),
        status: "pass",
        issue: None,
        actual: None,
        required: tool.required(),
        selected_path,
        shadowed: Vec::new(),
        reason: tool.reason.clone(),
        remediation: tool.upgrade.clone(),
        message: "installed version meets policy".into(),
    };
    let failure_status = match tool.severity {
        Severity::Warn => "warn",
        Severity::Error => "error",
    };
    let Some(selected) = &result.selected_path else {
        result.status = if tool.optional { "skipped" } else { "error" };
        result.issue = Some("missing");
        result.message = if tool.optional {
            "optional executable is absent"
        } else {
            "executable was not found on PATH"
        }
        .into();
        return result;
    };
    let deadline = Instant::now() + tool.timeout;
    match process::probe(selected.as_os_str(), tool, deadline) {
        Ok(actual) => {
            if !tool.matches(&actual) {
                result.status = failure_status;
                result.issue = Some("version_mismatch");
                result.message = "installed version does not meet policy".into();
            }
            result.actual = Some(actual.to_string());
        }
        Err(failure) => {
            result.status = failure_status;
            result.issue = Some(failure.issue);
            result.message = failure.message;
        }
    }
    for (index, path) in paths.into_iter().skip(1).enumerate() {
        let mut shadow = Shadow {
            path,
            actual: None,
            compatible: None,
            issue: Some("not_probed"),
            message: "shadow probe limit or tool deadline exhausted".into(),
        };
        // Probe shadows only when the selected installation has a known mismatch.
        if result.issue == Some("version_mismatch")
            && index < MAX_SHADOW_PROBES
            && Instant::now() < deadline
        {
            match process::probe(shadow.path.as_os_str(), tool, deadline) {
                Ok(actual) => {
                    shadow.compatible = Some(tool.matches(&actual));
                    shadow.actual = Some(actual.to_string());
                    shadow.issue = None;
                    shadow.message = "shadowed by the selected executable on PATH".into();
                    if shadow.compatible == Some(true) {
                        result.message = "selected version does not meet policy; a compatible installation is shadowed later on PATH (review PATH order)".into();
                    }
                }
                Err(failure) => {
                    shadow.issue = Some(failure.issue);
                    shadow.message = failure.message;
                }
            }
        }
        result.shadowed.push(shadow);
    }
    result
}

pub fn human(report: &Report) -> String {
    let mut output = String::new();
    for tool in &report.tools {
        output.push_str(&format!(
            "{} {}: installed {}; required {}\n",
            tool.status.to_uppercase(),
            tool.name,
            tool.actual.as_deref().unwrap_or("unknown"),
            tool.required
        ));
        output.push_str(&format!(
            "  Executable: {}\n  {}\n",
            tool.selected_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "not found".into()),
            tool.message
        ));
        if let Some(reason) = &tool.reason {
            output.push_str(&format!("  Reason: {reason}\n"));
        }
        if let Some(upgrade) = &tool.remediation {
            output.push_str(&format!("  Upgrade (display only): {upgrade}\n"));
        }
        for shadow in &tool.shadowed {
            output.push_str(&format!(
                "  Shadowed: {} ({}; {})\n",
                shadow.path.display(),
                shadow.actual.as_deref().unwrap_or("unknown"),
                shadow.message
            ));
        }
    }
    let s = &report.summary;
    output.push_str(&format!(
        "{} passed, {} warnings, {} errors, {} skipped — {}\n",
        s.pass,
        s.warn,
        s.error,
        s.skipped,
        if s.acceptable {
            "acceptable"
        } else {
            "requirements unmet"
        }
    ));
    output
}

#[cfg(test)]
#[path = "check_test.rs"]
mod tests;

fn serialize_path<S: serde::Serializer>(
    path: &std::path::Path,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&path.to_string_lossy())
}

fn serialize_optional_path<S: serde::Serializer>(
    path: &Option<PathBuf>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    path.as_ref()
        .map(|p| p.to_string_lossy())
        .serialize(serializer)
}
