use std::env;
use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};
use std::path::PathBuf;

const QWEN_PROMPT: &str = "Generate a git commit message for the following changes. Follow these rules strictly:
1. First line is the subject: max 50 characters, imperative mood, no period at end
2. Second line must be blank
3. Body paragraphs start on line 3: wrap all lines at 72 characters
4. The body should explain WHAT changed and WHY (not how)

Output only the commit message, nothing else:

";

fn main() {
    let args: Vec<String> = env::args().collect();
    
    // Check if --amend flag is present (we'll regenerate the message for amend)
    let is_amend = args.iter().any(|arg| arg == "--amend");

    // Check if user wants to skip the qwen generation (e.g., --fixup, etc.)
    let skip_generation = args.iter().enumerate().any(|(i, arg)| {
        // Flags that don't take values
        if arg == "--fixup" || arg == "--squash" ||
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

    // Check if -a or --all flag is present (commit all tracked modified files)
    let include_all = args.iter().any(|arg| arg == "-a" || arg == "--all");

    // Check if -s or --signoff flag is present
    let include_signoff = args.iter().any(|arg| arg == "-s" || arg == "--signoff");

    // Get git diff to generate commit message
    let diff_output = match get_git_diff(include_all, is_amend) {
        Ok(output) => output,
        Err(e) => {
            eprintln!("Error: Failed to get git diff: {}", e);
            std::process::exit(1);
        }
    };

    if diff_output.trim().is_empty() {
        if is_amend {
            eprintln!("Error: No changes found in HEAD commit.");
            eprintln!("Cannot generate commit message for an empty commit.");
        } else if include_all {
            eprintln!("Error: No changes to commit.");
            eprintln!("Nothing to commit (no modified tracked files).");
        } else {
            eprintln!("Error: No changes staged for commit.");
            eprintln!("Use 'git add' to stage changes, or use '-a' to commit all modified tracked files.");
        }
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
    let temp_file = match create_commit_msg_file(&commit_msg, include_signoff) {
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

fn get_git_diff(include_all: bool, is_amend: bool) -> Result<String, String> {
    if is_amend {
        // When amending, get the diff of HEAD commit plus any staged/unstaged changes
        // This shows all changes that will be in the amended commit
        let head_diff = Command::new("git")
            .args(&["diff", "HEAD~1", "HEAD"])
            .output()
            .map_err(|e| format!("Failed to execute git diff HEAD~1 HEAD: {}", e))?;

        if !head_diff.status.success() {
            return Err("git diff command failed (is there a parent commit?)".to_string());
        }

        let head_diff_str = String::from_utf8(head_diff.stdout)
            .map_err(|e| format!("Invalid UTF-8 in git diff output: {}", e))?;

        // Also get any additional staged changes that will be added to the amend
        let staged = Command::new("git")
            .args(&["diff", "--cached"])
            .output()
            .map_err(|e| format!("Failed to execute git diff --cached: {}", e))?;

        let staged_str = if staged.status.success() {
            String::from_utf8(staged.stdout)
                .map_err(|e| format!("Invalid UTF-8 in git diff output: {}", e))?
        } else {
            String::new()
        };

        // If -a flag is also used, include unstaged changes too
        let unstaged_str = if include_all {
            let unstaged = Command::new("git")
                .args(&["diff"])
                .output()
                .map_err(|e| format!("Failed to execute git diff: {}", e))?;

            if unstaged.status.success() {
                String::from_utf8(unstaged.stdout)
                    .map_err(|e| format!("Invalid UTF-8 in git diff output: {}", e))?
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        Ok(format!("{}{}{}", head_diff_str, staged_str, unstaged_str))
    } else if include_all {
        // When -a flag is used, we need to show what would be committed:
        // both staged changes AND unstaged changes to tracked files
        let staged = Command::new("git")
            .args(&["diff", "--cached"])
            .output()
            .map_err(|e| format!("Failed to execute git diff --cached: {}", e))?;

        let unstaged = Command::new("git")
            .args(&["diff"])
            .output()
            .map_err(|e| format!("Failed to execute git diff: {}", e))?;

        if !staged.status.success() || !unstaged.status.success() {
            return Err("git diff command failed".to_string());
        }

        let staged_str = String::from_utf8(staged.stdout)
            .map_err(|e| format!("Invalid UTF-8 in git diff output: {}", e))?;
        let unstaged_str = String::from_utf8(unstaged.stdout)
            .map_err(|e| format!("Invalid UTF-8 in git diff output: {}", e))?;

        // Combine both diffs
        Ok(format!("{}{}", staged_str, unstaged_str))
    } else {
        // Get only staged changes
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
        let prompt = format!("{}{}", QWEN_PROMPT, diff);
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

    // Strip markdown code block formatting if present
    let message = message.trim();
    let message = message.strip_prefix("```").unwrap_or(message);
    let message = message.strip_suffix("```").unwrap_or(message);
    // Also handle if there's a language identifier like ```text
    let message = if message.starts_with('\n') {
        &message[1..]
    } else if let Some(pos) = message.find('\n') {
        // Check if first line looks like a language identifier (no spaces, short)
        let first_line = &message[..pos];
        if !first_line.contains(' ') && first_line.len() < 20 {
            &message[pos + 1..]
        } else {
            message
        }
    } else {
        message
    };

    let message = message.trim().to_string();
    Ok(format_commit_message(&message))
}

fn format_commit_message(message: &str) -> String {
    let lines: Vec<&str> = message.lines().collect();

    if lines.is_empty() {
        return String::new();
    }

    // Truncate subject line to 50 characters
    let subject = if lines[0].len() > 50 {
        &lines[0][..50]
    } else {
        lines[0]
    };

    let mut result = subject.trim_end().to_string();

    // If there's more content, add blank line and wrap body at 72 chars
    if lines.len() > 1 {
        // Skip any existing blank lines after subject
        let body_start = lines.iter().skip(1).position(|l| !l.trim().is_empty());

        if let Some(start_idx) = body_start {
            result.push_str("\n\n");

            let body_lines = &lines[start_idx + 1..];
            let body_text = body_lines.join("\n");
            let wrapped_body = wrap_text(&body_text, 72);
            result.push_str(&wrapped_body);
        }
    }

    result
}

fn wrap_text(text: &str, max_width: usize) -> String {
    let mut result = String::new();

    for paragraph in text.split("\n\n") {
        if !result.is_empty() {
            result.push_str("\n\n");
        }

        // Preserve lines that are already short or intentionally formatted
        let mut wrapped_paragraph = String::new();
        for line in paragraph.lines() {
            if !wrapped_paragraph.is_empty() {
                wrapped_paragraph.push(' ');
            }
            wrapped_paragraph.push_str(line.trim());
        }

        // Now wrap the joined text
        let words: Vec<&str> = wrapped_paragraph.split_whitespace().collect();
        let mut current_line = String::new();

        for word in words {
            if current_line.is_empty() {
                current_line = word.to_string();
            } else if current_line.len() + 1 + word.len() <= max_width {
                current_line.push(' ');
                current_line.push_str(word);
            } else {
                if !result.is_empty() && !result.ends_with("\n\n") {
                    result.push('\n');
                }
                result.push_str(&current_line);
                current_line = word.to_string();
            }
        }

        if !current_line.is_empty() {
            if !result.is_empty() && !result.ends_with("\n\n") {
                result.push('\n');
            }
            result.push_str(&current_line);
        }
    }

    result
}

fn get_signoff_line() -> Result<String, String> {
    let name = Command::new("git")
        .args(&["config", "user.name"])
        .output()
        .map_err(|e| format!("Failed to get user.name: {}", e))?;

    let email = Command::new("git")
        .args(&["config", "user.email"])
        .output()
        .map_err(|e| format!("Failed to get user.email: {}", e))?;

    if !name.status.success() || !email.status.success() {
        return Err("Failed to get git user config".to_string());
    }

    let name = String::from_utf8(name.stdout)
        .map_err(|e| format!("Invalid UTF-8 in user.name: {}", e))?
        .trim()
        .to_string();

    let email = String::from_utf8(email.stdout)
        .map_err(|e| format!("Invalid UTF-8 in user.email: {}", e))?
        .trim()
        .to_string();

    Ok(format!("Signed-off-by: {} <{}>\n", name, email))
}

fn create_commit_msg_file(message: &str, include_signoff: bool) -> Result<PathBuf, String> {
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

    // Add Signed-off-by line if -s flag was used
    if include_signoff {
        let signoff = get_signoff_line()?;
        write!(file, "\n\n{}", signoff)
            .map_err(|e| format!("Failed to write signoff: {}", e))?;
    }

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
    // Try to clean up the temporary file, ignoring errors if it was
    // already deleted or is inaccessible for any reason
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
