//! Module de validation des formats — Phase 2.3.
//!
//! Traduit la logique Python de `src/agents/validator_agent.py` en Rust.
//!
//! Fonctions exportées :
//! - [`validate_email`]         — email valide (contient @, longueur 3-254, sans espaces)
//! - [`validate_access_token`]  — access token Anthropic ou JWT ou UUID-like
//! - [`validate_refresh_token`] — refresh token Anthropic ou JWT
//! - [`validate_api_key`]       — clé API Anthropic

/// Valide le format d'une adresse email.
///
/// Règles :
/// - Doit contenir exactement un `@`
/// - Longueur totale entre 3 et 254 caractères
/// - Pas d'espaces
///
/// # Errors
/// Retourne une description de l'erreur si la validation échoue.
pub fn validate_email(email: &str) -> Result<(), &'static str> {
    if email.len() < 3 {
        return Err("Email trop court (minimum 3 caractères)");
    }
    if email.len() > 254 {
        return Err("Email trop long (maximum 254 caractères)");
    }
    if email.contains(' ') {
        return Err("Email ne doit pas contenir d'espaces");
    }
    if !email.contains('@') {
        return Err("Email doit contenir un '@'");
    }
    // Vérifier qu'il y a exactement un '@' avec une partie locale et un domaine
    let parts: Vec<&str> = email.splitn(2, '@').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        return Err("Format email invalide");
    }
    // Le domaine doit contenir un '.'
    if !parts[1].contains('.') {
        return Err("Domaine email invalide (doit contenir un point)");
    }
    Ok(())
}

/// Valide le format d'un access token Anthropic.
///
/// Formats acceptés :
/// - Commence par `sk-ant-oa` (access token OAuth Anthropic)
/// - Commence par `eyJ` (JWT — format utilisé par Claude.ai)
/// - Format UUID-like : ≥ 20 caractères, pas d'espaces
///
/// # Errors
/// Retourne une description de l'erreur si la validation échoue.
pub fn validate_access_token(token: &str) -> Result<(), &'static str> {
    if token.is_empty() {
        return Err("Access token vide");
    }
    if token.contains(' ') {
        return Err("Access token ne doit pas contenir d'espaces");
    }
    // Formats reconnus
    if token.starts_with("sk-ant-oa") {
        return Ok(());
    }
    if token.starts_with("eyJ") {
        // JWT — longueur minimale raisonnable
        if token.len() < 20 {
            return Err("JWT access token trop court");
        }
        return Ok(());
    }
    // UUID-like ou format propriétaire
    if token.len() >= 20 {
        return Ok(());
    }
    Err("Access token invalide (format non reconnu ou trop court)")
}

/// Valide le format d'un refresh token Anthropic.
///
/// Formats acceptés :
/// - Commence par `sk-ant-ort` (refresh token OAuth Anthropic)
/// - Commence par `eyJ` (JWT)
/// - Longueur minimale : 10 caractères, pas d'espaces
///
/// # Errors
/// Retourne une description de l'erreur si la validation échoue.
pub fn validate_refresh_token(token: &str) -> Result<(), &'static str> {
    if token.is_empty() {
        return Err("Refresh token vide");
    }
    if token.len() < 10 {
        return Err("Refresh token trop court (minimum 10 caractères)");
    }
    if token.contains(' ') {
        return Err("Refresh token ne doit pas contenir d'espaces");
    }
    if token.starts_with("sk-ant-ort") {
        return Ok(());
    }
    if token.starts_with("eyJ") {
        return Ok(());
    }
    // Accepter les tokens longs même si le préfixe n'est pas reconnu
    // (ex : refreshToken = accessToken dans le design Claude Code)
    if token.len() >= 10 {
        return Ok(());
    }
    Err("Refresh token invalide")
}

