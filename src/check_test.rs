#[cfg(unix)]
#[test]
fn non_utf8_executable_paths_serialize_without_losing_the_report() {
    use super::*;
    use std::os::unix::ffi::OsStringExt;
    let path = PathBuf::from(std::ffi::OsString::from_vec(b"/bin-\xff/a".to_vec()));
    let report = ToolResult {
        name: "a".into(),
        command: "a".into(),
        status: "pass",
        issue: None,
        actual: Some("1.0.0".into()),
        required: ">=1.0.0".into(),
        selected_path: Some(path.clone()),
        shadowed: vec![Shadow {
            path,
            actual: None,
            compatible: None,
            issue: Some("not_probed"),
            message: "shadowed".into(),
        }],
        reason: None,
        remediation: None,
        message: "meets policy".into(),
    };
    let value = serde_json::to_value(report).expect("report must always serialize");
    assert_eq!(value["selected_path"], "/bin-\u{fffd}/a");
    assert_eq!(value["shadowed"][0]["path"], "/bin-\u{fffd}/a");
}
