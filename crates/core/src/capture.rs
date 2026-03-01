//! Capture OAuth token from Claude CLI (`claude setup-token`).
//!
//! Runs the `claude` binary as a subprocess (stdout + stderr piped),
//! captures its output, and extracts any JWT or Anthropic OAuth token
//! present in that output. Falls back to manual instructions when the
//! process is interactive and no token is found automatically.

use serde::Serialize;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Result returned by [`capture_claude_token`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureResult {
    /// Captured OAuth access token (JWT or Anthropic format), if found.
    pub access_token: Option<String>,
    /// Refresh token (mirrors access_token for Claude Code compatibility).
    pub refresh_token: Option<String>,
    /// Email detected in the output, if any.
    pub email: Option<String>,
    /// `true` when a token was successfully captured.
    pub success: bool,
    /// Human-readable error or informational message.
    pub error: Option<String>,
    /// Raw captured stdout + stderr output (useful for display in UI).
    pub output: String,
    /// Path to the `claude` binary that was used.
    pub binary_path: Option<String>,
}

// ---------------------------------------------------------------------------
// Token extraction helpers
// ---------------------------------------------------------------------------

/// Try to extract an OAuth / JWT token from a block of text.
///
/// Patterns tried in order:
/// 1. Anthropic OAuth: `sk-ant-oa[a-zA-Z0-9_-]{40,}`
/// 2. Generic JWT:    `eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+`
/// 3. Labeled line:  `[Aa]ccess [Tt]oken[: ]+(.+)`
fn extract_token(text: &str) -> Option<String> {
    // Pattern 1 — Anthropic OAuth token
    let anthropic_re = regex_lite::Regex::new(r"sk-ant-oa[a-zA-Z0-9_\-]{40,}").ok()?;
    if let Some(m) = anthropic_re.find(text) {
        let tok = m.as_str().trim_end_matches(['"', '\'', ' ', '\n', '\r']).to_string();
        if !tok.is_empty() {
            return Some(tok);
        }
    }

    // Pattern 2 — JWT (3 base64url segments)
    let jwt_re = regex_lite::Regex::new(r"eyJ[A-Za-z0-9_\-]+\.[A-Za-z0-9_\-]+\.[A-Za-z0-9_\-]+")
        .ok()?;
    if let Some(m) = jwt_re.find(text) {
        let tok = m.as_str().trim_end_matches(['"', '\'', ' ', '\n', '\r']).to_string();
        if !tok.is_empty() {
            return Some(tok);
        }
    }

    // Pattern 3 — Labeled "Access token: <value>" line
    for line in text.lines() {
        let lower = line.to_lowercase();
        if lower.contains("access token") || lower.contains("access_token") {
            if let Some(pos) = line.find(':') {
                let val = line[pos + 1..].trim().trim_matches(['"', '\'']).to_string();
                if !val.is_empty() && val.len() > 10 {
                    return Some(val);
                }
            }
        }
    }

    None
}

/// Try to extract an email address from a block of text.
fn extract_email(text: &str) -> Option<String> {
    let re = regex_lite::Regex::new(r"[a-zA-Z0-9._%+\-]+@[a-zA-Z0-9.\-]+\.[a-zA-Z]{2,}").ok()?;
    re.find(text).map(|m| m.as_str().to_string())
}

// ---------------------------------------------------------------------------
// Binary discovery
// ---------------------------------------------------------------------------

/// Find the `claude` CLI binary.
///
/// Tries `claude_binary` first, then searches PATH via `which claude` /
/// `where.exe claude` depending on the platform.
pub async fn find_claude_binary(claude_binary: Option<&str>) -> Option<String> {
    // Explicit path provided by caller
    if let Some(path) = claude_binary {
        if !path.is_empty() {
            return Some(path.to_string());
        }
    }

    // Try `which` (Unix) / `where` (Windows) to resolve from PATH
    #[cfg(target_os = "windows")]
    let (cmd, args) = ("where.exe", vec!["claude"]);
    #[cfg(not(target_os = "windows"))]
    let (cmd, args) = ("which", vec!["claude"]);

    if let Ok(out) = Command::new(cmd).args(&args).output().await {
        if out.status.success() {
            let path = String::from_utf8_lossy(&out.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            if !path.is_empty() {
                return Some(path);
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Main capture function
// ---------------------------------------------------------------------------

/// Launch `claude setup-token` and capture any OAuth token from its output.
///
/// # Arguments
/// * `claude_binary` — path to the `claude` CLI, or `None` to search PATH.
/// * `timeout_secs`  — max seconds to wait for the process; defaults to 60.
pub async fn capture_claude_token(
    claude_binary: Option<&str>,
    timeout_secs: u64,
) -> CaptureResult {
    // Resolve binary
    let binary = match find_claude_binary(claude_binary).await {
        Some(b) => b,
        None => {
            return CaptureResult {
                access_token: None,
                refresh_token: None,
                email: None,
                success: false,
                error: Some(
                    "Claude CLI introuvable. Installez-le depuis https://claude.ai/download puis réessayez.".to_string(),
                ),
                output: String::new(),
                binary_path: None,
            };
        }
    };

    info!("capture_claude_token: using binary '{}'", binary);

    // Build command: `claude setup-token`
    // We pipe both stdout and stderr so we capture all output regardless of
    // which fd Claude writes to.
    use std::process::Stdio;
    let mut cmd = Command::new(&binary);
    cmd.arg("setup-token")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Prevent the child from inheriting our terminal (avoids PTY issues)
        .stdin(Stdio::null());

    // On Windows, avoid opening a separate console window
    #[cfg(target_os = "windows")]
    {
        #[allow(unused_imports)]
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW = 0x08000000
        cmd.creation_flags(0x08000000);
    }

    debug!("capture_claude_token: spawning '{} setup-token'", binary);

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return CaptureResult {
                access_token: None,
                refresh_token: None,
                email: None,
                success: false,
                error: Some(format!("Impossible de lancer claude : {}", e)),
                output: String::new(),
                binary_path: Some(binary),
            };
        }
    };

    // Wait with timeout
    let wait_result = timeout(
        Duration::from_secs(timeout_secs.max(5)),
        child.wait_with_output(),
    )
    .await;

    match wait_result {
        // Timed out
        Err(_elapsed) => {
            warn!("capture_claude_token: timed out after {}s", timeout_secs);
            CaptureResult {
                access_token: None,
                refresh_token: None,
                email: None,
                success: false,
                error: Some(format!(
                    "Délai dépassé ({timeout_secs}s). `claude setup-token` attend peut-être \
                     une interaction. Suivez les instructions manuelles ci-dessous."
                )),
                output: String::new(),
                binary_path: Some(binary),
            }
        }

        // Process finished (or error)
        Ok(Err(io_err)) => {
            warn!("capture_claude_token: wait_with_output error: {}", io_err);
            CaptureResult {
                access_token: None,
                refresh_token: None,
                email: None,
                success: false,
                error: Some(format!("Erreur lors de l'exécution : {}", io_err)),
                output: String::new(),
                binary_path: Some(binary),
            }
        }

        Ok(Ok(output)) => {
            // Combine stdout + stderr into one string for display and parsing
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let combined = format!("{}{}", stdout, stderr);

            debug!(
                "capture_claude_token: exit={:?} output_len={}",
                output.status.code(),
                combined.len()
            );

            // Try to extract token
            let token = extract_token(&combined);
            let email = extract_email(&combined);

            if let Some(ref tok) = token {
                info!(
                    "capture_claude_token: token captured (len={}) email={:?}",
                    tok.len(),
                    email
                );
                CaptureResult {
                    refresh_token: Some(tok.clone()),
                    access_token: Some(tok.clone()),
                    email: email.clone(),
                    success: true,
                    error: None,
                    output: combined.trim().to_string(),
                    binary_path: Some(binary),
                }
            } else {
                // No token found — the command may have printed a URL or instructions
                let hint = if combined.contains("http") || combined.contains("url") || combined.contains("URL") {
                    Some("Un navigateur a peut-être été ouvert. Copiez le token affiché dans votre navigateur et utilisez 'Importer un token'.".to_string())
                } else if combined.is_empty() {
                    Some("`claude setup-token` n'a produit aucune sortie. Essayez de lancer `claude setup-token` dans un terminal.".to_string())
                } else {
                    Some("Aucun token trouvé dans la sortie. Copiez manuellement le token depuis la sortie ci-dessus et utilisez 'Importer un token'.".to_string())
                };
                CaptureResult {
                    access_token: None,
                    refresh_token: None,
                    email,
                    success: false,
                    error: hint,
                    output: combined.trim().to_string(),
                    binary_path: Some(binary),
                }
            }
        }
    }
}
