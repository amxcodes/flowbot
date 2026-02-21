use nanobot_core::tools::filesystem::{edit_file, EditFileArgs};

fn main() {
    let args = EditFileArgs {
        path: "foo.txt".to_string(),
        old_text: "a".to_string(),
        new_text: "b".to_string(),
        all_occurrences: false,
    };

    let _ = edit_file(args);
}
