use rig::streaming::StreamedAssistantContent;
use rig::completion::CompletionRequest;

fn test() {
    let s: StreamedAssistantContent<String> = todo!();
    match s {
        StreamedAssistantContent::MakeCompilerTellMeVariants => {}
    }

    let c: CompletionRequest = todo!();
    let _: () = c.additional_params; // Compiler will say: expected (), found TYPE
}
