# git-qwen

A tool that automatically generates git commit messages using `qwen -y`.

## Overview

`git-qwen` is a wrapper around `git commit` that uses the Qwen AI model to automatically generate commit messages based on your staged changes. It opens a text editor (just like `git commit`) with a pre-populated message that you can edit before committing.

## Prerequisites

- git
- `qwen` command-line tool

## Installation

```bash
cargo build --release
cargo install --path .
```

## Usage

Use `git-qwen` just like you would use `git commit`:

```bash
# Stage your changes
git add .

# Generate commit message and commit
git-qwen

# With additional git commit flags
git-qwen --signoff
git-qwen -a
git-qwen --dry-run
```

## How It Works

1. **Checks for staged changes**: Ensures you have changes ready to commit
2. **Generates message**: Runs `qwen -y` with your git diff to generate a commit message
3. **Opens editor**: Opens your preferred text editor with the generated message
4. **Commits**: After you save and close the editor, commits with the message

## Command-Line Arguments

`git-qwen` supports all the same command-line arguments as `git commit`. Some special cases:

- `--help`, `--version`: Passed directly to git commit
- `-m`, `--message`, `-F`, `--file`: Bypasses qwen generation and uses your provided message
- `--amend`, `--fixup`, `--squash`: Bypasses qwen generation (these already have context)

## Editor Configuration

The tool respects the same editor configuration as git:

1. `GIT_EDITOR` environment variable
2. `VISUAL` environment variable
3. `EDITOR` environment variable
4. Falls back to `vi` on Unix-like systems or `notepad` on Windows

## Example

```bash
$ git add src/main.rs
$ git-qwen

# Editor opens with generated message:
# "Add initial implementation of git-qwen tool
#
# - Implement git diff parsing
# - Add qwen integration for message generation
# - Support all git commit flags
# "

# Edit the message if needed, save, and close
# The commit is created with your message
```

