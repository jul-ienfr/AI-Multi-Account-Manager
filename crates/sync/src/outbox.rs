//! Outbox persistante — file d'attente de messages non encore livrés.
//!
//! Traduit la logique de l'outbox Python de `sync_bus.py` en Rust.
//!
//! # Format
//!
//! Le fichier est un JSONL (une entrée JSON par ligne) situé à :
//! `~/.claude/multi-account/sync-outbox.jsonl`
//!
//! # Opérations
//!
//! - `push`  : ajoute un message (append au fichier)
//! - `drain` : lit toutes les entrées non expirées
//! - `ack`   : acquitte (supprime) une entrée par ID
//! - `prune` : supprime les entrées expirées et les excédentaires (TTL + max)
//!
//! # Invariants
//!
//! - Maximum 500 entrées conservées.
//! - TTL de 1 heure : les entrées plus vieilles sont supprimées par `prune`.
//! - Le fichier est réécrit intégralement lors des opérations `ack` et `prune`
//!   (volume attendu faible — quelques Ko au maximum).

use std::cell::Cell;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{Result, SyncError};

// ────────────────────────────────────────────────────────────────────────────
// Constantes
// ────────────────────────────────────────────────────────────────────────────

/// Nombre maximum d'entrées dans l'outbox.
const MAX_ENTRIES: usize = 500;

/// Durée de vie d'une entrée (1 heure).
const TTL: Duration = Duration::from_secs(3600);

/// Nom du fichier JSONL de l'outbox.
const OUTBOX_FILENAME: &str = "sync-outbox.jsonl";

// ────────────────────────────────────────────────────────────────────────────
// OutboxEntry
// ────────────────────────────────────────────────────────────────────────────

/// Message en attente de livraison.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboxEntry {
    /// Identifiant unique du message (UUID v4).
    pub id: String,
    /// Payload sérialisé (opaque pour l'outbox).
    pub payload: Vec<u8>,
    /// Horodatage de création (UTC).
    pub created_at: DateTime<Utc>,
    /// Nombre de tentatives de livraison effectuées.
    pub attempts: u32,
}

// ────────────────────────────────────────────────────────────────────────────
// Outbox
// ────────────────────────────────────────────────────────────────────────────

/// File d'attente persistante JSONL pour les messages sync non livrés.
pub struct Outbox {
    /// Chemin vers le fichier JSONL.
    path: PathBuf,
    /// Nombre maximum d'entrées conservées.
    max_entries: usize,
    /// Durée de vie d'une entrée.
    ttl: Duration,
    /// Cache du nombre d'entrées non expirées.
    ///
    /// `Cell` permet la mutation intérieure depuis `&self` (pour `len` /
    /// `is_empty` qui ne peuvent pas prendre `&mut self` sans casser l'API).
    /// Invalidé (→ `None`) après toute opération qui modifie le fichier.
    cached_count: Cell<Option<usize>>,
}

impl Outbox {
    /// Crée une nouvelle instance pointant vers `{base_dir}/sync-outbox.jsonl`.
    ///
    /// Le répertoire `base_dir` n'a pas besoin d'exister — il sera créé lors
    /// du premier `push`.
    pub fn new(base_dir: &Path) -> Self {
        Self {
            path: base_dir.join(OUTBOX_FILENAME),
            max_entries: MAX_ENTRIES,
            ttl: TTL,
            cached_count: Cell::new(None),
        }
    }

    /// Crée une instance avec des paramètres personnalisés (utile pour les tests).
    pub fn with_params(base_dir: &Path, max_entries: usize, ttl: Duration) -> Self {
        Self {
            path: base_dir.join(OUTBOX_FILENAME),
            max_entries,
            ttl,
            cached_count: Cell::new(None),
        }
    }

