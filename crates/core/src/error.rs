//! Types d'erreurs centraux du crate `core`.

use thiserror::Error;

/// Erreur principale du crate core.
#[derive(Debug, Error)]
pub enum CoreError {
    /// Erreur d'entrée/sortie (filesystem, réseau bas niveau).
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Erreur de sérialisation/désérialisation JSON.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Erreur d'authentification (token invalide, révoqué, etc.).
    #[error("Authentication error: {0}")]
    Auth(String),

    /// Ressource non trouvée (compte, fichier, etc.).
    #[error("Not found: {0}")]
    NotFound(String),

    /// Erreur de configuration (valeur invalide, fichier manquant, etc.).
    #[error("Configuration error: {0}")]
    Config(String),

    /// Erreur de quota (dépassement, limite atteinte, etc.).
    #[error("Quota error: {0}")]
    Quota(String),

    /// Erreur HTTP lors d'un appel API.
    #[error("HTTP error: {0}")]
    Http(String),

    /// Erreur de concurrence (lock poison, etc.).
    #[error("Concurrency error: {0}")]
    Concurrency(String),
}

/// Type alias pour `Result<T, CoreError>`.
pub type Result<T> = std::result::Result<T, CoreError>;

impl From<reqwest::Error> for CoreError {
    fn from(e: reqwest::Error) -> Self {
        CoreError::Http(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display_io() {
        let e = CoreError::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "file missing"));
        assert!(e.to_string().contains("IO error"));
    }

    #[test]
    fn test_error_display_auth() {
        let e = CoreError::Auth("token revoked".to_string());
        assert_eq!(e.to_string(), "Authentication error: token revoked");
    }

    #[test]
    fn test_error_display_not_found() {
        let e = CoreError::NotFound("account_key".to_string());
        assert!(e.to_string().contains("Not found"));
    }

    #[test]
    fn test_error_display_config() {
        let e = CoreError::Config("missing field".to_string());
        assert!(e.to_string().contains("Configuration error"));
    }

    #[test]
    fn test_error_display_quota() {
        let e = CoreError::Quota("5h limit exceeded".to_string());
        assert!(e.to_string().contains("Quota error"));
    }

    #[test]
    fn test_from_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let core_err: CoreError = io_err.into();
        assert!(matches!(core_err, CoreError::Io(_)));
    }

    #[test]
    fn test_from_json_error() {
        let json_err = serde_json::from_str::<serde_json::Value>("invalid json !!!").unwrap_err();
        let core_err: CoreError = json_err.into();
        assert!(matches!(core_err, CoreError::Json(_)));
    }
}
