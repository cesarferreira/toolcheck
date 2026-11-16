use crate::check::{Report, ToolResult};
use unicode_width::UnicodeWidthStr;

pub fn human(report: &Report, verbose: bool, color: bool) -> String {
    if report.tools.is_empty() {
        return "No tools configured. Add tools to your manifest to start checking.\n".into();
    }
    let name_width = report
        .tools
        .iter()
        .map(|t| t.name.width())
        .max()
        .unwrap_or(4)
        .max(4);
    let version_width = report
        .tools
        .iter()
        .map(|t| actual(t).len())
        .max()
        .unwrap_or(9)
        .max(9);
    let header = format!(
        "  {:<6} {:<name_width$}  {:<version_width$}  REQUIRED",
        "STATUS", "TOOL", "INSTALLED"
    );
    let mut output = format!("{}\n", paint(&header, "2", color));
    for tool in &report.tools {
        let (label, style) = match tool.status {
            "pass" => ("PASS", "32"),
            "warn" => ("WARN", "33"),
            "skipped" => ("SKIP", "2"),
            _ => ("FAIL", "31"),
        };
        let required = if tool.status == "skipped" && !verbose {
            "optional"
        } else {
            &tool.required
        };
        output.push_str(&format!(
            "  {} {}  {}  {}\n",
            paint(&format!("{label:<6}"), style, color),
            paint(
                &pad(&tool.name, name_width),
                if tool.status == "skipped" { "2" } else { "1" },
                color
            ),
            paint(
                &format!("{:<version_width$}", actual(tool)),
                if matches!(tool.status, "warn" | "error") {
                    style
                } else {
                    "0"
                },
                color
            ),
            paint(required, "2", color)
        ));
        let attention = matches!(tool.status, "warn" | "error");
        if attention && tool.issue != Some("version_mismatch") {
            detail(
                &mut output,
                &format!("{}: {}", tool.command, tool.message),
                style,
                color,
            );
        }
        if verbose {
            let path = tool
                .selected_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "not found".into());
            detail(&mut output, &format!("Path: {path}"), "2", color);
            if let Some(reason) = &tool.reason {
                detail(&mut output, &format!("Reason: {reason}"), "2", color);
            }
        }
        if attention || verbose {
            if let Some(upgrade) = &tool.remediation {
                detail(&mut output, &format!("Upgrade: {upgrade}"), "36", color);
            } else if attention && !verbose && tool.issue == Some("version_mismatch") {
                if let Some(reason) = &tool.reason {
                    detail(&mut output, reason, "2", color);
                }
            }
        }
        if verbose {
            for shadow in &tool.shadowed {
                detail(
                    &mut output,
                    &format!(
                        "Shadowed: {} ({}; {})",
                        shadow.path.display(),
                        shadow.actual.as_deref().unwrap_or("unknown"),
                        shadow.message
                    ),
                    "2",
                    color,
                );
            }
        } else if let Some(shadow) = tool.shadowed.iter().find(|s| s.compatible == Some(true)) {
            detail(
                &mut output,
                &format!(
                    "PATH: compatible {} at {}",
                    shadow.actual.as_deref().unwrap_or("version"),
                    shadow.path.display()
                ),
                "36",
                color,
            );
        }
    }
    let summary = &report.summary;
    let mut counts = Vec::new();
    for (count, singular, plural, style) in [
        (summary.pass, "passed", "passed", "32"),
        (summary.warn, "warning", "warnings", "33"),
        (summary.error, "error", "errors", "31"),
        (summary.skipped, "skipped", "skipped", "2"),
    ] {
        if count > 0 {
            counts.push(paint(
                &format!("{count} {}", if count == 1 { singular } else { plural }),
                style,
                color,
            ));
        }
    }
    if !summary.acceptable && summary.error == 0 {
        counts.push(paint("failed (strict)", "31", color));
    }
    output.push_str(&format!("\n  {}\n", counts.join(" · ")));
    output
}

fn actual(tool: &ToolResult) -> &str {
    tool.actual
        .as_deref()
        .unwrap_or(if tool.issue == Some("missing") {
            "missing"
        } else {
            "unknown"
        })
}

fn paint(text: &str, style: &str, color: bool) -> String {
    if color && style != "0" {
        format!("\x1b[{style}m{text}\x1b[0m")
    } else {
        text.to_owned()
    }
}

fn detail(output: &mut String, text: &str, style: &str, color: bool) {
    output.push_str(&format!("         {}\n", paint(text, style, color)));
}

fn pad(text: &str, width: usize) -> String {
    format!("{text}{}", " ".repeat(width.saturating_sub(text.width())))
}