    /// Ajoute un message à l'outbox.
    ///
    /// Retourne l'ID généré pour ce message.
    /// Le message est ajouté en mode append au fichier JSONL.
    pub fn push(&mut self, payload: Vec<u8>) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let entry = OutboxEntry {
            id: id.clone(),
            payload,
            created_at: Utc::now(),
            attempts: 0,
        };
        self.push_entry(entry)?;
        self.cached_count.set(None); // invalide le cache
        Ok(id)
    }

    /// Ajoute une entrée déjà construite (utile en interne et pour les tests).
    fn push_entry(&self, entry: OutboxEntry) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(SyncError::Io)?;
        }
        let line = serde_json::to_string(&entry)?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(SyncError::Io)?;
        writeln!(file, "{}", line).map_err(SyncError::Io)?;
        Ok(())
    }

    /// Retourne toutes les entrées non expirées de l'outbox.
    ///
    /// Les entrées corrompues (JSON invalide) ou expirées sont ignorées
    /// silencieusement (elles seront supprimées lors du prochain `prune`).
    /// Met à jour le cache de compteur au passage.
    pub fn drain(&self) -> Vec<OutboxEntry> {
        let entries: Vec<OutboxEntry> = self
            .read_valid_entries()
            .into_iter()
            .filter(|e| !self.is_expired(e))
            .collect();
        // Peuple le cache — une lecture complète vient d'avoir lieu, autant en
        // profiter pour éviter un deuxième passage si `len` / `is_empty` sont
        // appelés juste après.
        self.cached_count.set(Some(entries.len()));
        entries
    }

    /// Acquitte un message : le supprime du fichier JSONL.
    ///
    /// Le fichier est réécrit intégralement sans l'entrée ACK.
    pub fn ack(&mut self, id: &str) {
        let entries: Vec<OutboxEntry> = self
            .read_valid_entries()
            .into_iter()
            .filter(|e| e.id != id)
            .collect();
        let _ = self.rewrite(&entries);
        self.cached_count.set(None); // invalide le cache
    }

    /// Supprime les entrées expirées et réduit l'outbox à `max_entries`.
    ///
    /// Les entrées les plus anciennes sont supprimées en premier si le nombre
    /// dépasse la limite.
    pub fn prune(&mut self) {
        let mut entries: Vec<OutboxEntry> = self
            .read_valid_entries()
            .into_iter()
            .filter(|e| !self.is_expired(e))
            .collect();

        // Tronque aux max_entries les plus récentes (garde la queue de la liste)
        if entries.len() > self.max_entries {
            let excess = entries.len() - self.max_entries;
            entries.drain(..excess);
        }

        let _ = self.rewrite(&entries);
        self.cached_count.set(None); // invalide le cache
    }

    /// Retourne le nombre d'entrées non expirées dans l'outbox.
    ///
    /// Si le cache est valide, la réponse est instantanée (pas d'I/O).
    /// Sinon, effectue un `drain()` complet qui peuplera le cache pour les
    /// appels suivants.
    pub fn len(&self) -> usize {
        if let Some(count) = self.cached_count.get() {
            return count;
        }
        // Pas de cache valide : drain() lit le fichier et met à jour le cache.
        self.drain().len()
    }

    /// Retourne `true` si l'outbox est vide (ou n'existe pas).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Chemin du fichier JSONL.
    pub fn path(&self) -> &Path {
        &self.path
    }

    // ── Helpers privés ────────────────────────────────────────────────────

    /// Lit toutes les entrées valides du fichier (ignore les lignes corrompues).
    fn read_valid_entries(&self) -> Vec<OutboxEntry> {
        let file = match std::fs::File::open(&self.path) {
            Ok(f) => f,
            Err(_) => return Vec::new(), // fichier absent → outbox vide
        };
        BufReader::new(file)
            .lines()
            .filter_map(|line| {
                let line = line.ok()?;
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    return None;
                }
                serde_json::from_str(trimmed).ok()
            })
            .collect()
    }

    /// Vérifie si une entrée a dépassé son TTL.
    fn is_expired(&self, entry: &OutboxEntry) -> bool {
        let age = Utc::now()
            .signed_duration_since(entry.created_at)
            .to_std()
            .unwrap_or(Duration::ZERO);
        age > self.ttl
    }

    /// Réécrit le fichier JSONL avec les entrées fournies.
    fn rewrite(&self, entries: &[OutboxEntry]) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(SyncError::Io)?;
        }

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&self.path)
            .map_err(SyncError::Io)?;

        for entry in entries {
            let line = serde_json::to_string(entry)?;
            writeln!(file, "{}", line).map_err(SyncError::Io)?;
        }
        Ok(())
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tmp_outbox() -> (TempDir, Outbox) {
        let dir = TempDir::new().unwrap();
        let outbox = Outbox::new(dir.path());
        (dir, outbox)
    }

    // ── test_outbox_push_drain_ack ────────────────────────────────────────

    #[test]
    fn test_outbox_push_drain_ack() {
        let (_dir, mut outbox) = tmp_outbox();

        // Push deux messages
        let id1 = outbox.push(b"hello".to_vec()).unwrap();
        let id2 = outbox.push(b"world".to_vec()).unwrap();

        // drain retourne les deux
        let entries = outbox.drain();
        assert_eq!(entries.len(), 2);
        let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
        assert!(ids.contains(&id1.as_str()));
        assert!(ids.contains(&id2.as_str()));

        // Vérifie les payloads
        let e1 = entries.iter().find(|e| e.id == id1).unwrap();
        assert_eq!(e1.payload, b"hello");
        assert_eq!(e1.attempts, 0);

        // Ack le premier
        outbox.ack(&id1);

        // drain ne retourne plus que le second
        let entries = outbox.drain();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, id2);

        // Ack le second
        outbox.ack(&id2);
        assert!(outbox.is_empty());
    }

    #[test]
    fn test_outbox_drain_empty() {
        let (_dir, outbox) = tmp_outbox();
        assert!(outbox.drain().is_empty());
        assert!(outbox.is_empty());
    }

    #[test]
    fn test_outbox_push_generates_unique_ids() {
        let (_dir, mut outbox) = tmp_outbox();
        let id1 = outbox.push(b"a".to_vec()).unwrap();
        let id2 = outbox.push(b"b".to_vec()).unwrap();
        assert_ne!(id1, id2);
    }

    // ── test_outbox_prune_ttl ─────────────────────────────────────────────

    /// Teste que prune supprime les entrées expirées.
    ///
    /// On injecte directement une entrée avec un `created_at` dans le passé
    /// pour éviter d'attendre le TTL dans le test.
    #[test]
    fn test_outbox_prune_ttl() {
        let dir = TempDir::new().unwrap();
        let mut outbox = Outbox::with_params(dir.path(), 500, Duration::from_secs(60));

        // Injecte une entrée avec created_at dans le passé (2 heures ago)
        let old_entry = OutboxEntry {
            id: "old-id".to_string(),
            payload: b"old".to_vec(),
            created_at: Utc::now() - chrono::Duration::hours(2),
            attempts: 0,
        };
        outbox.push_entry(old_entry).unwrap();

        // Injecte une entrée récente
        let fresh_id = outbox.push(b"fresh".to_vec()).unwrap();

        // Avant prune : drain ne retourne que "fresh" (old est expiré, TTL=60s)
        let before = outbox.drain();
        assert_eq!(before.len(), 1, "only fresh entry should be visible via drain");
        assert_eq!(before[0].id, fresh_id);

        // Prune réécrit le fichier sans l'entrée expirée
        outbox.prune();

        // Après prune : uniquement "fresh"
        let after = outbox.drain();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].id, fresh_id);
        assert!(
            after.iter().all(|e| e.payload != b"old"),
            "expired entry should have been removed by prune"
        );
    }

    /// Teste que prune limite l'outbox à max_entries (FIFO).
    #[test]
    fn test_outbox_prune_max_entries() {
        let dir = TempDir::new().unwrap();
        // Outbox max 3 entrées, TTL long
        let mut outbox = Outbox::with_params(dir.path(), 3, Duration::from_secs(3600));

        let mut ids = Vec::new();
        for i in 0..5 {
            ids.push(outbox.push(format!("payload-{}", i).into_bytes()).unwrap());
        }

        outbox.prune();
        let entries = outbox.drain();
        assert_eq!(
            entries.len(),
            3,
            "prune should cap at max_entries (3), got {}",
            entries.len()
        );

        // Les 3 dernières entrées (payload-2, payload-3, payload-4) sont conservées
        let payloads: Vec<String> = entries
            .iter()
            .map(|e| String::from_utf8(e.payload.clone()).unwrap())
            .collect();
        assert!(payloads.contains(&"payload-2".to_string()));
        assert!(payloads.contains(&"payload-3".to_string()));
        assert!(payloads.contains(&"payload-4".to_string()));
        // Les 2 premières ont été évincées
        assert!(!payloads.contains(&"payload-0".to_string()));
        assert!(!payloads.contains(&"payload-1".to_string()));
    }

    #[test]
    fn test_outbox_ack_nonexistent_is_noop() {
        let (_dir, mut outbox) = tmp_outbox();
        outbox.push(b"msg".to_vec()).unwrap();
        // Ack d'un ID inexistant → ne plante pas, ne supprime rien
        outbox.ack("nonexistent-id");
        assert_eq!(outbox.len(), 1);
    }

    #[test]
    fn test_outbox_len() {
        let (_dir, mut outbox) = tmp_outbox();
        assert_eq!(outbox.len(), 0);
        outbox.push(b"a".to_vec()).unwrap();
        assert_eq!(outbox.len(), 1);
        outbox.push(b"b".to_vec()).unwrap();
        assert_eq!(outbox.len(), 2);
    }

    #[test]
    fn test_outbox_file_is_jsonl() {
        let (_dir, mut outbox) = tmp_outbox();
        outbox.push(b"test".to_vec()).unwrap();

        let content = std::fs::read_to_string(outbox.path()).unwrap();
        // Chaque ligne doit être un JSON valide
        for line in content.lines() {
            let v: serde_json::Value = serde_json::from_str(line).expect("valid JSON line");
            assert!(v.get("id").is_some());
            assert!(v.get("payload").is_some());
            assert!(v.get("created_at").is_some());
            assert!(v.get("attempts").is_some());
        }
    }

    #[test]
    fn test_outbox_created_at_is_recent() {
        let (_dir, mut outbox) = tmp_outbox();
        let before = Utc::now();
        outbox.push(b"ts-check".to_vec()).unwrap();
        let after = Utc::now();

        let entries = outbox.drain();
        assert_eq!(entries.len(), 1);
        let ts = entries[0].created_at;
        assert!(ts >= before, "created_at should be >= before push");
        assert!(ts <= after, "created_at should be <= after push");
    }

    /// Vérifie que len() / is_empty() utilisent le cache après un drain().
    /// (test comportemental : on vérifie la cohérence, pas les internals)
    #[test]
    fn test_outbox_cached_count_coherence() {
        let (_dir, mut outbox) = tmp_outbox();

        // Outbox vide : cache doit retourner 0
        assert_eq!(outbox.len(), 0);
        assert!(outbox.is_empty());

        outbox.push(b"x".to_vec()).unwrap();
        // Après push, cache invalidé → len() relit le fichier
        assert_eq!(outbox.len(), 1);
        // Second appel immédiat → cache valide, pas d'I/O supplémentaire
        assert_eq!(outbox.len(), 1);
        assert!(!outbox.is_empty());

        // drain() peuple aussi le cache
        let _ = outbox.drain();
        assert_eq!(outbox.len(), 1);

        // ack invalide le cache
        let id = outbox.push(b"y".to_vec()).unwrap();
        outbox.ack(&id);
        assert_eq!(outbox.len(), 1); // "x" reste

        // prune invalide le cache
        outbox.prune();
        assert_eq!(outbox.len(), 1);
    }
}
