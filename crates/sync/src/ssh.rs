//! SSH Sync — push credentials to remote machines via SCP/SSH.
//!
//! Uses `scp` and `ssh` system binaries (the same approach as `capture.rs` which
//! uses `tokio::process::Command`). No external SSH crate is required.
//!
//! # Platform support
//!
//! SSH sync is only supported on Unix-like systems where `scp` and `ssh` are
//! available on PATH. On Windows this module compiles but every function returns
//! [`SyncError::Connection`] with a clear "not supported" message.

use crate::error::{Result, SyncError};
#[cfg(unix)]
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for an SSH/SCP connection.
#[derive(Debug, Clone)]
pub struct SshConfig {
    /// Remote hostname or IP address.
    pub host: String,
    /// SSH port (default: 22).
    pub port: u16,
    /// Remote username.
    pub username: String,
    /// Path to a private key file for public-key authentication, if any.
    pub identity_path: Option<String>,
    /// Whether to enforce strict host key checking.
    ///
    /// Default is `false` for backward-compatibility with V2 behaviour.
    /// Set to `true` in production environments to prevent MITM attacks.
    pub strict_host_checking: bool,
}

impl SshConfig {
    /// Create a new [`SshConfig`], validating `host` and `username`.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Connection`] if:
    /// - `host` contains whitespace or starts with `-`
    /// - `username` contains whitespace, `@`, or starts with `-`
    pub fn new(
        host: impl Into<String>,
        port: u16,
        username: impl Into<String>,
        identity_path: Option<String>,
    ) -> Result<Self> {
        let host: String = host.into();
        let username: String = username.into();

        if host.contains(char::is_whitespace) || host.starts_with('-') {
            return Err(SyncError::Connection(format!(
                "Invalid SSH host: '{host}'"
            )));
        }
        if username.contains(char::is_whitespace)
            || username.contains('@')
            || username.starts_with('-')
        {
            return Err(SyncError::Connection(format!(
                "Invalid SSH username: '{username}'"
            )));
        }

        Ok(Self {
            host,
            port,
            username,
            identity_path,
            strict_host_checking: false,
        })
    }

    /// Return the `user@host` destination string used by `scp`/`ssh`.
    #[cfg(any(unix, test))]
    fn destination(&self) -> String {
        format!("{}@{}", self.username, self.host)
    }
}

// ---------------------------------------------------------------------------
// SshSync
// ---------------------------------------------------------------------------

/// SSH sync client — wraps an [`SshConfig`] and exposes async operations.
#[derive(Debug, Clone)]
pub struct SshSync {
    pub config: SshConfig,
}

impl SshSync {
    /// Create a new [`SshSync`] from the given configuration.
    pub fn new(config: SshConfig) -> Self {
        Self { config }
    }

    /// Build the common SSH option arguments for both `ssh` and `scp`.
    ///
    /// `port_flag` is `-P` for `scp` and `-p` for `ssh`.
    ///
    /// These mirror the options used by the V2 Python `ssh_sync_agent.py`.
    #[cfg(any(unix, test))]
    fn build_common_opts(&self, port_flag: &str) -> Vec<String> {
        let strict = if self.config.strict_host_checking {
            "yes"
        } else {
            "no"
        };

        let mut opts = vec![
            // Never interactively ask for passwords (fail fast instead)
            "-o".to_string(),
            "BatchMode=yes".to_string(),
            // Short connection timeout so we don't hang
            "-o".to_string(),
            "ConnectTimeout=5".to_string(),
            // StrictHostKeyChecking controlled by config field.
            // Default: no — matches V2 behaviour. Set strict_host_checking=true in production.
            "-o".to_string(),
            format!("StrictHostKeyChecking={strict}"),
            // Port (flag differs between scp and ssh)
            port_flag.to_string(),
            self.config.port.to_string(),
        ];

        if let Some(ref key) = self.config.identity_path {
            opts.push("-i".to_string());
            opts.push(key.clone());
        }

        opts
    }

    /// Build common SSH option arguments for the `scp` binary (`-P` for port).
    #[cfg(any(unix, test))]
    fn common_ssh_opts(&self) -> Vec<String> {
        self.build_common_opts("-P")
    }

