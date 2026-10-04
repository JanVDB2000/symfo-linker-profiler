use symfolinker_lib::engine::{detect, Engine};

#[test]
fn prefers_docker_when_both_engines_are_installed() {
    let engine = detect(|binary| binary == "docker" || binary == "podman");

    assert_eq!(engine, Some(Engine::Docker));
}

#[test]
fn falls_back_to_podman_when_docker_is_absent() {
    let engine = detect(|binary| binary == "podman");

    assert_eq!(engine, Some(Engine::Podman));
}

#[test]
fn reports_nothing_when_neither_engine_answers() {
    assert_eq!(detect(|_| false), None);
}

#[test]
fn a_binary_that_exists_but_does_not_answer_is_skipped() {
    // Docker Desktop installed but stopped: the CLI is there, the daemon is not.
    let engine = detect(|binary| binary != "docker");

    assert_eq!(engine, Some(Engine::Podman));
}

#[test]
fn each_engine_knows_its_binary_and_label() {
    assert_eq!(Engine::Docker.binary(), "docker");
    assert_eq!(Engine::Podman.binary(), "podman");
    // Shown in the interface, so the user knows which engine answered.
    assert_eq!(Engine::Docker.label(), "Docker");
    assert_eq!(Engine::Podman.label(), "Podman");
}
