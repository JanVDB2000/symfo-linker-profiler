use std::{
    process::Command,
    time::{Duration, Instant},
};
use symfolinker_lib::process::{output_within, CommandError};

fn fixture(mode: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "process_child", "--nocapture"])
        .env("SYMFOLINKER_TEST_CHILD", mode);
    command
}

#[test]
fn process_child() {
    match std::env::var("SYMFOLINKER_TEST_CHILD").as_deref() {
        Ok("slow") => std::thread::sleep(Duration::from_secs(2)),
        Ok("output") => {
            println!("captured stdout");
            eprintln!("captured stderr");
        }
        Ok("inherited") => {
            // The direct child exits but its child keeps the output pipes open.
            let mut child = fixture("slow").spawn().unwrap();
            // Leave the child alive to exercise the pipe deadline.
            let _ = child.try_wait();
        }
        _ => {}
    }
}

#[test]
fn collects_both_output_streams() {
    let output = output_within(fixture("output"), Duration::from_secs(5)).unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("captured stdout"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("captured stderr"));
}

#[test]
fn kills_a_command_that_exceeds_its_deadline() {
    let start = Instant::now();
    assert_eq!(
        output_within(fixture("slow"), Duration::from_millis(250)).unwrap_err(),
        CommandError::TimedOut
    );
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn inherited_pipes_cannot_make_the_output_collection_wait_forever() {
    assert_eq!(
        output_within(fixture("inherited"), Duration::from_millis(500)).unwrap_err(),
        CommandError::TimedOut
    );
}

#[test]
fn a_missing_binary_has_a_distinct_error() {
    assert_eq!(
        output_within(
            Command::new("symfolinker-nonexistent-test-command"),
            Duration::from_secs(1)
        )
        .unwrap_err(),
        CommandError::NotStarted
    );
}
