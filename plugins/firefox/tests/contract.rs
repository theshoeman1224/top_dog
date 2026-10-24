use std::{
    io::Write,
    process::{Command, Stdio},
};
use top_dog_protocol::{Operation, Outcome, Reply, Request, Response};

#[test]
fn real_firefox_plugin_serves_describe_and_diagnosis_and_recovers_from_errors() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_top-dog-firefox"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(b"malformed\n").unwrap();
    for (id, op) in [(1, Operation::Describe), (2, Operation::Diagnose)] {
        writeln!(
            stdin,
            "{}",
            serde_json::to_string(&Request::new(id, op)).unwrap()
        )
        .unwrap();
    }
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    let replies: Vec<Response> = text
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(replies.len(), 3);
    assert!(matches!(&replies[0].outcome, Outcome::Error(_)));
    assert!(
        matches!(&replies[1].outcome,Outcome::Ok(Reply::Description {name,..}) if name=="firefox")
    );
    assert!(matches!(
        &replies[2].outcome,
        Outcome::Ok(Reply::Diagnosis {
            available: false,
            ..
        })
    ));
}
