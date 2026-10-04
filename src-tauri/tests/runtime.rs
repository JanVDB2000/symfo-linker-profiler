use symfolinker_lib::runtime::parse_compose_ps;

// Compose v2 schrijft een object per regel.
const JSON_LINES: &str = r#"
{"Name":"shop-api-php-fpm-1","Service":"php-fpm","State":"running","Status":"Up 3 minutes","Health":"healthy","Publishers":[{"PublishedPort":9000,"TargetPort":9000,"Protocol":"tcp"}]}
{"Name":"shop-api-redis-1","Service":"redis","State":"exited","Status":"Exited (0) 5 minutes ago","Health":"","Publishers":[]}
"#;

// Oudere compose-versies schrijven een JSON-array.
const JSON_ARRAY: &str = r#"[
  {"Name":"shop-api-nginx-1","Service":"nginx","State":"running","Status":"Up 3 minutes","Publishers":[{"PublishedPort":8080,"TargetPort":80,"Protocol":"tcp"}]}
]"#;

#[test]
fn reads_json_lines_output() {
    let services = parse_compose_ps(JSON_LINES);

    assert_eq!(services.len(), 2);
    assert!(services[0].running);
    assert_eq!(services[0].service, "php-fpm");
    assert_eq!(services[0].health.as_deref(), Some("healthy"));
    assert_eq!(services[0].ports, "9000->9000/tcp");
}

#[test]
fn reads_json_array_output() {
    let services = parse_compose_ps(JSON_ARRAY);

    assert_eq!(services.len(), 1);
    assert_eq!(services[0].service, "nginx");
    assert_eq!(services[0].ports, "8080->80/tcp");
}

#[test]
fn marks_non_running_states_as_stopped_without_health() {
    let services = parse_compose_ps(JSON_LINES);

    let redis = &services[1];
    assert!(!redis.running);
    assert_eq!(redis.state, "exited");
    // Een lege Health-string is "onbekend", niet de letterlijke tekst "".
    assert_eq!(redis.health, None);
    assert_eq!(redis.ports, "");
}

#[test]
fn state_matching_ignores_casing() {
    let services =
        parse_compose_ps(r#"{"Name":"a","Service":"a","State":"Running","Status":"Up"}"#);

    assert!(services[0].running);
}

#[test]
fn skips_unreadable_lines_instead_of_dropping_the_whole_status() {
    let mixed =
        "not json\n{\"Name\":\"a\",\"Service\":\"a\",\"State\":\"running\",\"Status\":\"Up\"}\n";

    let services = parse_compose_ps(mixed);

    assert_eq!(services.len(), 1);
    assert_eq!(services[0].name, "a");
}

#[test]
fn empty_output_yields_no_services() {
    assert!(parse_compose_ps("").is_empty());
    assert!(parse_compose_ps("   \n  ").is_empty());
}

#[test]
fn falls_back_to_container_name_when_service_is_absent() {
    let services = parse_compose_ps(r#"{"Name":"lone-container","State":"running","Status":"Up"}"#);

    assert_eq!(services[0].service, "lone-container");
}

#[test]
fn hides_unpublished_ports_and_deduplicates() {
    let row = r#"{"Name":"a","Service":"a","State":"running","Status":"Up","Publishers":[{"PublishedPort":0,"TargetPort":80,"Protocol":"tcp"},{"PublishedPort":8080,"TargetPort":80,"Protocol":"tcp"},{"PublishedPort":8080,"TargetPort":80,"Protocol":"tcp"}]}"#;

    assert_eq!(parse_compose_ps(row)[0].ports, "8080->80/tcp");
}

#[test]
fn ignores_json_that_is_not_a_service_row() {
    // Elk veld heeft een default, dus losse JSON mag geen lege service opleveren.
    let services = parse_compose_ps(r#"{"PublishedPort":8080,"TargetPort":80,"Protocol":"tcp"}"#);

    assert!(services.is_empty());
}
