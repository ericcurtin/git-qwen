use std::env;
use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = env::args().collect();
    
    // Check if user wants to skip the qwen generation (e.g., --amend, --fixup, etc.)
    let skip_generation = args.iter().enumerate().any(|(i, arg)| {
        // Flags that don't take values
        if arg == "--amend" || arg == "--fixup" || arg == "--squash" || 
           arg == "--help" || arg == "-h" || arg == "--version" {
            return true;
        }
        
        // Flags with values (can be --flag=value or --flag value)
        if arg.starts_with("--fixup=") || arg.starts_with("--squash=") ||
           arg.starts_with("--message=") || arg.starts_with("--file=") ||
           arg.starts_with("--reuse-message=") || arg.starts_with("--reedit-message=") {
            return true;
        }
        
        // Short flags that take values: check if there's a next argument
        if (arg == "-m" || arg == "-F" || arg == "-C" || arg == "-c") && i + 1 < args.len() {
            return true;
        }
        
        false
    });

    if skip_generation {
        // If user is providing their own message or amending, just pass through to git commit
        execute_git_commit(&args[1..]);
        return;
    }

    // Get git diff to generate commit message
    let diff_output = match get_git_diff() {
        Ok(output) => output,
        Err(e) => {
            eprintln!("Error: Failed to get git diff: {}", e);
            std::process::exit(1);
        }
    };

    if diff_output.trim().is_empty() {
        eprintln!("Error: No changes staged for commit.");
        eprintln!("Use 'git add' to stage changes.");
        std::process::exit(1);
    }

    // Generate commit message using qwen
    let commit_msg = match generate_commit_message(&diff_output) {
        Ok(msg) => msg,
        Err(e) => {
            eprintln!("Error: Failed to generate commit message: {}", e);
            eprintln!("Make sure 'qwen' is installed and available in PATH.");
            std::process::exit(1);
        }
    };

    // Create temporary file with the generated message
    let temp_file = match create_commit_msg_file(&commit_msg) {
        Ok(path) => path,
        Err(e) => {
            eprintln!("Error: Failed to create temporary file: {}", e);
            std::process::exit(1);
        }
    };

    // Open editor with the temporary file
    let editor = get_editor();
    if let Err(e) = open_editor(&editor, &temp_file) {
        eprintln!("Error: Failed to open editor: {}", e);
        cleanup_temp_file(&temp_file);
        std::process::exit(1);
    }

    // Read the edited message
    let edited_msg = match fs::read_to_string(&temp_file) {
        Ok(msg) => msg,
        Err(e) => {
            eprintln!("Error: Failed to read edited message: {}", e);
            cleanup_temp_file(&temp_file);
            std::process::exit(1);
        }
    };

    // Clean up temp file
    cleanup_temp_file(&temp_file);

    // Check if message is empty
    let trimmed_msg = edited_msg.lines()
        .filter(|line| !line.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();

    if trimmed_msg.is_empty() {
        eprintln!("Aborting commit due to empty commit message.");
        std::process::exit(1);
    }

    // Execute git commit with the message and any additional arguments
    execute_git_commit_with_message(&trimmed_msg, &args[1..]);
}

fn get_git_diff() -> Result<String, String> {
    // Get staged changes
    let output = Command::new("git")
        .args(&["diff", "--cached"])
        .output()
        .map_err(|e| format!("Failed to execute git diff: {}", e))?;

    if !output.status.success() {
        return Err("git diff command failed".to_string());
    }

    String::from_utf8(output.stdout)
        .map_err(|e| format!("Invalid UTF-8 in git diff output: {}", e))
}

