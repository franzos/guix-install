use std::io::{BufRead, BufReader, Write};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::thread;

use anyhow::{Context, Result, bail};

/// Prevents orphaned children from surviving a killed installer (re-parented to init).
fn apply_pdeathsig(cmd: &mut Command) {
    unsafe {
        cmd.pre_exec(|| {
            rustix::process::set_parent_process_death_signal(Some(rustix::process::Signal::Term))
                .map_err(std::io::Error::from)
        });
    }
}

/// Result of a command execution with captured output.
#[derive(Debug)]
pub struct CommandResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// Runs a command, captures stdout/stderr, and returns an error on non-zero exit.
pub fn run_cmd(args: &[&str]) -> Result<CommandResult> {
    let (program, cmd_args) = args
        .split_first()
        .context("run_cmd called with empty args")?;

    let mut cmd = Command::new(program);
    cmd.args(cmd_args);
    apply_pdeathsig(&mut cmd);
    let output = cmd
        .output()
        .with_context(|| format!("failed to execute: {}", args.join(" ")))?;

    let result = CommandResult {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        exit_code: output.status.code().unwrap_or(-1),
    };

    if !output.status.success() {
        bail!(
            "command failed (exit {}): {}\nstderr: {}",
            result.exit_code,
            args.join(" "),
            result.stderr.trim()
        );
    }

    Ok(result)
}

