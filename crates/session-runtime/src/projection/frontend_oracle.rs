//! Reference states come from the unmodified pinned Rust SDK, not a second
//! implementation of the web reducer. Shared traces exercise our current wire
//! surface; the partial-reorder SDK/spec divergence is deliberately explicit.
use ahp::reducers::apply_action_to_chat;
use ahp_types::{actions::StateAction, state::ChatState};
use serde_json::{Value, json};
#[test]
fn pinned_sdk_frontend_reducer_oracle_matches_checked_in_fixture() {
    let traces: Vec<Value> = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../apps/web/tests/fixtures/ahp-reducer-traces.json"
    )))
    .unwrap();
    let mut output = Vec::new();
    for trace in traces {
        let mut state: ChatState = serde_json::from_value(trace["initial"].clone()).unwrap();
        let mut states = vec![serde_json::to_value(&state).unwrap()];
        let mut outcomes = Vec::new();
        for (index, raw) in trace["actions"].as_array().unwrap().iter().enumerate() {
            let action: StateAction = serde_json::from_value(raw.clone()).unwrap_or_else(|error| {
                panic!("Invalid trace {} action {index}: {error}", trace["name"])
            });
            let outcome = apply_action_to_chat(&mut state, &action);
            outcomes.push(format!("{outcome:?}"));
            states.push(serde_json::to_value(&state).unwrap());
        }
        output.push(json!({"name":trace["name"],"states":states,"outcomes":outcomes}));
    }
    let generated = json!({"sdk":"ahp-1.0.0","traces":output});
    let checked_in: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../apps/web/tests/fixtures/ahp-reducer-expected.json"
    )))
    .unwrap();
    if generated != checked_in {
        let raw = serde_json::to_string(&generated).unwrap();
        assert!(
            raw.len() <= 512 * 1024,
            "Oracle exceeds bounded test fixture budget"
        );
        println!("GENERATED_AHP_ORACLE={raw}");
        panic!(
            "Pinned SDK oracle changed; import the generated reference fixture, then verify again"
        );
    }
}
