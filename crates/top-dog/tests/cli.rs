use std::process::Command;

#[test]
fn binary_help_and_invalid_arguments() {
    let bin = env!("CARGO_BIN_EXE_top-dog");
    let output = Command::new(bin).arg("--help").output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("--doctor"));
    assert!(
        !Command::new(bin)
            .args(["--pid", "0"])
            .output()
            .unwrap()
            .status
            .success()
    );
}

#[test]
fn json_sampling_filters_and_computes_second_cpu_sample() {
    let pid = std::process::id().to_string();
    let output = Command::new(env!("CARGO_BIN_EXE_top-dog"))
        .args([
            "--json",
            "--iterations",
            "2",
            "--refresh-ms",
            "100",
            "--pid",
            &pid,
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    let rows: Vec<serde_json::Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(rows.len(), 2);
    assert!(rows[0]["cpu_percent"].is_null());
    assert!(rows[1]["cpu_percent"].is_number());
    for row in &rows {
        assert_eq!(row["processes"].as_array().unwrap().len(), 1);
        assert_eq!(row["processes"][0]["identity"]["pid"], std::process::id());
    }
    assert!(rows[1]["processes"][0]["cpu_percent"].is_number());
}

#[test]
fn redirected_interactive_mode_explains_batch_option() {
    let output = Command::new(env!("CARGO_BIN_EXE_top-dog"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--batch"));
}