    /// Build common SSH option arguments for the `ssh` binary (`-p` for port).
    #[cfg(any(unix, test))]
    fn common_ssh_opts_for_ssh_binary(&self) -> Vec<String> {
        self.build_common_opts("-p")
    }

    /// Push a local file to a remote path via `scp`.
    ///
    /// # Arguments
    /// * `local_path`  — absolute or relative path of the file to copy.
    /// * `remote_path` — destination path on the remote host (e.g. `~/.config/credentials.json`).
    ///
    /// # Errors
    /// Returns [`SyncError::Connection`] when:
    /// - the platform is Windows,
    /// - `local_path` does not exist,
    /// - `scp` exits with a non-zero status.
    pub async fn push_credentials(
        &self,
        local_path: &str,
        remote_path: &str,
    ) -> Result<()> {
        // ------------------------------------------------------------------
        // Windows: not supported
        // ------------------------------------------------------------------
        #[cfg(target_os = "windows")]
        {
            let _ = (local_path, remote_path); // suppress unused-variable warnings
            return Err(SyncError::Connection(
                "SSH sync is not supported on Windows. \
                 Use the P2P LAN sync instead."
                    .to_string(),
            ));
        }

        // ------------------------------------------------------------------
        // Unix: proceed with scp
        // ------------------------------------------------------------------
        #[cfg(unix)]
        {
            use std::path::Path;

            // Validate that the source file exists before attempting the copy.
            if !Path::new(local_path).exists() {
                return Err(SyncError::Connection(format!(
                    "Source file does not exist: {local_path}"
                )));
            }

            let remote_dest = format!("{}:{}", self.config.destination(), remote_path);

            // Collect arguments: common opts then source then destination
            let mut args: Vec<String> = self.common_ssh_opts();
            args.push(local_path.to_string());
            args.push(remote_dest.clone());

            debug!(
                "ssh_sync: scp {} → {}",
                local_path, remote_dest
            );

            let output = tokio::process::Command::new("scp")
                .args(&args)
                .output()
                .await
                .map_err(|e| SyncError::Connection(format!("Failed to spawn scp: {e}")))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                warn!("ssh_sync: scp failed — {stderr}");
                return Err(SyncError::Connection(format!(
                    "scp failed (exit {:?}): {stderr}",
                    output.status.code()
                )));
            }

            info!(
                "ssh_sync: pushed '{}' → '{}' on {}",
                local_path, remote_path, self.config.host
            );

            Ok(())
        }
    }

    /// Test whether an SSH connection to the remote host can be established.
    ///
    /// Runs `ssh … exit` with a short `ConnectTimeout` and returns `Ok(())` on
    /// success or a [`SyncError::Connection`] on failure.
    pub async fn test_connection(&self) -> Result<()> {
        // ------------------------------------------------------------------
        // Windows: not supported
        // ------------------------------------------------------------------
        #[cfg(target_os = "windows")]
        {
            return Err(SyncError::Connection(
                "SSH sync is not supported on Windows.".to_string(),
            ));
        }

        // ------------------------------------------------------------------
        // Unix: ssh … exit
        // ------------------------------------------------------------------
        #[cfg(unix)]
        {
            let mut args: Vec<String> = self.common_ssh_opts_for_ssh_binary();
            args.push(self.config.destination());
            args.push("exit".to_string());

            debug!(
                "ssh_sync: testing connection to {}",
                self.config.destination()
            );

            let output = tokio::process::Command::new("ssh")
                .args(&args)
                .output()
                .await
                .map_err(|e| SyncError::Connection(format!("Failed to spawn ssh: {e}")))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                warn!("ssh_sync: connection test failed — {stderr}");
                return Err(SyncError::Connection(format!(
                    "SSH connection failed (exit {:?}): {stderr}",
                    output.status.code()
                )));
            }

            info!(
                "ssh_sync: connection to {} OK",
                self.config.destination()
            );

            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sync(port: u16, identity: Option<&str>) -> SshSync {
        SshSync::new(
            SshConfig::new(
                "192.168.1.42",
                port,
                "alice",
                identity.map(str::to_string),
            )
            .expect("valid test config"),
        )
    }

    // -----------------------------------------------------------------------
    // test_ssh_config_new — verify struct creation and field values
    // -----------------------------------------------------------------------
    #[test]
    fn test_ssh_config_new() {
        let cfg = SshConfig::new(
            "10.0.0.1",
            2222,
            "bob",
            Some("/home/bob/.ssh/id_rsa".to_string()),
        )
        .expect("valid config");

        assert_eq!(cfg.host, "10.0.0.1");
        assert_eq!(cfg.port, 2222);
        assert_eq!(cfg.username, "bob");
        assert_eq!(cfg.identity_path, Some("/home/bob/.ssh/id_rsa".to_string()));
        assert!(!cfg.strict_host_checking, "default should be false");

        // destination() should return "user@host"
        assert_eq!(cfg.destination(), "bob@10.0.0.1");

        // SshSync wraps the config
        let sync = SshSync::new(cfg.clone());
        assert_eq!(sync.config.host, cfg.host);
    }

    // -----------------------------------------------------------------------
    // test_strict_host_checking_field — field survives round-trip
    // -----------------------------------------------------------------------
    #[test]
    fn test_strict_host_checking_field() {
        let mut cfg = SshConfig::new("10.0.0.1", 22, "bob", None)
            .expect("valid config");

        // default is false
        assert!(!cfg.strict_host_checking);

        cfg.strict_host_checking = true;
        let sync = SshSync::new(cfg);
        let args = sync.common_ssh_opts();

        let shk_pos = args
            .iter()
            .position(|a| a == "StrictHostKeyChecking=yes")
            .expect("StrictHostKeyChecking=yes should appear in args");
        // The preceding element must be "-o"
        assert_eq!(args[shk_pos - 1], "-o");

        // Also test the ssh-binary variant
        let args2 = sync.common_ssh_opts_for_ssh_binary();
        assert!(
            args2.iter().any(|a| a == "StrictHostKeyChecking=yes"),
            "ssh-binary opts should also use StrictHostKeyChecking=yes"
        );
    }

    // -----------------------------------------------------------------------
    // test_strict_host_checking_default_no — default produces =no
    // -----------------------------------------------------------------------
    #[test]
    fn test_strict_host_checking_default_no() {
        let sync = make_sync(22, None);
        let args = sync.common_ssh_opts();
        assert!(
            args.iter().any(|a| a == "StrictHostKeyChecking=no"),
            "Default should produce StrictHostKeyChecking=no, got: {args:?}"
        );
    }

    // -----------------------------------------------------------------------
    // test_push_nonexistent_file_returns_error — source file missing → Err
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn test_push_nonexistent_file_returns_error() {
        let sync = make_sync(22, None);

        let result = sync
            .push_credentials(
                "/tmp/this_file_definitely_does_not_exist_xyz123.json",
                "~/creds.json",
            )
            .await;

        assert!(
            result.is_err(),
            "Expected error for non-existent source file"
        );

        let err_msg = result.unwrap_err().to_string();
        // On Unix: "Source file does not exist: …"
        // On Windows: "SSH sync is not supported on Windows."
        assert!(
            err_msg.contains("does not exist") || err_msg.contains("not supported"),
            "Unexpected error message: {err_msg}"
        );
    }

    // -----------------------------------------------------------------------
    // test_ssh_args_include_port — SCP args must contain the port flag
    // -----------------------------------------------------------------------
    #[test]
    fn test_ssh_args_include_port() {
        let sync = make_sync(2222, None);
        let args = sync.common_ssh_opts();

        // The args slice should contain "-P" followed by "2222"
        let port_flag_pos = args.iter().position(|a| a == "-P");
        assert!(
            port_flag_pos.is_some(),
            "Expected '-P' flag in scp args, got: {args:?}"
        );
        let pos = port_flag_pos.unwrap();
        assert_eq!(
            args[pos + 1], "2222",
            "Expected port value '2222' after '-P', got: {}",
            args[pos + 1]
        );
    }

    // -----------------------------------------------------------------------
    // test_ssh_binary_args_include_lowercase_p — ssh uses -p (not -P)
    // -----------------------------------------------------------------------
    #[test]
    fn test_ssh_binary_args_include_lowercase_p() {
        let sync = make_sync(2222, None);
        let args = sync.common_ssh_opts_for_ssh_binary();

        let port_flag_pos = args.iter().position(|a| a == "-p");
        assert!(
            port_flag_pos.is_some(),
            "Expected '-p' flag in ssh args, got: {args:?}"
        );
        let pos = port_flag_pos.unwrap();
        assert_eq!(
            args[pos + 1], "2222",
            "Expected port value '2222' after '-p', got: {}",
            args[pos + 1]
        );
        // Must NOT contain the uppercase variant
        assert!(
            !args.iter().any(|a| a == "-P"),
            "ssh args must not contain '-P', got: {args:?}"
        );
    }

    // -----------------------------------------------------------------------
    // test_ssh_args_include_identity — -i flag present when identity_path set
    // -----------------------------------------------------------------------
    #[test]
    fn test_ssh_args_include_identity() {
        let sync = make_sync(22, Some("/home/alice/.ssh/id_ed25519"));
        let args = sync.common_ssh_opts();

        let identity_flag_pos = args.iter().position(|a| a == "-i");
        assert!(
            identity_flag_pos.is_some(),
            "Expected '-i' flag when identity_path is set, got: {args:?}"
        );
        let pos = identity_flag_pos.unwrap();
        assert_eq!(args[pos + 1], "/home/alice/.ssh/id_ed25519");
    }

    // -----------------------------------------------------------------------
    // test_ssh_args_no_identity — -i flag absent when identity_path is None
    // -----------------------------------------------------------------------
    #[test]
    fn test_ssh_args_no_identity() {
        let sync = make_sync(22, None);
        let args = sync.common_ssh_opts();

        assert!(
            !args.iter().any(|a| a == "-i"),
            "Expected no '-i' flag when identity_path is None, got: {args:?}"
        );
    }

    // -----------------------------------------------------------------------
    // test_destination_format — user@host string
    // -----------------------------------------------------------------------
    #[test]
    fn test_destination_format() {
        let sync = make_sync(22, None);
        assert_eq!(sync.config.destination(), "alice@192.168.1.42");
    }

    // -----------------------------------------------------------------------
    // test_default_port — port 22 is preserved as-is
    // -----------------------------------------------------------------------
    #[test]
    fn test_default_port() {
        let sync = make_sync(22, None);
        assert_eq!(sync.config.port, 22);
        let args = sync.common_ssh_opts();
        let pos = args.iter().position(|a| a == "-P").unwrap();
        assert_eq!(args[pos + 1], "22");
    }

    // -----------------------------------------------------------------------
    // test_invalid_host — host with whitespace or leading dash is rejected
    // -----------------------------------------------------------------------
    #[test]
    fn test_invalid_host() {
        let err1 = SshConfig::new("victim.com -o ProxyCommand=evil", 22, "alice", None)
            .unwrap_err()
            .to_string();
        assert!(
            err1.contains("Invalid SSH host"),
            "Expected 'Invalid SSH host' in: {err1}"
        );

        let err2 = SshConfig::new("-oProxyCommand=evil", 22, "alice", None)
            .unwrap_err()
            .to_string();
        assert!(
            err2.contains("Invalid SSH host"),
            "Expected 'Invalid SSH host' in: {err2}"
        );
    }

    // -----------------------------------------------------------------------
    // test_invalid_username — username with whitespace, @, or leading dash rejected
    // -----------------------------------------------------------------------
    #[test]
    fn test_invalid_username() {
        let err1 = SshConfig::new("10.0.0.1", 22, "alice bob", None)
            .unwrap_err()
            .to_string();
        assert!(
            err1.contains("Invalid SSH username"),
            "Expected 'Invalid SSH username' in: {err1}"
        );

        let err2 = SshConfig::new("10.0.0.1", 22, "alice@evil.com", None)
            .unwrap_err()
            .to_string();
        assert!(
            err2.contains("Invalid SSH username"),
            "Expected 'Invalid SSH username' in: {err2}"
        );

        let err3 = SshConfig::new("10.0.0.1", 22, "-oProxyCommand=evil", None)
            .unwrap_err()
            .to_string();
        assert!(
            err3.contains("Invalid SSH username"),
            "Expected 'Invalid SSH username' in: {err3}"
        );
    }
}
