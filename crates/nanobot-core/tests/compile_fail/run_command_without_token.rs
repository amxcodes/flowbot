use nanobot_core::tools::commands::{run_command, RunCommandArgs};

fn main() {
    let args = RunCommandArgs {
        command: "echo".to_string(),
        args: vec!["hello".to_string()],
        use_docker: false,
        docker_image: None,
    };

    let _ = run_command(args);
}
