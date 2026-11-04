//! One complete provider resource for native/streamed parity fixtures.
use serde_json::{Value, json};

pub fn response_resource(text: &str) -> Value {
    json!({"id":"resp-native","status":"completed","model":"gpt-6.1-sol","output":[
        {"type":"reasoning","id":"rs-native","summary":[{"type":"summary_text","text":"checked"}],"encrypted_content":"opaque"},
        {"type":"message","id":"msg-native","role":"assistant","status":"completed","phase":"final_answer","content":[{"type":"output_text","text":text,"annotations":[]}]},
        {"type":"function_call","id":"fc-native","call_id":"call-native","name":"lookup","arguments":"{\"id\":42}","status":"completed"}
    ],"usage":{"input_tokens":100,"output_tokens":20,"input_tokens_details":{"cached_tokens":30},"output_tokens_details":{"reasoning_tokens":5},"cost":0.001}})
}
