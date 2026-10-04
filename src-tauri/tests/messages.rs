use symfolinker_lib::models::Message;

#[test]
fn ipc_messages_preserve_keys_and_parameter_data() {
    let path = r"D:\dev\{name}\bundle";
    let message = Message::with("{path} does not contain composer.json.", "path", path);
    let json = serde_json::to_value(message).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "key": "{path} does not contain composer.json.",
            "params": { "path": path }
        })
    );
}

#[test]
fn messages_without_parameters_serialize_an_empty_parameter_map() {
    let json = serde_json::to_value(Message::new("The scan could not be completed.")).unwrap();
    assert_eq!(json["params"], serde_json::json!({}));
}