/// Runs a command with inherited stdin/stdout/stderr.
///
/// Used for commands that need user interaction (e.g. cryptsetup passphrase prompts).
/// Returns the exit code.
pub fn run_cmd_interactive(args: &[&str]) -> Result<i32> {
    let (program, cmd_args) = args
        .split_first()
        .context("run_cmd_interactive called with empty args")?;

    let mut cmd = Command::new(program);
    cmd.args(cmd_args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    apply_pdeathsig(&mut cmd);
    let status = cmd
        .status()
        .with_context(|| format!("failed to execute: {}", args.join(" ")))?;

    Ok(status.code().unwrap_or(-1))
}

/// Runs a command with data piped to stdin.
///
/// Useful for commands like `guix archive --authorize` that read a key from stdin,
/// or `chpasswd` that reads `user:password` from stdin.
pub fn run_cmd_with_stdin(args: &[&str], stdin_data: &str) -> Result<CommandResult> {
    let (program, cmd_args) = args
        .split_first()
        .context("run_cmd_with_stdin called with empty args")?;

    let mut cmd = Command::new(program);
    cmd.args(cmd_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apply_pdeathsig(&mut cmd);
    let mut child = cmd
        .spawn()
        .with_context(|| format!("failed to execute: {}", args.join(" ")))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(stdin_data.as_bytes())
            .with_context(|| format!("failed to write stdin for: {}", args.join(" ")))?;
    }

    let output = child
        .wait_with_output()
        .with_context(|| format!("failed to wait for: {}", args.join(" ")))?;

    let result = CommandResult {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        exit_code: output.status.code().unwrap_or(-1),
    };

    if result.exit_code != 0 {
        bail!(
            "command failed (exit {}): {}\nstderr: {}",
            result.exit_code,
            args.join(" "),
            result.stderr.trim()
        );
    }

    Ok(result)
}

/// Retry delay schedule: immediate, 60s, 300s.
const RETRY_DELAYS: &[u64] = &[0, 60, 300];

/// Runs a command with retry logic for transient errors.
///
/// Retries up to `max_retries` times if stderr contains any of the `retry_patterns`.
/// Delays between retries follow: 0s, 60s, 300s (matching px-install behavior).
pub fn run_cmd_with_retry(
    args: &[&str],
    max_retries: u32,
    retry_patterns: &[&str],
) -> Result<CommandResult> {
    let mut last_error = None;

    for attempt in 0..=max_retries {
        match run_cmd(args) {
            Ok(result) => return Ok(result),
            Err(err) => {
                let err_str = format!("{err}");
                let is_retryable = retry_patterns
                    .iter()
                    .any(|pattern| err_str.contains(pattern));

                if !is_retryable || attempt == max_retries {
                    return Err(err);
                }

                let delay_idx = attempt as usize;
                let delay_secs = RETRY_DELAYS
                    .get(delay_idx)
                    .copied()
                    .unwrap_or(*RETRY_DELAYS.last().unwrap_or(&0));

                if delay_secs > 0 {
                    eprintln!(
                        "Retryable error (attempt {}/{}), waiting {}s: {}",
                        attempt + 1,
                        max_retries,
                        delay_secs,
                        err_str.lines().next().unwrap_or(&err_str)
                    );
                    thread::sleep(std::time::Duration::from_secs(delay_secs));
                }

                last_error = Some(err);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("run_cmd_with_retry: no attempts made")))
}

/// Runs a command and calls a callback for each line of stdout.
///
/// Useful for streaming progress output from long-running commands like `guix system init`.
pub fn run_cmd_streaming(args: &[&str], on_line: &mut dyn FnMut(&str)) -> Result<CommandResult> {
    let (program, cmd_args) = args
        .split_first()
        .context("run_cmd_streaming called with empty args")?;

    let mut cmd = Command::new(program);
    cmd.args(cmd_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apply_pdeathsig(&mut cmd);
    let mut child = cmd
        .spawn()
        .with_context(|| format!("failed to execute: {}", args.join(" ")))?;

    let stdout = child.stdout.take().context("failed to capture stdout")?;
    let stderr = child.stderr.take().context("failed to capture stderr")?;

    // Read stderr in a background thread
    let stderr_handle = thread::spawn(move || {
        let reader = BufReader::new(stderr);
        let mut output = String::new();
        for line in reader.lines().map_while(Result::ok) {
            output.push_str(&line);
            output.push('\n');
        }
        output
    });

    // Stream stdout line-by-line
    let reader = BufReader::new(stdout);
    let mut stdout_output = String::new();
    for line in reader.lines() {
        let line = line.context("failed to read stdout line")?;
        on_line(&line);
        stdout_output.push_str(&line);
        stdout_output.push('\n');
    }

    let status = child.wait().context("failed to wait for child process")?;
    let stderr_output = stderr_handle
        .join()
        .unwrap_or_else(|_| "failed to read stderr".into());

    let result = CommandResult {
        stdout: stdout_output,
        stderr: stderr_output,
        exit_code: status.code().unwrap_or(-1),
    };

    if !status.success() {
        bail!(
            "command failed (exit {}): {}\nstderr: {}",
            result.exit_code,
            args.join(" "),
            result.stderr.trim()
        );
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_cmd_echo() {
        let result = run_cmd(&["echo", "hello"]).unwrap();
        assert_eq!(result.stdout.trim(), "hello");
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn run_cmd_failure() {
        let result = run_cmd(&["false"]);
        assert!(result.is_err());
    }

    #[test]
    fn run_cmd_empty_args() {
        let result = run_cmd(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn run_cmd_interactive_true() {
        let code = run_cmd_interactive(&["true"]).unwrap();
        assert_eq!(code, 0);
    }

    #[test]
    fn run_cmd_interactive_false() {
        let code = run_cmd_interactive(&["false"]).unwrap();
        assert_ne!(code, 0);
    }

    #[test]
    fn run_cmd_with_stdin_echo() {
        // `tr` reads from stdin and transforms — good way to verify stdin piping
        let result = run_cmd_with_stdin(&["tr", "a-z", "A-Z"], "hello").unwrap();
        assert_eq!(result.stdout.trim(), "HELLO");
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn run_cmd_with_stdin_empty_args() {
        let result = run_cmd_with_stdin(&[], "data");
        assert!(result.is_err());
    }

    #[test]
    fn run_cmd_with_stdin_failure() {
        let result = run_cmd_with_stdin(&["false"], "data");
        assert!(result.is_err());
    }

    #[test]
    fn run_cmd_streaming_echo() {
        let mut lines = Vec::new();
        let result = run_cmd_streaming(&["echo", "line1"], &mut |line| {
            lines.push(line.to_string());
        })
        .unwrap();
        assert_eq!(lines, vec!["line1"]);
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn run_cmd_streaming_failure() {
        let result = run_cmd_streaming(&["false"], &mut |_| {});
        assert!(result.is_err());
    }

    #[test]
    fn run_cmd_with_retry_immediate_success() {
        let result = run_cmd_with_retry(&["echo", "ok"], 3, &["TLS error"]).unwrap();
        assert_eq!(result.stdout.trim(), "ok");
    }

    #[test]
    fn run_cmd_with_retry_no_pattern_match() {
        // Fails immediately without retry since the error doesn't match patterns
        let result = run_cmd_with_retry(&["false"], 3, &["TLS error"]);
        assert!(result.is_err());
    }
}