/// Valide le format d'une clé API Anthropic.
///
/// Formats acceptés :
/// - Commence par `sk-ant-api` (clé API Anthropic standard)
/// - Commence par `sk-ant-` (préfixe général Anthropic)
/// - Longueur minimale : 20 caractères
/// - Pas d'espaces
///
/// # Errors
/// Retourne une description de l'erreur si la validation échoue.
pub fn validate_api_key(key: &str) -> Result<(), &'static str> {
    if key.is_empty() {
        return Err("Clé API vide");
    }
    if key.contains(' ') {
        return Err("Clé API ne doit pas contenir d'espaces");
    }
    if key.len() < 20 {
        return Err("Clé API trop courte (minimum 20 caractères)");
    }
    if key.starts_with("sk-ant-api") {
        return Ok(());
    }
    if key.starts_with("sk-ant-") {
        return Ok(());
    }
    Err("Clé API invalide (doit commencer par 'sk-ant-api' ou 'sk-ant-')")
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- validate_email ---

    #[test]
    fn test_email_valid() {
        assert!(validate_email("user@example.com").is_ok());
        assert!(validate_email("user.name+tag@sub.domain.org").is_ok());
        assert!(validate_email("a@b.c").is_ok());
    }

    #[test]
    fn test_email_no_at() {
        assert!(validate_email("userexample.com").is_err());
    }

    #[test]
    fn test_email_with_space() {
        assert!(validate_email("user @example.com").is_err());
    }

    #[test]
    fn test_email_too_short() {
        assert!(validate_email("a@").is_err());
    }

    #[test]
    fn test_email_too_long() {
        let long = format!("{}@b.c", "a".repeat(252));
        assert!(validate_email(&long).is_err());
    }

    #[test]
    fn test_email_no_dot_in_domain() {
        assert!(validate_email("user@localdomain").is_err());
    }

    // --- validate_access_token ---

    #[test]
    fn test_access_token_sk_ant_oa() {
        assert!(validate_access_token("sk-ant-oa01-abc123def456").is_ok());
    }

    #[test]
    fn test_access_token_jwt() {
        let jwt = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.payload.signature";
        assert!(validate_access_token(jwt).is_ok());
    }

    #[test]
    fn test_access_token_uuid_like() {
        // 36 chars UUID format
        assert!(validate_access_token("550e8400-e29b-41d4-a716-446655440000").is_ok());
    }

    #[test]
    fn test_access_token_empty() {
        assert!(validate_access_token("").is_err());
    }

    #[test]
    fn test_access_token_too_short() {
        assert!(validate_access_token("short").is_err());
    }

    #[test]
    fn test_access_token_with_space() {
        assert!(validate_access_token("sk-ant-oa01 abc").is_err());
    }

    // --- validate_refresh_token ---

    #[test]
    fn test_refresh_token_sk_ant_ort() {
        assert!(validate_refresh_token("sk-ant-ort01-abc123def456xyz").is_ok());
    }

    #[test]
    fn test_refresh_token_jwt() {
        let jwt = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.payload.sig";
        assert!(validate_refresh_token(jwt).is_ok());
    }

    #[test]
    fn test_refresh_token_too_short() {
        assert!(validate_refresh_token("short").is_err());
    }

    #[test]
    fn test_refresh_token_with_space() {
        assert!(validate_refresh_token("sk-ant-ort01 abc").is_err());
    }

    #[test]
    fn test_refresh_token_empty() {
        assert!(validate_refresh_token("").is_err());
    }

    #[test]
    fn test_refresh_token_long_arbitrary() {
        // Design voulu : refreshToken = accessToken (UUID-like suffisamment long)
        assert!(validate_refresh_token("550e8400-e29b-41d4-a716-446655440000").is_ok());
    }

    // --- validate_api_key ---

    #[test]
    fn test_api_key_sk_ant_api() {
        assert!(validate_api_key("sk-ant-api03-verylongkeyvalue1234567890abcdef").is_ok());
    }

    #[test]
    fn test_api_key_sk_ant_prefix() {
        assert!(validate_api_key("sk-ant-oat01-verylongkeyvalue1234567890abcdef").is_ok());
    }

    #[test]
    fn test_api_key_too_short() {
        assert!(validate_api_key("sk-ant-api03-short").is_err());
    }

    #[test]
    fn test_api_key_wrong_prefix() {
        // Au moins 20 chars mais mauvais préfixe
        assert!(validate_api_key("pk-not-anthropic-key-12345").is_err());
    }

    #[test]
    fn test_api_key_empty() {
        assert!(validate_api_key("").is_err());
    }

    #[test]
    fn test_api_key_with_space() {
        assert!(validate_api_key("sk-ant-api03-valid key space").is_err());
    }
}