fn generate_commit_message(diff: &str) -> Result<String, String> {
    let mut child = Command::new("qwen")
        .arg("-y")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn qwen: {}", e))?;

    // Write the prompt to qwen's stdin
    if let Some(mut stdin) = child.stdin.take() {
        let prompt = format!("Generate a concise git commit message for the following changes. Only output the commit message, nothing else:\n\n{}", diff);
        stdin.write_all(prompt.as_bytes())
            .map_err(|e| format!("Failed to write to qwen stdin: {}", e))?;
    }

    let output = child.wait_with_output()
        .map_err(|e| format!("Failed to wait for qwen: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("qwen command failed: {}", stderr));
    }

    let message = String::from_utf8(output.stdout)
        .map_err(|e| format!("Invalid UTF-8 in qwen output: {}", e))?;

    Ok(message.trim().to_string())
}

fn create_commit_msg_file(message: &str) -> Result<PathBuf, String> {
    let git_dir = Command::new("git")
        .args(&["rev-parse", "--git-dir"])
        .output()
        .map_err(|e| format!("Failed to get git directory: {}", e))?;

    if !git_dir.status.success() {
        return Err("Failed to determine git directory".to_string());
    }

    let git_dir_path = String::from_utf8(git_dir.stdout)
        .map_err(|e| format!("Invalid UTF-8 in git dir path: {}", e))?
        .trim()
        .to_string();

    let commit_msg_path = PathBuf::from(git_dir_path).join("COMMIT_EDITMSG");

    let mut file = fs::File::create(&commit_msg_path)
        .map_err(|e| format!("Failed to create commit message file: {}", e))?;

    // Write the generated message
    file.write_all(message.as_bytes())
        .map_err(|e| format!("Failed to write to commit message file: {}", e))?;

    // Add git commit template comments
    let status_output = Command::new("git")
        .args(&["status", "--porcelain"])
        .output()
        .map_err(|e| format!("Failed to get git status: {}", e))?;

    if status_output.status.success() {
        let status = String::from_utf8_lossy(&status_output.stdout);
        
        // Get current branch name
        let branch_output = Command::new("git")
            .args(&["branch", "--show-current"])
            .output()
            .ok();
        
        let branch_name = branch_output
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "detached HEAD".to_string());
        
        writeln!(file, "\n# Please enter the commit message for your changes. Lines starting")
            .map_err(|e| format!("Failed to write to file: {}", e))?;
        writeln!(file, "# with '#' will be ignored, and an empty message aborts the commit.")
            .map_err(|e| format!("Failed to write to file: {}", e))?;
        writeln!(file, "#")
            .map_err(|e| format!("Failed to write to file: {}", e))?;
        writeln!(file, "# On branch {}", branch_name)
            .map_err(|e| format!("Failed to write to file: {}", e))?;
        writeln!(file, "# Changes to be committed:")
            .map_err(|e| format!("Failed to write to file: {}", e))?;
        
        for line in status.lines() {
            writeln!(file, "# {}", line)
                .map_err(|e| format!("Failed to write to file: {}", e))?;
        }
    }

    Ok(commit_msg_path)
}

fn get_editor() -> String {
    // Check environment variables in order of precedence
    env::var("GIT_EDITOR")
        .or_else(|_| env::var("VISUAL"))
        .or_else(|_| env::var("EDITOR"))
        .unwrap_or_else(|_| {
            // Default editors by platform
            if cfg!(target_os = "windows") {
                "notepad".to_string()
            } else {
                "vi".to_string()
            }
        })
}

fn open_editor(editor: &str, file_path: &PathBuf) -> Result<(), String> {
    let status = Command::new(editor)
        .arg(file_path)
        .status()
        .map_err(|e| format!("Failed to execute editor: {}", e))?;

    if !status.success() {
        return Err(format!("Editor exited with non-zero status: {}", status));
    }

    Ok(())
}

fn cleanup_temp_file(path: &PathBuf) {
    // Try to clean up the temporary file, but ignore errors
    // (the file might be needed by git or already deleted)
    let _ = fs::remove_file(path);
}

fn execute_git_commit(args: &[String]) {
    let status = Command::new("git")
        .arg("commit")
        .args(args)
        .status()
        .unwrap_or_else(|e| {
            eprintln!("Failed to execute git commit: {}", e);
            std::process::exit(1);
        });

    std::process::exit(status.code().unwrap_or(1));
}

fn execute_git_commit_with_message(message: &str, additional_args: &[String]) {
    let status = Command::new("git")
        .arg("commit")
        .arg("-m")
        .arg(message)
        .args(additional_args)
        .status()
        .unwrap_or_else(|e| {
            eprintln!("Failed to execute git commit: {}", e);
            std::process::exit(1);
        });

    std::process::exit(status.code().unwrap_or(1));
}
