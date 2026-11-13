use super::*;
#[derive(Default)]
pub(super) struct Forms {
    pub mode: AtomicUsize,
    pub continuations: AtomicUsize,
    previous: Mutex<Option<serde_json::Value>>,
}
pub(in crate::worker_fixture) fn content() -> serde_json::Value {
    json!({"name":"Reviewed","count":0.0,"active":false,"choice":"","many":["x","y"],"ratio":0.5,"date":"2026-11-13","email":"user@example.test","uri":"https://docs.example.test/plain-text","__proto__":"reviewed-proto"})
}
fn schema() -> serde_json::Value {
    json!({"type":"object","properties":{
        "name":{"type":"string","title":"Nazwa","minLength":2,"maxLength":16,"default":"Propozycja"},
        "count":{"type":"integer","title":"Liczba","minimum":0,"maximum":9},
        "active":{"type":"boolean","title":"Aktywne","default":true},
        "choice":{"type":"string","title":"Wybór","enum":["","a"],"default":"a"},
        "many":{"type":"array","title":"Kilka opcji","items":{"type":"string","enum":["x","y"]},"minItems":1,"default":["x"]},
        "ratio":{"type":"number","title":"Proporcja","minimum":-1,"maximum":1},
        "date":{"type":"string","title":"Data","format":"date"},
        "email":{"type":"string","title":"Email","format":"email"},
        "uri":{"type":"string","title":"Adres jako tekst","format":"uri"},
        "__proto__":{"type":"string","title":"Pole z nazwą specjalną"}},
        "required":["name","count","active","choice","many","ratio","date","email","uri","__proto__"]})
}
impl Forms {
    pub(super) fn reply(&self, request: &serde_json::Value) -> Option<serde_json::Value> {
        let mode = self.mode.load(Ordering::Acquire);
        if mode == 0 {
            return None;
        }
        let state = request["params"].get("requestState");
        let mut previous = self.previous.lock().unwrap();
        if state.is_some() {
            assert_ne!(previous.as_ref(), Some(&request["id"]));
            self.continuations.fetch_add(1, Ordering::Release);
        }
        *previous = Some(request["id"].clone());
        if mode >= 4 {
            assert_eq!(
                request["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"]["elicitation"]
                    ["url"],
                json!({})
            );
            let action = match mode {
                4 => "accept",
                5 => "decline",
                6 => "cancel",
                _ => panic!("unknown URL fixture mode"),
            };
            if let Some(state) = state {
                assert_eq!(
                    request["params"]["inputResponses"],
                    json!({"review":{"action":action}})
                );
                if action != "accept" {
                    return Some(
                        json!({"resultType":"complete","isError":true,"content":[{"type":"text","text":"The human declined or cancelled the URL interaction."}]}),
                    );
                }
                if state == "private-url-state-second" {
                    return Some(
                        json!({"resultType":"complete","content":[{"type":"text","text":"native broker result"}]}),
                    );
                }
                assert_eq!(state, "private-url-state-first");
            } else {
                assert!(request["params"].get("inputResponses").is_none());
            }
            let step = if state.is_none() { 1 } else { 2 };
            return Some(
                json!({"resultType":"input_required","requestState":if step==1 {"private-url-state-first"}else{"private-url-state-second"},"inputRequests":{"review":{"method":"elicitation/create","params":{"mode":"url","message":if step==1 {"Sprawdź stronę dostawcy"}else{"Dostawca wymaga ponownej decyzji"},"url":format!("https://review.example.test/connect?step={step}")}}}}),
            );
        }
        if state.is_none() {
            assert!(request["params"].get("inputResponses").is_none());
            return Some(
                json!({"resultType":"input_required","requestState":"private-mrtr-state-first","inputRequests":{"details":{"method":"elicitation/create","params":{"mode":"form","message":"Sprawdź dane do wyszukania","requestedSchema":schema()}}}}),
            );
        }
        let action = match mode {
            1 => "accept",
            2 => "decline",
            3 => "cancel",
            _ => panic!("unknown fixture form mode"),
        };
        if state == Some(&json!("private-mrtr-state-first")) {
            let response = if mode == 1 {
                json!({"action":action,"content":content()})
            } else {
                json!({"action":action})
            };
            assert_eq!(
                request["params"]["inputResponses"],
                json!({"details":response})
            );
            if mode == 1 {
                return Some(
                    json!({"resultType":"input_required","requestState":"private-mrtr-state-second","inputRequests":{"review":{"method":"elicitation/create","params":{"mode":"form","message":"Potwierdź drugą rundę","requestedSchema":{"type":"object","properties":{"reviewed":{"type":"boolean","title":"Sprawdzone","default":false}},"required":["reviewed"]}}}}}),
                );
            }
            return Some(
                json!({"resultType":"complete","isError":true,"content":[{"type":"text","text":"The human declined or cancelled the form."}]}),
            );
        }
        assert_eq!(state, Some(&json!("private-mrtr-state-second")));
        assert_eq!(
            request["params"]["inputResponses"],
            json!({"review":{"action":"accept","content":{"reviewed":true}}})
        );
        Some(
            json!({"resultType":"complete","content":[{"type":"text","text":"native broker result"}]}),
        )
    }
}
