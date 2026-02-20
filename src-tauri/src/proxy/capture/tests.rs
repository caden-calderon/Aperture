use super::sse::{
    extract_anthropic_final_response, extract_openai_chat_final_response,
    extract_openai_responses_final_response,
};
use super::*;

fn sample_anthropic_request() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "system": "Be helpful.",
        "messages": [
            {"role": "user", "content": "Hello!"},
            {"role": "assistant", "content": "Hi there!"}
        ]
    }))
    .unwrap()
}

fn sample_anthropic_response() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "id": "msg_123",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-20250514",
        "content": [{"type": "text", "text": "I'm doing well!"}],
        "usage": {"input_tokens": 20, "output_tokens": 10}
    }))
    .unwrap()
}

#[test]
fn test_capture_request_stores_exchange() {
    let store = CaptureStore::default();
    let body = sample_anthropic_request();

    let result = store.capture_request("req_1", "/v1/messages", &body);
    assert!(result.is_some());

    let exchange = store.get_exchange("req_1").unwrap();
    assert_eq!(exchange.provider, Provider::Anthropic);
    assert_eq!(exchange.status, ExchangeStatus::Pending);
    assert_eq!(exchange.request_blocks.len(), 3); // system + user + assistant
}

#[test]
fn test_capture_request_non_api_endpoint() {
    let store = CaptureStore::default();
    let result = store.capture_request("req_1", "/health", b"{}");
    assert!(result.is_none());
}

#[test]
fn test_capture_response_completes_exchange() {
    let store = CaptureStore::default();
    let req_body = sample_anthropic_request();
    store.capture_request("req_1", "/v1/messages", &req_body);

    let resp_body = sample_anthropic_response();
    let result = store.capture_response("req_1", 200, &resp_body);
    assert!(result.is_some());

    let exchange = result.unwrap();
    assert_eq!(exchange.status, ExchangeStatus::Complete);
    assert_eq!(exchange.response_blocks.len(), 1);
    assert!(exchange.usage.is_some());
    assert_eq!(exchange.usage.as_ref().unwrap().input_tokens, 20);
}

#[test]
fn test_capture_response_error_status() {
    let store = CaptureStore::default();
    let req_body = sample_anthropic_request();
    store.capture_request("req_1", "/v1/messages", &req_body);

    let result = store.capture_response("req_1", 500, b"Internal Server Error");
    assert!(result.is_some());
    assert_eq!(result.unwrap().status, ExchangeStatus::Failed);
}

#[test]
fn test_sse_chunk_accumulation() {
    let store = CaptureStore::default();
    let req_body = sample_anthropic_request();
    store.capture_request("req_1", "/v1/messages", &req_body);

    let bytes = store.append_sse_chunk("req_1", b"data: {\"type\":\"message_start\"}\n\n");
    assert!(bytes > 0);

    let exchange = store.get_exchange("req_1").unwrap();
    assert_eq!(exchange.status, ExchangeStatus::Streaming);
}

#[test]
fn test_extract_anthropic_sse() {
    let sse = "\
data: {\"type\":\"message_start\",\"message\":{\"model\":\"claude-sonnet-4-20250514\",\"usage\":{\"input_tokens\":15}}}\n\
\n\
data: {\"type\":\"content_block_start\",\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\
\n\
data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello \"}}\n\
\n\
data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"world!\"}}\n\
\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":5}}\n\
\n\
data: {\"type\":\"message_stop\"}\n\
\n";

    let result = extract_anthropic_final_response(sse).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&result).unwrap();

    assert_eq!(json["content"][0]["text"], "Hello world!");
    assert_eq!(json["usage"]["input_tokens"], 15);
    assert_eq!(json["usage"]["output_tokens"], 5);
}

#[test]
fn test_extract_openai_sse() {
    let sse = "\
data: {\"model\":\"gpt-4\",\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\
\n\
data: {\"choices\":[{\"delta\":{\"content\":\"Hi \"}}]}\n\
\n\
data: {\"choices\":[{\"delta\":{\"content\":\"there!\"}}]}\n\
\n\
data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":3,\"total_tokens\":13}}\n\
\n\
data: [DONE]\n\
\n";

    let result = extract_openai_chat_final_response(sse).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&result).unwrap();

    assert_eq!(json["choices"][0]["message"]["content"], "Hi there!");
    assert_eq!(json["usage"]["prompt_tokens"], 10);
    assert_eq!(json["usage"]["completion_tokens"], 3);
}

#[test]
fn test_extract_openai_responses_sse() {
    let sse = "\
data: {\"type\":\"response.output_text.delta\",\"model\":\"gpt-4.1\",\"delta\":\"Hi \"}\n\
\n\
data: {\"type\":\"response.output_text.delta\",\"delta\":\"there!\"}\n\
\n\
data: {\"type\":\"response.completed\",\"usage\":{\"input_tokens\":11,\"output_tokens\":4}}\n\
\n\
data: [DONE]\n\
\n";

    let result = extract_openai_responses_final_response(sse).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&result).unwrap();

    assert_eq!(json["output"][0]["content"][0]["text"], "Hi there!");
    assert_eq!(json["usage"]["input_tokens"], 11);
    assert_eq!(json["usage"]["output_tokens"], 4);
}

#[test]
fn test_all_blocks_returns_completed() {
    let store = CaptureStore::default();
    let req_body = sample_anthropic_request();
    store.capture_request("req_1", "/v1/messages", &req_body);

    // Not complete yet - should return empty
    assert!(store.all_blocks().is_empty());

    let resp_body = sample_anthropic_response();
    store.capture_response("req_1", 200, &resp_body);

    let blocks = store.all_blocks();
    assert_eq!(blocks.len(), 4); // 3 request + 1 response
}

#[test]
fn test_is_api_endpoint() {
    assert!(is_api_endpoint("/v1/messages"));
    assert!(is_api_endpoint("/v1/chat/completions"));
    assert!(is_api_endpoint("/v1/responses"));
    // Bare paths (no /v1/ prefix)
    assert!(is_api_endpoint("/messages"));
    assert!(is_api_endpoint("/chat/completions"));
    assert!(is_api_endpoint("/responses"));
    // Non-API paths
    assert!(!is_api_endpoint("/health"));
    assert!(!is_api_endpoint("/v1/models"));
}
