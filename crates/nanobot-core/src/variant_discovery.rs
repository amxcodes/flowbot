use rig::streaming::StreamedAssistantContent;

pub fn describe_streamed_variant<T>(content: &StreamedAssistantContent<T>) -> &'static str {
    match content {
        StreamedAssistantContent::Text(_) => "text",
        StreamedAssistantContent::ToolCall(_) => "tool_call",
        StreamedAssistantContent::Final(_) => "final",
        _ => "other",
    }
}
