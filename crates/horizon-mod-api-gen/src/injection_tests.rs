use super::inject;

fn injection(name: &str, event: &str) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "event": event,
        "declaration": format!("fn {name}(&self) {{}}"),
        "event_declaration": format!("pub struct {event};"),
    })
}

#[test]
fn rejects_duplicate_methods_and_event_names_across_injections() {
    let api = "pub trait Hooks: Send + Sync {}";
    for declarations in [
        vec![
            injection("tick", "TickEvent"),
            injection("tick", "OtherEvent"),
        ],
        vec![
            injection("tick", "TickEvent"),
            injection("other", "TickEvent"),
        ],
    ] {
        assert!(inject(api, &serde_json::to_string(&declarations).unwrap()).is_err());
    }
}

#[test]
fn rejects_malformed_or_misnamed_declarations_at_the_boundary() {
    let api = "pub trait Hooks: Send + Sync {}";
    let mut wrong_method = injection("tick", "TickEvent");
    wrong_method["declaration"] = "fn other(&self) {}".into();
    let mut wrong_event = injection("tick", "TickEvent");
    wrong_event["event_declaration"] = "pub struct OtherEvent;".into();
    let mut invalid_rust = injection("tick", "TickEvent");
    invalid_rust["event_declaration"] = "not a struct".into();
    for declaration in [wrong_method, wrong_event, invalid_rust] {
        assert!(inject(api, &serde_json::json!([declaration]).to_string()).is_err());
    }
    for invalid in ["{}", "[{}]", "null", "not json"] {
        assert!(inject(api, invalid).is_err());
    }
}
