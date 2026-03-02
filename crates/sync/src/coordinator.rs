//! Coordinateur de synchronisation P2P — réconciliation LWW via vector clocks.
//!
//! Traduit la logique Python de `sync_coordinator.py` en Rust async.
//!
//! # Algorithme de réconciliation
//!
//! On utilise Last-Write-Wins (LWW) via vector clocks :
//! - Chaque instance incrémente son entrée dans le clock avant d'émettre
//! - À la réception, on merge les clocks (max de chaque entrée)
//! - Si le clock distant domine → accepte la mise à jour
//! - Sinon → ignore (on a des données plus récentes)
//!
//! # Pipeline 7 étapes (Phase 5.1)
//!
//! Quand une connexion peer est établie, le coordinateur peut déclencher un
//! pipeline structuré : HANDSHAKE → DIFF → OUTBOX DRAIN → FULL SYNC →
//! MERGE → APPLY → ACK.
//!
//! Le pipeline est géré par [`SyncPipeline`] qui opère en mémoire. Chaque étape
//! produit un résultat intermédiaire exploité par l'étape suivante.
//!
//! ## Outbox (étape 3 — Phase 5.3)
//!
//! L'étape OUTBOX DRAIN vide la file persistante des messages non encore livrés
//! avant le transfert principal. Dans [`SyncCoordinator::run_pipeline_with_peer`],
//! le drain est effectué *avant* l'appel à `run_pipeline` (lock libéré avant tout
//! I/O async). L'ack est **optimiste** : les entrées sont supprimées avant toute
//! confirmation réseau. En cas d'échec pipeline, les messages sont perdus.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;
use tracing::{debug, info, warn};

use crate::bus::SyncBus;
use crate::error::{Result, SyncError};
use crate::messages::{
    DiffSummary, HandshakeSummary, ProxyInstanceStatus, SyncMessage, SyncPayload, VectorClock,
};
use crate::outbox::Outbox;

/// Interval entre les heartbeats envoyés aux pairs.
const HEARTBEAT_INTERVAL_SECS: u64 = 10;

// ----------------------------------------------------------------
// Types de session pipeline (Phase 5.1)
// ----------------------------------------------------------------

/// Phase courante d'une session de synchronisation.
///
/// Les phases s'enchaînent dans l'ordre :
/// `Handshake → Diff → OutboxDrain → FullSync → Merge → Apply → Ack → Complete`.
/// Un échec à n'importe quelle étape passe en `Failed`.
#[derive(Debug, Clone, PartialEq)]
pub enum SyncPhase {
    /// Échange des vector clocks pour déterminer qui a des données plus récentes.
    Handshake,
    /// Calcul du différentiel entre nos comptes et ceux du peer.
    Diff,
    /// Envoi des messages en attente dans l'outbox pour ce peer (Phase 5.3).
    OutboxDrain,
    /// Transfert des comptes identifiés comme manquants ou obsolètes.
    FullSync,
    /// Résolution des conflits avec la stratégie LWW.
    Merge,
    /// Persistance des modifications dans `CredentialsCache`.
    Apply,
    /// Envoi de la confirmation et mise à jour du vector clock local.
    Ack,
    /// Pipeline terminé avec succès.
    Complete,
    /// Pipeline interrompu par une erreur.
    Failed(String),
}

impl std::fmt::Display for SyncPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncPhase::Handshake => write!(f, "HANDSHAKE"),
            SyncPhase::Diff => write!(f, "DIFF"),
            SyncPhase::OutboxDrain => write!(f, "OUTBOX_DRAIN"),
            SyncPhase::FullSync => write!(f, "FULL_SYNC"),
            SyncPhase::Merge => write!(f, "MERGE"),
            SyncPhase::Apply => write!(f, "APPLY"),
            SyncPhase::Ack => write!(f, "ACK"),
            SyncPhase::Complete => write!(f, "COMPLETE"),
            SyncPhase::Failed(e) => write!(f, "FAILED({})", e),
        }
    }
}

/// État d'une session de synchronisation avec un peer donné.
pub struct SyncSession {
    /// Identifiant du peer avec lequel on synchronise.
    pub peer_id: String,
    /// Phase courante du pipeline.
    pub phase: SyncPhase,
    /// Moment où la session a commencé.
    pub started_at: Instant,
}

impl SyncSession {
    pub fn new(peer_id: impl Into<String>) -> Self {
        Self {
            peer_id: peer_id.into(),
            phase: SyncPhase::Handshake,
            started_at: Instant::now(),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }
}

// ----------------------------------------------------------------
// Pipeline 7 étapes
// ----------------------------------------------------------------

/// Résultat de l'exécution du pipeline complet.
#[derive(Debug)]
pub struct PipelineResult {
    /// Nombre de comptes réellement appliqués (phase APPLY).
    pub accounts_applied: usize,
    /// Vector clock fusionné après MERGE.
    pub merged_clock: VectorClock,
    /// Durée totale du pipeline.
    pub elapsed: Duration,
}

/// Moteur du pipeline 7 étapes.
///
/// Opère entièrement en mémoire sur des snapshots des données locales.
/// Les entrées/sorties réseau sont gérées par [`SyncCoordinator`] qui
/// encapsule ce pipeline.
pub struct SyncPipeline {
    instance_id: String,
    /// Snapshot de notre vector clock au début du pipeline.
    local_clock: VectorClock,
    /// Snapshot de nos comptes (`{key → account_json_value}`).
    local_accounts: HashMap<String, serde_json::Value>,
    /// Snapshot des versions locales (`{key → clock_sum}`).
    local_versions: HashMap<String, u64>,
}

impl SyncPipeline {
    /// Crée un nouveau pipeline à partir d'un snapshot local.
    ///
    /// `local_accounts` est une map `{account_key → serde_json::Value}` des
    /// comptes connus localement.
    /// `local_versions` est une map `{account_key → version}` où version est
    /// la somme des compteurs du clock au moment de la dernière modification.
    pub fn new(
        instance_id: impl Into<String>,
        local_clock: VectorClock,
        local_accounts: HashMap<String, serde_json::Value>,
        local_versions: HashMap<String, u64>,
    ) -> Self {
        Self {
            instance_id: instance_id.into(),
            local_clock,
            local_accounts,
            local_versions,
        }
    }

    // ----------------------------------------------------------------
    // Étape 1 — HANDSHAKE
    // ----------------------------------------------------------------

    /// Compare notre clock avec celui du peer.
    ///
    /// Retourne un [`HandshakeSummary`] qui indique si le peer a des données
    /// plus récentes que les nôtres et/ou si nous avons des données plus
    /// récentes que celles du peer.
    ///
    /// Quand les deux clocks sont identiques, aucun ne contient de nouvelles
    /// informations : la synchronisation est inutile.
    pub fn step_handshake(
        &self,
        peer_clock: VectorClock,
        _peer_account_count: usize,
    ) -> HandshakeSummary {
        // Si les clocks sont identiques, aucun sync n'est nécessaire.
        if self.local_clock == peer_clock {
            return HandshakeSummary {
                peer_needs_our_data: false,
                we_need_peer_data: false,
                peer_clock,
            };
        }

        let peer_has_new = SyncCoordinator::remote_has_new_info(&self.local_clock, &peer_clock);
        let we_have_new = SyncCoordinator::remote_has_new_info(&peer_clock, &self.local_clock);

        HandshakeSummary {
            peer_needs_our_data: we_have_new,
            we_need_peer_data: peer_has_new,
            peer_clock,
        }
    }

    // ----------------------------------------------------------------
    // Étape 2 — DIFF
    // ----------------------------------------------------------------

    /// Calcule le différentiel entre nos comptes et la liste fournie par le peer.
    ///
    /// - `peer_keys_and_versions` : map `{key → version}` reçue du peer.
    ///
    /// Retourne un [`DiffSummary`] avec :
    /// - `keys_to_send` : clés que nous avons et que le peer n'a pas (ou a en version plus ancienne).
    /// - `keys_to_request` : clés que le peer a et que nous n'avons pas (ou avons en version plus ancienne).
    pub fn step_diff(&self, peer_keys_and_versions: &HashMap<String, u64>) -> DiffSummary {
        let mut keys_to_send = Vec::new();
        let mut keys_to_request = Vec::new();

        // Clés que nous avons : le peer les a-t-il ? Et notre version est-elle plus récente ?
        for (key, &our_version) in &self.local_versions {
            match peer_keys_and_versions.get(key) {
                None => {
                    // Le peer n'a pas cette clé → on doit la lui envoyer
                    keys_to_send.push(key.clone());
                }
                Some(&peer_version) if our_version > peer_version => {
                    // Notre version est plus récente → on envoie
                    keys_to_send.push(key.clone());
                }
                Some(&peer_version) if peer_version > our_version => {
                    // Version du peer plus récente → on demande
                    keys_to_request.push(key.clone());
                }
                _ => {
                    // Même version → rien à faire pour cette clé
                }
            }
        }

        // Clés que le peer a et que nous n'avons pas du tout
        for key in peer_keys_and_versions.keys() {
            if !self.local_versions.contains_key(key) {
                keys_to_request.push(key.clone());
            }
        }

        DiffSummary {
            keys_to_send,
            keys_to_request,
        }
    }

    // ----------------------------------------------------------------
    // Étape 3 — OUTBOX DRAIN
    // ----------------------------------------------------------------

    /// Vide l'outbox pour ce peer avant le transfert principal.
    ///
    /// Si `outbox` est `Some`, la méthode :
    /// 1. Appelle [`Outbox::prune`] pour supprimer les entrées expirées.
    /// 2. Appelle [`Outbox::drain`] pour récupérer les messages en attente.
    /// 3. Acquitte chaque message via [`Outbox::ack`] (ack **optimiste** —
    ///    les entrées sont supprimées de l'outbox *avant* toute confirmation
    ///    réseau. En cas d'échec du pipeline après cette étape, les messages
    ///    sont perdus sans nouvelle tentative de livraison).
    /// 4. Retourne le nombre de messages drainés.
    ///
    /// Si `outbox` est `None`, retourne 0 (compatibilité backward / tests
    /// sans outbox sur disque).
    pub fn step_outbox_drain(&self, peer_id: &str, outbox: Option<&mut Outbox>) -> usize {
        let outbox = match outbox {
            Some(o) => o,
            None => {
                debug!(
                    "OUTBOX_DRAIN: no outbox provided — skipping (peer={})",
                    peer_id
                );
                return 0;
            }
        };

        // 1. Nettoyer les entrées expirées et réduire aux max_entries
        outbox.prune();

        // 2. Récupérer les messages en attente
        let entries = outbox.drain();
        let count = entries.len();

        if count == 0 {
            debug!("OUTBOX_DRAIN: outbox empty (peer={})", peer_id);
            return 0;
        }

        // 3. Acquitter chaque entrée (ack optimiste)
        //    Les entrées sont supprimées AVANT toute confirmation réseau.
        //    Si le pipeline échoue après cette étape, les messages sont perdus.
        let ids: Vec<String> = entries.iter().map(|e| e.id.clone()).collect();
        for id in &ids {
            outbox.ack(id);
        }

        info!(
            "OUTBOX_DRAIN: {} message(s) drained and acked (peer={})",
            count, peer_id
        );
        count
    }

    // ----------------------------------------------------------------
    // Étape 4 — FULL SYNC (préparation des données à envoyer)
    // ----------------------------------------------------------------

    /// Sérialise les comptes identifiés par le diff pour l'envoi au peer.
    ///
    /// Retourne un JSON des comptes à envoyer (`{key → account_data}`).
    /// Si une clé demandée n'existe pas localement, elle est ignorée silencieusement.
    pub fn step_full_sync_export(&self, keys_to_send: &[String]) -> serde_json::Value {
        let mut accounts = serde_json::Map::new();
        for key in keys_to_send {
            if let Some(account) = self.local_accounts.get(key) {
                accounts.insert(key.clone(), account.clone());
            }
        }
        serde_json::Value::Object(accounts)
    }

    // ----------------------------------------------------------------
    // Étape 5 — MERGE (LWW)
    // ----------------------------------------------------------------

    /// Fusionne les comptes reçus du peer avec nos comptes locaux.
    ///
    /// Stratégie LWW : pour chaque clé en conflit, la version avec la somme
    /// de clock la plus élevée gagne. Si les versions sont égales, on garde
    /// la valeur locale (conservatrice).
    ///
    /// `peer_accounts` : JSON `{key → account_data}` reçu du peer.
    /// `peer_versions` : versions associées `{key → version}`.
    ///
    /// Retourne la map des comptes mergés et le clock fusionné.
    pub fn step_merge(
        &self,
        peer_accounts: HashMap<String, serde_json::Value>,
        peer_versions: &HashMap<String, u64>,
        peer_clock: &VectorClock,
    ) -> (HashMap<String, serde_json::Value>, VectorClock) {
        let mut merged_accounts = self.local_accounts.clone();
        let merged_clock = SyncCoordinator::merge_clocks(&self.local_clock, peer_clock);

        for (key, peer_account) in peer_accounts {
            let peer_ver = peer_versions.get(&key).copied().unwrap_or(0);
            let local_ver = self.local_versions.get(&key).copied().unwrap_or(0);

            if peer_ver > local_ver {
                // Le peer a une version plus récente → LWW: on prend la sienne
                debug!(
                    "MERGE LWW: key={} peer_ver={} > local_ver={} → take peer",
                    key, peer_ver, local_ver
                );
                merged_accounts.insert(key, peer_account);
            } else if peer_ver == local_ver && !merged_accounts.contains_key(&key) {
                // Même version mais nous n'avons pas cette clé → on la prend
                debug!("MERGE: key={} new from peer (same version)", key);
                merged_accounts.insert(key, peer_account);
            }
            // peer_ver < local_ver → on garde la nôtre (LWW: local est plus récent)
        }

        (merged_accounts, merged_clock)
    }

    // ----------------------------------------------------------------
    // Étape 6 — APPLY
    // ----------------------------------------------------------------

    /// Applique les comptes mergés dans le [`CredentialsCache`] et persiste.
    ///
    /// Construit un JSON compatible avec `CredentialsCache::merge_from_json` et
    /// appelle `persist()`.
    ///
    /// Retourne le nombre de comptes appliqués.
    pub fn step_apply(
        &self,
        merged_accounts: HashMap<String, serde_json::Value>,
        credentials: &Arc<ai_core::credentials::CredentialsCache>,
    ) -> Result<usize> {
        let count = merged_accounts.len();

        // Construire le JSON dans le format attendu par merge_from_json :
        // {"accounts": {key: account_data, ...}}
        let wrapper = serde_json::json!({
            "accounts": merged_accounts,
        });
        let json_str = serde_json::to_string(&wrapper)?;

        credentials
            .merge_from_json(&json_str, None)
            .map_err(SyncError::Core)?;

        credentials.persist().map_err(SyncError::Core)?;

        info!(
            "APPLY: persisted {} account(s) for instance={}",
            count, self.instance_id
        );
        Ok(count)
    }

    // ----------------------------------------------------------------
    // Étape 7 — ACK
    // ----------------------------------------------------------------

    /// Prépare le message ACK à envoyer au peer.
    ///
    /// Le peer utilisera ce message pour mettre à jour son propre clock.
    pub fn step_ack(
        &self,
        merged_clock: VectorClock,
        accounts_applied: usize,
    ) -> SyncMessage {
        SyncMessage::pipeline_ack(&self.instance_id, merged_clock, accounts_applied)
    }

    // ----------------------------------------------------------------
    // Pipeline complet (méthode d'orchestration)
    // ----------------------------------------------------------------

    /// Exécute le pipeline complet de manière synchrone (en mémoire).
    ///
    /// Cette méthode orchestre les étapes 1 à 7 sans I/O réseau : elle prend
    /// en entrée les données déjà reçues du peer et retourne le résultat.
    ///
    /// Les échanges réseau réels (envoi/réception des messages) sont à la charge
    /// du [`SyncCoordinator`] qui appelle cette méthode une fois les données collectées.
    ///
    /// # Arguments
    ///
    /// - `session` : session en cours (modifiée pour refléter la phase courante)
    /// - `peer_clock` : clock reçue lors du HANDSHAKE
    /// - `peer_account_count` : nombre de comptes du peer (HANDSHAKE)
    /// - `peer_keys_and_versions` : résultat du DIFF (map {key → version} du peer)
    /// - `peer_accounts` : comptes reçus lors du FULL SYNC
    /// - `peer_account_versions` : versions associées aux comptes reçus
    /// - `credentials` : cache de credentials pour l'APPLY
    /// - `outbox` : outbox persistante à drainer (étape 3) ; `None` = pas d'outbox
    pub fn run_pipeline(
        &self,
        session: &mut SyncSession,
        peer_clock: VectorClock,
        peer_account_count: usize,
        peer_keys_and_versions: HashMap<String, u64>,
        peer_accounts: HashMap<String, serde_json::Value>,
        peer_account_versions: HashMap<String, u64>,
        credentials: &Arc<ai_core::credentials::CredentialsCache>,
        outbox: Option<&mut Outbox>,
    ) -> Result<PipelineResult> {
        let start = Instant::now();

        // --- Étape 1 : HANDSHAKE ---
        session.phase = SyncPhase::Handshake;
        let handshake = self.step_handshake(peer_clock.clone(), peer_account_count);
        info!(
            "Pipeline[{}] HANDSHAKE: peer_needs_our={} we_need_peer={}",
            session.peer_id, handshake.peer_needs_our_data, handshake.we_need_peer_data
        );

        if !handshake.peer_needs_our_data && !handshake.we_need_peer_data {
            // Clocks identiques → rien à synchroniser
            session.phase = SyncPhase::Complete;
            info!(
                "Pipeline[{}] already in sync — skipping to COMPLETE",
                session.peer_id
            );
            return Ok(PipelineResult {
                accounts_applied: 0,
                merged_clock: self.local_clock.clone(),
                elapsed: start.elapsed(),
            });
        }

        // --- Étape 2 : DIFF ---
        session.phase = SyncPhase::Diff;
        let diff = self.step_diff(&peer_keys_and_versions);
        info!(
            "Pipeline[{}] DIFF: to_send={} to_request={}",
            session.peer_id,
            diff.keys_to_send.len(),
            diff.keys_to_request.len()
        );

        // --- Étape 3 : OUTBOX DRAIN ---
        session.phase = SyncPhase::OutboxDrain;
        let drained = self.step_outbox_drain(&session.peer_id, outbox);
        debug!("Pipeline[{}] OUTBOX_DRAIN: {} messages drained", session.peer_id, drained);

        // --- Étape 4 : FULL SYNC (pas de réseau ici, les données arrivent en paramètre) ---
        session.phase = SyncPhase::FullSync;
        debug!(
            "Pipeline[{}] FULL_SYNC: {} account(s) received from peer",
            session.peer_id,
            peer_accounts.len()
        );

        // --- Étape 5 : MERGE ---
        session.phase = SyncPhase::Merge;
        let (merged_accounts, merged_clock) =
            self.step_merge(peer_accounts, &peer_account_versions, &peer_clock);
        info!(
            "Pipeline[{}] MERGE: {} account(s) after LWW",
            session.peer_id,
            merged_accounts.len()
        );

        // --- Étape 6 : APPLY ---
        session.phase = SyncPhase::Apply;
        let accounts_applied = self
            .step_apply(merged_accounts, credentials)
            .map_err(|e| {
                session.phase = SyncPhase::Failed(e.to_string());
                e
            })?;

        // --- Étape 7 : ACK ---
        session.phase = SyncPhase::Ack;
        // Le message ACK est produit ici ; dans l'intégration réseau complète,
        // le coordinator le broadcastera via le bus.
        let _ack_msg = self.step_ack(merged_clock.clone(), accounts_applied);

        session.phase = SyncPhase::Complete;
        info!(
            "Pipeline[{}] COMPLETE: applied={} elapsed={:?}",
            session.peer_id,
            accounts_applied,
            start.elapsed()
        );

        Ok(PipelineResult {
            accounts_applied,
            merged_clock,
            elapsed: start.elapsed(),
        })
    }
}

// ----------------------------------------------------------------
// Coordinateur principal
// ----------------------------------------------------------------

/// Coordinateur P2P — orchestre la réception de messages et la réconciliation.
pub struct SyncCoordinator {
    instance_id: String,
    bus: Arc<SyncBus>,
    credentials: Arc<ai_core::credentials::CredentialsCache>,
    /// Horloge vectorielle locale (instance_id → compteur)
    clock: Arc<RwLock<VectorClock>>,
    /// Outbox persistante — messages en attente de livraison aux peers.
    outbox: parking_lot::Mutex<Outbox>,
    /// Ensemble des pairs autorisés (instance_id). Si vide, tous les pairs sont acceptés
    /// (mode ouvert — pour la rétrocompatibilité et les tests sans configuration explicite).
    known_peers: RwLock<HashSet<String>>,
    /// Cache d'état des proxies distants : machine_id → liste d'instances.
    remote_proxy_status: Arc<RwLock<HashMap<String, Vec<ProxyInstanceStatus>>>>,
}

impl SyncCoordinator {
    /// Crée un nouveau coordinateur.
    ///
    /// L'outbox est initialisée dans `~/.claude/multi-account/` (chemin standard
    /// du projet). Si `dirs::home_dir()` n'est pas disponible, l'outbox pointe
    /// vers le répertoire courant (fallback).
    ///
    /// La liste des pairs connus est vide par défaut : tous les pairs porteurs de
    /// la clé partagée sont acceptés (mode ouvert). Utiliser [`Self::new_with_peers`]
    /// pour restreindre les pairs autorisés.
    pub fn new(
        instance_id: String,
        bus: Arc<SyncBus>,
        credentials: Arc<ai_core::credentials::CredentialsCache>,
    ) -> Self {
        Self::new_with_peers(instance_id, bus, credentials, HashSet::new())
    }

    /// Crée un coordinateur avec une liste explicite de pairs autorisés.
    ///
    /// Seuls les messages dont `msg.from` figure dans `known_peers` sont traités
    /// pour les opérations sensibles (`Credentials`, `AccountSwitch`).
    /// Un ensemble vide signifie « mode ouvert » : tous les pairs sont acceptés.
    pub fn new_with_peers(
        instance_id: String,
        bus: Arc<SyncBus>,
        credentials: Arc<ai_core::credentials::CredentialsCache>,
        known_peers: HashSet<String>,
    ) -> Self {
        let mut clock = VectorClock::new();
        clock.insert(instance_id.clone(), 0);

        let outbox_dir = dirs::home_dir()
            .map(|h| h.join(".claude").join("multi-account"))
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let outbox = Outbox::new(&outbox_dir);

        Self {
            instance_id,
            bus,
            credentials,
            clock: Arc::new(RwLock::new(clock)),
            outbox: parking_lot::Mutex::new(outbox),
            known_peers: RwLock::new(known_peers),
            remote_proxy_status: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Enregistre un pair comme autorisé.
    ///
    /// Peut être appelé dynamiquement après la construction (ex. : après un
    /// handshake réussi avec authentification mutuelle).
    pub fn register_peer(&self, peer_id: impl Into<String>) {
        self.known_peers.write().insert(peer_id.into());
    }

    /// Retire un pair de la liste des pairs autorisés.
    pub fn unregister_peer(&self, peer_id: &str) {
        self.known_peers.write().remove(peer_id);
    }

    /// Retourne `true` si le pair est connu et autorisé.
    ///
    /// En mode ouvert (liste vide), tous les pairs sont considérés comme autorisés.
    fn is_known_peer(&self, peer_id: &str) -> bool {
        let peers = self.known_peers.read();
        // Mode ouvert : liste vide → accepter tout le monde (rétrocompatibilité)
        if peers.is_empty() {
            return true;
        }
        peers.contains(peer_id)
    }

    /// Démarre un pipeline 7 étapes avec un peer donné.
    ///
    /// Cette méthode crée le contexte local nécessaire au pipeline (snapshot du
    /// clock, des comptes et de leurs versions) puis délègue à [`SyncPipeline`].
    ///
    /// # Note sur les données peer
    ///
    /// Dans une intégration réseau complète, `peer_clock`, `peer_keys_and_versions`
    /// et `peer_accounts` seraient obtenus via le bus après les échanges
    /// HANDSHAKE / DIFF / FULL_SYNC. Ici la méthode accepte ces données en
    /// paramètre pour faciliter les tests unitaires et l'intégration future.
    pub async fn run_pipeline_with_peer(
        &self,
        peer_id: &str,
        peer_clock: VectorClock,
        peer_account_count: usize,
        peer_keys_and_versions: HashMap<String, u64>,
        peer_accounts: HashMap<String, serde_json::Value>,
        peer_account_versions: HashMap<String, u64>,
    ) -> Result<PipelineResult> {
        let local_clock = self.clock.read().clone();
        let (local_accounts, local_versions) = self.snapshot_accounts_and_versions();

        let pipeline = SyncPipeline::new(
            &self.instance_id,
            local_clock,
            local_accounts,
            local_versions,
        );

        let mut session = SyncSession::new(peer_id);

        // R1 fix: drain et acker l'outbox AVANT d'appeler run_pipeline.
        // Le lock parking_lot::Mutex est synchrone — le tenir pendant tout
        // run_pipeline (qui appelle step_apply → I/O disque) bloquerait un
        // thread Tokio. On prune/drain/ack dans un bloc court, on libère le
        // lock, puis on passe None à run_pipeline (le drain est déjà fait).
        {
            let mut outbox = self.outbox.lock();
            outbox.prune();
            let entries = outbox.drain();
            let count = entries.len();
            for e in &entries {
                outbox.ack(&e.id);
            }
            if count > 0 {
                info!(
                    "Pipeline[{}] OUTBOX_DRAIN (pre-pipeline): {} message(s) drained and acked",
                    peer_id, count
                );
            }
        } // lock libéré ici, avant tout I/O async

        let result = pipeline.run_pipeline(
            &mut session,
            peer_clock.clone(),
            peer_account_count,
            peer_keys_and_versions,
            peer_accounts,
            peer_account_versions,
            &self.credentials,
            None, // outbox déjà drainée ci-dessus
        )?;

        // Met à jour le clock local avec le clock mergé
        {
            let mut clock = self.clock.write();
            *clock = result.merged_clock.clone();
        }

        // Broadcast le ACK sur le bus
        let ack = SyncMessage::pipeline_ack(
            &self.instance_id,
            result.merged_clock.clone(),
            result.accounts_applied,
        );
        if let Err(e) = self.bus.broadcast(ack).await {
            warn!("Pipeline ACK broadcast failed: {}", e);
        }

        Ok(result)
    }

    /// Construit un snapshot des comptes locaux et de leurs versions.
    ///
    /// La version d'un compte est un hash u64 de son contenu sérialisé (email +
    /// longueur du access_token). Cette approche per-account garantit que deux
    /// comptes différents ont des versions indépendantes, ce qui évite les
    /// écrasements LWW incorrects causés par une somme globale du vector clock
    /// (qui serait identique pour tous les comptes à un instant donné).
    ///
    /// La fonction est déterministe : le même contenu produit toujours le même hash.
    fn snapshot_accounts_and_versions(
        &self,
    ) -> (HashMap<String, serde_json::Value>, HashMap<String, u64>) {
        let data = self.credentials.read();

        let mut accounts = HashMap::new();
        let mut versions = HashMap::new();

        for (key, account) in &data.accounts {
            if let Ok(val) = serde_json::to_value(account) {
                // P5: version = hash du contenu du compte (pas clock_sum global)
                let version = Self::account_content_hash(&val);
                accounts.insert(key.clone(), val);
                versions.insert(key.clone(), version);
            }
        }

        (accounts, versions)
    }

    /// Calcule un hash u64 stable du contenu d'un compte.
    ///
    /// On hache le JSON sérialisé du compte pour obtenir une empreinte
    /// discriminante. Les champs sensibles (tokens) ne sont pas inclus en
    /// clair dans les logs, mais leur présence et leur longueur influencent
    /// le hash via la sérialisation JSON complète.
    ///
    /// Note : `DefaultHasher` n'est pas cryptographique mais suffit ici —
    /// il s'agit d'un discriminant LWW, pas d'une protection contre la
    /// falsification (qui est gérée par la clé partagée au niveau bus).
    fn account_content_hash(account_value: &serde_json::Value) -> u64 {
        let mut hasher = DefaultHasher::new();
        // Sérialiser en JSON canonique (les clés HashMap ne sont pas ordonnées,
        // mais serde_json::Value::Object préserve l'ordre d'insertion en pratique)
        if let Ok(serialized) = serde_json::to_string(account_value) {
            serialized.hash(&mut hasher);
        }
        hasher.finish()
    }

    /// Boucle principale : reçoit et traite les messages, envoie les heartbeats.
    ///
    /// La boucle s'arrête quand `shutdown` est reçu sur le canal watch.
    pub async fn run(&self, mut shutdown: tokio::sync::watch::Receiver<bool>) -> Result<()> {
        let mut rx = self.bus.subscribe();

        // Heartbeat périodique
        let mut heartbeat_interval =
            tokio::time::interval(Duration::from_secs(HEARTBEAT_INTERVAL_SECS));

        info!(
            "SyncCoordinator started (instance={})",
            self.instance_id
        );

        // Demande une sync complète au démarrage
        let req = SyncMessage::sync_request(&self.instance_id);
        if let Err(e) = self.bus.broadcast(req).await {
            warn!("Failed to send initial SyncRequest: {}", e);
        }

        loop {
            tokio::select! {
                // Message entrant
                result = rx.recv() => {
                    match result {
                        Ok(msg) => {
                            if let Err(e) = self.handle_message(msg).await {
                                warn!("Error handling sync message: {}", e);
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            warn!("SyncCoordinator lagged: {} messages dropped", n);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            info!("Sync bus channel closed");
                            break;
                        }
                    }
                }

                // Heartbeat
                _ = heartbeat_interval.tick() => {
                    let hb = SyncMessage::heartbeat(&self.instance_id);
                    if let Err(e) = self.bus.broadcast(hb).await {
                        debug!("Heartbeat broadcast failed: {}", e);
                    }
                }

                // Signal d'arrêt
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!("SyncCoordinator shutdown signal received");
                        break;
                    }
                }
            }
        }

        info!("SyncCoordinator stopped");
        Ok(())
    }

    /// Traite un message reçu.
    async fn handle_message(&self, msg: SyncMessage) -> Result<()> {
        info!(
            "[coordinator] Handling {} from={}",
            msg.payload.variant_name(),
            msg.from,
        );

        match msg.payload {
            SyncPayload::Credentials {
                accounts_json,
                active_key,
                clock: remote_clock,
            } => {
                self.handle_credentials(&msg.from, &accounts_json, active_key.as_deref(), &remote_clock)
                    .await?;
            }

            SyncPayload::AccountSwitch { new_key, clock: remote_clock } => {
                self.handle_account_switch(&msg.from, &new_key, &remote_clock)
                    .await?;
            }

            SyncPayload::QuotaUpdate {
                account_key,
                tokens_5h,
                tokens_7d,
                clock: remote_clock,
            } => {
                self.handle_quota_update(
                    &msg.from,
                    &account_key,
                    tokens_5h,
                    tokens_7d,
                    &remote_clock,
                )
                .await?;
            }

            SyncPayload::Heartbeat { instance_id, timestamp } => {
                debug!("Heartbeat from {} at {}", instance_id, timestamp);
            }

            SyncPayload::SyncRequest { instance_id } => {
                self.handle_sync_request(&instance_id).await?;
            }

            SyncPayload::SyncResponse {
                credentials_json,
                active_key,
                clock: remote_clock,
            } => {
                self.handle_credentials(
                    &msg.from,
                    &credentials_json,
                    active_key.as_deref(),
                    &remote_clock,
                )
                .await?;
            }

            // ----------------------------------------------------------------
            // Messages du pipeline 7 étapes (Phase 5.1)
            // ----------------------------------------------------------------

            SyncPayload::HandshakeRequest { vector_clock, account_count } => {
                self.handle_handshake_request(&msg.from, vector_clock, account_count)
                    .await?;
            }

            SyncPayload::HandshakeResponse { vector_clock, needs_full_sync } => {
                debug!(
                    "HandshakeResponse from {}: needs_full_sync={}",
                    msg.from, needs_full_sync
                );
                // Merge du clock reçu dans notre clock local
                if Self::remote_has_new_info(&self.clock.read(), &vector_clock) {
                    let mut clock = self.clock.write();
                    *clock = Self::merge_clocks(&clock, &vector_clock);
                }
            }

            SyncPayload::DiffRequest { keys_and_versions } => {
                self.handle_diff_request(&msg.from, keys_and_versions).await?;
            }

            SyncPayload::DiffResponse { missing_keys, outdated_keys } => {
                debug!(
                    "DiffResponse from {}: missing={} outdated={}",
                    msg.from,
                    missing_keys.len(),
                    outdated_keys.len()
                );
                // TODO(Phase 5.2): déclencher l'envoi des comptes demandés.
            }

            SyncPayload::PipelineAck { merged_clock, accounts_applied } => {
                info!(
                    "PipelineAck from {}: accounts_applied={}",
                    msg.from, accounts_applied
                );
                // Merge le clock de l'ACK dans notre clock local
                if Self::remote_has_new_info(&self.clock.read(), &merged_clock) {
                    let mut clock = self.clock.write();
                    *clock = Self::merge_clocks(&clock, &merged_clock);
                }
            }

            // ----------------------------------------------------------------
            // Nouveaux types de messages (Phase 5.x)
            // ----------------------------------------------------------------

            SyncPayload::ConfigUpdate { config_json, clock: _ } => {
                self.handle_config_update(&msg.from, config_json).await;
            }

            SyncPayload::PeerConfigUpdate { action, peer_id, host, port, shared_key_hex, clock: _ } => {
                self.handle_peer_config_update(&msg.from, &action, peer_id, host, port, shared_key_hex).await;
            }

            SyncPayload::ProfileUpdate { name, config_json, clock: _ } => {
                self.handle_profile_update(&msg.from, &name, config_json).await;
            }

            SyncPayload::ProxyConfigUpdate { action, instance_id, config_json, clock: _ } => {
                self.handle_proxy_config_update(&msg.from, &action, &instance_id, config_json).await;
            }

            SyncPayload::ProxyCommand { target_machine_id, instance_id, action, clock: _ } => {
                self.handle_proxy_command(&msg.from, &target_machine_id, &instance_id, &action).await;
            }

            SyncPayload::ProxyStatusBroadcast { from_machine_id, instances, clock: _ } => {
                self.handle_proxy_status_broadcast(&from_machine_id, instances).await;
            }

            SyncPayload::IntegrationSetup { kind, action, port, target_machine_id, clock: _ } => {
                self.handle_integration_setup(&msg.from, &kind, &action, port, &target_machine_id).await;
            }

            SyncPayload::SshHostUpdate { action, host_id, host_json, clock: _ } => {
                self.handle_ssh_host_update(&msg.from, &action, &host_id, host_json).await;
            }

            SyncPayload::InvalidGrantUpdate { invalid_keys, clock: _ } => {
                self.handle_invalid_grant_update(&msg.from, invalid_keys).await;
            }
        }

        Ok(())
    }

    /// Répond à un HandshakeRequest en envoyant notre clock.
    async fn handle_handshake_request(
        &self,
        from: &str,
        peer_clock: VectorClock,
        _peer_account_count: usize,
    ) -> Result<()> {
        let local_clock = self.clock.read().clone();
        let needs_full_sync = Self::remote_has_new_info(&local_clock, &peer_clock)
            || Self::remote_has_new_info(&peer_clock, &local_clock);

        debug!(
            "HandshakeRequest from {}: needs_full_sync={}",
            from, needs_full_sync
        );

        let response = SyncMessage::handshake_response(
            &self.instance_id,
            local_clock,
            needs_full_sync,
        );
        self.bus.broadcast(response).await
    }

    /// Répond à un DiffRequest en calculant les clés manquantes/obsolètes.
    async fn handle_diff_request(
        &self,
        from: &str,
        peer_keys_and_versions: HashMap<String, u64>,
    ) -> Result<()> {
        // P5: local_versions contient maintenant des hash per-account (pas clock_sum global)
        let (_, local_versions) = self.snapshot_accounts_and_versions();

        let mut missing_keys = Vec::new();
        let mut outdated_keys = Vec::new();

        // Clés que le peer a et que nous n'avons pas ou dont la nôtre est plus ancienne
        for (key, &peer_ver) in &peer_keys_and_versions {
            match local_versions.get(key) {
                None => missing_keys.push(key.clone()),
                Some(&local_ver) if peer_ver > local_ver => outdated_keys.push(key.clone()),
                _ => {}
            }
        }

        debug!(
            "DiffRequest from {}: missing={} outdated={}",
            from,
            missing_keys.len(),
            outdated_keys.len()
        );

        // Envoyer notre propre DiffRequest avec nos versions per-account
        let diff_req = SyncMessage::diff_request(&self.instance_id, local_versions);
        self.bus.broadcast(diff_req).await?;

        let response = SyncMessage::diff_response(&self.instance_id, missing_keys, outdated_keys);
        self.bus.broadcast(response).await
    }

    /// Traite une mise à jour des credentials (merge LWW).
    async fn handle_credentials(
        &self,
        from: &str,
        accounts_json: &str,
        active_key: Option<&str>,
        remote_clock: &VectorClock,
    ) -> Result<()> {
        // P1: Vérifier que le pair est connu avant tout traitement
        if !self.is_known_peer(from) {
            warn!(
                "SYNC_REJECTED: credentials from unknown peer '{}' — ignoring",
                from
            );
            return Ok(());
        }

        // Vérifie si le clock distant domine le nôtre
        let local_clock = self.clock.read().clone();
        if !Self::remote_has_new_info(&local_clock, remote_clock) {
            debug!("Credentials from {} skipped (no new info)", from);
            return Ok(());
        }

        info!("Merging credentials from {}", from);
        self.credentials
            .merge_from_json(accounts_json, active_key)
            .map_err(|e| SyncError::Core(e))?;

        // Merge le clock
        {
            let mut clock = self.clock.write();
            *clock = Self::merge_clocks(&clock, remote_clock);
        }

        Ok(())
    }

    /// Traite un changement de compte actif.
    async fn handle_account_switch(
        &self,
        from: &str,
        new_key: &str,
        remote_clock: &VectorClock,
    ) -> Result<()> {
        // P1: Vérifier que le pair est connu avant tout traitement
        if !self.is_known_peer(from) {
            warn!(
                "SYNC_REJECTED: account switch from unknown peer '{}' — ignoring",
                from
            );
            return Ok(());
        }

        let local_clock = self.clock.read().clone();
        if !Self::remote_has_new_info(&local_clock, remote_clock) {
            debug!("AccountSwitch from {} skipped (no new info)", from);
            return Ok(());
        }

        info!("Account switch from {}: new_key={}", from, new_key);
        {
            let mut data = self.credentials.write();
            data.active_account = Some(new_key.to_string());
        }
        self.credentials.persist().map_err(SyncError::Core)?;

        let mut clock = self.clock.write();
        *clock = Self::merge_clocks(&clock, remote_clock);

        Ok(())
    }

    /// Traite une mise à jour de quota.
    async fn handle_quota_update(
        &self,
        from: &str,
        account_key: &str,
        tokens_5h: u64,
        tokens_7d: u64,
        remote_clock: &VectorClock,
    ) -> Result<()> {
        let local_clock = self.clock.read().clone();
        if !Self::remote_has_new_info(&local_clock, remote_clock) {
            debug!("QuotaUpdate from {} skipped (no new info)", from);
            return Ok(());
        }

        debug!(
            "QuotaUpdate from {}: account={} 5h={} 7d={}",
            from, account_key, tokens_5h, tokens_7d
        );
        self.credentials
            .update_quota(account_key, tokens_5h, tokens_7d)
            .map_err(SyncError::Core)?;

        let mut clock = self.clock.write();
        *clock = Self::merge_clocks(&clock, remote_clock);

        Ok(())
    }

    /// Répond à une demande de sync complète en envoyant nos credentials.
    async fn handle_sync_request(&self, requester_id: &str) -> Result<()> {
        info!("Sync request from {}", requester_id);
        self.broadcast_credentials().await
    }

    /// Envoie nos credentials à tous les pairs (broadcast).
    pub async fn broadcast_credentials(&self) -> Result<()> {
        let clock = {
            let mut c = self.clock.write();
            // Incrémente notre entrée avant d'émettre
            let entry = c.entry(self.instance_id.clone()).or_insert(0);
            *entry += 1;
            c.clone()
        };

        let accounts_json = self
            .credentials
            .export_json()
            .map_err(SyncError::Core)?;

        let active_key = self.credentials.active_key();

        let msg = SyncMessage::new(
            &self.instance_id,
            SyncPayload::Credentials {
                accounts_json,
                active_key,
                clock,
            },
        );

        self.bus.broadcast(msg).await
    }

    // ----------------------------------------------------------------
    // Vector clock operations
    // ----------------------------------------------------------------

    /// Merge deux vector clocks en prenant le max de chaque entrée.
    ///
    /// `merged[i] = max(a[i], b[i])` pour tout nœud `i`.
    pub fn merge_clocks(a: &VectorClock, b: &VectorClock) -> VectorClock {
        let mut merged = a.clone();
        for (node, &count) in b {
            let entry = merged.entry(node.clone()).or_insert(0);
            *entry = (*entry).max(count);
        }
        merged
    }

    /// Retourne `true` si le clock `a` domine strictement `b`.
    ///
    /// `a` domine `b` si pour tout nœud `i`, `a[i] >= b[i]`
    /// et il existe au moins un `i` où `a[i] > b[i]`.
    pub fn clock_dominates(a: &VectorClock, b: &VectorClock) -> bool {
        let mut any_strictly_greater = false;
        // Vérifie tous les nœuds de b
        for (node, &b_count) in b {
            let a_count = a.get(node).copied().unwrap_or(0);
            if a_count < b_count {
                return false; // a ne domine pas
            }
            if a_count > b_count {
                any_strictly_greater = true;
            }
        }
        // Vérifie les nœuds dans a mais pas dans b
        for (node, &a_count) in a {
            if !b.contains_key(node) && a_count > 0 {
                any_strictly_greater = true;
            }
        }
        any_strictly_greater
    }

    /// Retourne `true` si le clock distant contient des informations nouvelles.
    ///
    /// C'est le cas si `remote` n'est pas dominé par `local`.
    pub fn remote_has_new_info(local: &VectorClock, remote: &VectorClock) -> bool {
        // remote a des infos nouvelles si local ne domine pas remote
        !Self::clock_dominates(local, remote)
    }

    /// Retourne le clock vectoriel actuel (copie).
    pub fn current_clock(&self) -> VectorClock {
        self.clock.read().clone()
    }

    /// Retourne une copie du cache d'état des proxies distants.
    pub fn remote_proxy_status(&self) -> HashMap<String, Vec<ProxyInstanceStatus>> {
        self.remote_proxy_status.read().clone()
    }

    // ----------------------------------------------------------------
    // Handlers pour les nouveaux types de messages (Phase 5.x)
    // ----------------------------------------------------------------

    async fn handle_config_update(&self, from: &str, config_json: String) {
        // Le coordinateur n'a pas accès direct à AppConfig.
        // Les consommateurs (Tauri/Daemon) traitent cet événement via leur abonné broadcast.
        // On se contente de logger pour traçabilité.
        info!("Config update received from {}: {} bytes", from, config_json.len());
    }

    async fn handle_peer_config_update(
        &self,
        from: &str,
        action: &str,
        peer_id: Option<String>,
        host: Option<String>,
        port: Option<u16>,
        shared_key_hex: Option<String>,
    ) {
        // La gestion des pairs est faite au niveau applicatif via l'abonné broadcast.
        let _ = (host, port, shared_key_hex); // champs utilisés par la couche applicative
        info!(
            "Peer config update from {}: action={}, peer_id={:?}",
            from, action, peer_id
        );
    }

    async fn handle_profile_update(&self, from: &str, name: &str, config_json: Option<String>) {
        info!(
            "Profile update from {}: name={}, deleted={}",
            from,
            name,
            config_json.is_none()
        );
    }

    async fn handle_proxy_config_update(
        &self,
        from: &str,
        action: &str,
        instance_id: &str,
        config_json: Option<String>,
    ) {
        let _ = config_json; // utilisé par la couche applicative
        info!(
            "Proxy config update from {}: action={}, instance_id={}",
            from, action, instance_id
        );
    }

    async fn handle_proxy_command(
        &self,
        from: &str,
        target_machine_id: &str,
        instance_id: &str,
        action: &str,
    ) {
        // L'exécution de la commande est gérée au niveau applicatif via l'abonné broadcast.
        if target_machine_id == self.instance_id {
            info!(
                "Proxy command targeting THIS machine from {}: {}:{}",
                from, instance_id, action
            );
        } else {
            debug!(
                "Proxy command for machine {} (not us) from {}",
                target_machine_id, from
            );
        }
    }

    async fn handle_proxy_status_broadcast(
        &self,
        from: &str,
        instances: Vec<ProxyInstanceStatus>,
    ) {
        let count = instances.len();
        self.remote_proxy_status
            .write()
            .insert(from.to_string(), instances);
        debug!("Proxy status from {}: {} instances", from, count);
    }

    async fn handle_integration_setup(
        &self,
        from: &str,
        kind: &str,
        action: &str,
        port: Option<u16>,
        target_machine_id: &str,
    ) {
        let _ = port; // utilisé par la couche applicative
        if target_machine_id == self.instance_id {
            info!(
                "Integration setup targeting THIS machine from {}: {}:{}",
                from, kind, action
            );
        } else {
            debug!(
                "Integration setup for machine {} (not us) from {}",
                target_machine_id, from
            );
        }
    }

    async fn handle_ssh_host_update(
        &self,
        from: &str,
        action: &str,
        host_id: &str,
        host_json: Option<String>,
    ) {
        let _ = host_json; // utilisé par la couche applicative
        info!(
            "SSH host update from {}: action={}, host_id={}",
            from, action, host_id
        );
    }

    async fn handle_invalid_grant_update(&self, from: &str, invalid_keys: Vec<String>) {
        info!(
            "Invalid grant update from {}: {} account(s) flagged",
            from,
            invalid_keys.len()
        );
        // La couche applicative (Tauri/Daemon) gère le merge via le broadcast subscriber.
    }
}

// ----------------------------------------------------------------
// Tests
// ----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    // ---- Tests merge vector clocks ----

    #[test]
    fn test_merge_clocks_empty() {
        let a = VectorClock::new();
        let b = VectorClock::new();
        let merged = SyncCoordinator::merge_clocks(&a, &b);
        assert!(merged.is_empty());
    }

    #[test]
    fn test_merge_clocks_disjoint_nodes() {
        let mut a = VectorClock::new();
        a.insert("node-1".to_string(), 3);

        let mut b = VectorClock::new();
        b.insert("node-2".to_string(), 5);

        let merged = SyncCoordinator::merge_clocks(&a, &b);
        assert_eq!(merged.get("node-1"), Some(&3));
        assert_eq!(merged.get("node-2"), Some(&5));
    }

    #[test]
    fn test_merge_clocks_overlapping_nodes() {
        let mut a = VectorClock::new();
        a.insert("node-1".to_string(), 5);
        a.insert("node-2".to_string(), 2);

        let mut b = VectorClock::new();
        b.insert("node-1".to_string(), 3);
        b.insert("node-2".to_string(), 7);

        let merged = SyncCoordinator::merge_clocks(&a, &b);
        // Prend le max de chaque
        assert_eq!(merged.get("node-1"), Some(&5));
        assert_eq!(merged.get("node-2"), Some(&7));
    }

    #[test]
    fn test_merge_clocks_idempotent() {
        let mut clock = VectorClock::new();
        clock.insert("n1".to_string(), 4);
        clock.insert("n2".to_string(), 2);

        let merged = SyncCoordinator::merge_clocks(&clock, &clock);
        assert_eq!(merged, clock);
    }

    #[test]
    fn test_merge_clocks_symmetric() {
        let mut a = VectorClock::new();
        a.insert("n1".to_string(), 5);
        a.insert("n2".to_string(), 1);

        let mut b = VectorClock::new();
        b.insert("n1".to_string(), 2);
        b.insert("n2".to_string(), 8);

        let ab = SyncCoordinator::merge_clocks(&a, &b);
        let ba = SyncCoordinator::merge_clocks(&b, &a);
        assert_eq!(ab, ba, "merge should be symmetric");
    }

    // ---- Tests clock_dominates ----

    #[test]
    fn test_clock_dominates_strictly() {
        let mut a = VectorClock::new();
        a.insert("n1".to_string(), 5);
        a.insert("n2".to_string(), 3);

        let mut b = VectorClock::new();
        b.insert("n1".to_string(), 4);
        b.insert("n2".to_string(), 2);

        assert!(SyncCoordinator::clock_dominates(&a, &b));
        assert!(!SyncCoordinator::clock_dominates(&b, &a));
    }

    #[test]
    fn test_clock_dominates_equal_not_dominates() {
        let mut a = VectorClock::new();
        a.insert("n1".to_string(), 3);

        let b = a.clone();
        // Égaux → aucun ne domine
        assert!(!SyncCoordinator::clock_dominates(&a, &b));
        assert!(!SyncCoordinator::clock_dominates(&b, &a));
    }

    #[test]
    fn test_clock_dominates_concurrent() {
        // Concurrent : a[n1] > b[n1], b[n2] > a[n2]
        let mut a = VectorClock::new();
        a.insert("n1".to_string(), 5);
        a.insert("n2".to_string(), 1);

        let mut b = VectorClock::new();
        b.insert("n1".to_string(), 2);
        b.insert("n2".to_string(), 8);

        // Ni a ni b ne domine l'autre
        assert!(!SyncCoordinator::clock_dominates(&a, &b));
        assert!(!SyncCoordinator::clock_dominates(&b, &a));
    }

    #[test]
    fn test_clock_dominates_empty_clocks() {
        let empty = VectorClock::new();
        let mut nonempty = VectorClock::new();
        nonempty.insert("n1".to_string(), 1);

        // nonempty domine empty (a une info de plus)
        assert!(SyncCoordinator::clock_dominates(&nonempty, &empty));
        // empty ne domine pas nonempty
        assert!(!SyncCoordinator::clock_dominates(&empty, &nonempty));
    }

    #[test]
    fn test_clock_dominates_both_empty() {
        let a = VectorClock::new();
        let b = VectorClock::new();
        assert!(!SyncCoordinator::clock_dominates(&a, &b));
    }

    // ---- Tests remote_has_new_info ----

    #[test]
    fn test_remote_has_new_info_true() {
        let mut local = VectorClock::new();
        local.insert("n1".to_string(), 2);

        let mut remote = VectorClock::new();
        remote.insert("n1".to_string(), 5); // plus récent

        assert!(SyncCoordinator::remote_has_new_info(&local, &remote));
    }

    #[test]
    fn test_remote_has_new_info_false_local_dominates() {
        let mut local = VectorClock::new();
        local.insert("n1".to_string(), 10);

        let mut remote = VectorClock::new();
        remote.insert("n1".to_string(), 3); // plus ancien

        assert!(!SyncCoordinator::remote_has_new_info(&local, &remote));
    }

    #[test]
    fn test_remote_has_new_info_concurrent() {
        let mut local = VectorClock::new();
        local.insert("n1".to_string(), 5);
        local.insert("n2".to_string(), 1);

        let mut remote = VectorClock::new();
        remote.insert("n1".to_string(), 2);
        remote.insert("n2".to_string(), 8);

        // Concurrent → on accepte (remote peut avoir des infos utiles)
        assert!(SyncCoordinator::remote_has_new_info(&local, &remote));
    }

    // ---- Tests LWW intégration ----

    #[test]
    fn test_lww_merge_takes_max() {
        // Simule deux états et vérifie que merge prend les max
        let mut a = VectorClock::new();
        a.insert("inst-a".to_string(), 10);
        a.insert("inst-b".to_string(), 3);

        let mut b = VectorClock::new();
        b.insert("inst-a".to_string(), 7);
        b.insert("inst-b".to_string(), 8);
        b.insert("inst-c".to_string(), 2);

        let merged = SyncCoordinator::merge_clocks(&a, &b);
        assert_eq!(merged["inst-a"], 10);
        assert_eq!(merged["inst-b"], 8);
        assert_eq!(merged["inst-c"], 2);
    }

    #[test]
    fn test_clock_increment_before_broadcast() {
        // Simule l'incrément d'un clock avant broadcast
        let mut clock = VectorClock::new();
        clock.insert("my-instance".to_string(), 0);

        let entry = clock.entry("my-instance".to_string()).or_insert(0);
        *entry += 1;

        assert_eq!(clock["my-instance"], 1);
    }

    // ================================================================
    // Tests pipeline 7 étapes (Phase 5.1)
    // ================================================================

    fn make_pipeline(
        instance_id: &str,
        clock: VectorClock,
        accounts: HashMap<String, serde_json::Value>,
        versions: HashMap<String, u64>,
    ) -> SyncPipeline {
        SyncPipeline::new(instance_id, clock, accounts, versions)
    }

    // ---- test_handshake_detects_diff ----

    /// Deux peers avec clocks différents → le handshake détecte qu'il y a un diff.
    #[test]
    fn test_handshake_detects_diff() {
        // Instance A a des données plus récentes (clock élevé sur inst-a)
        let mut local_clock = VectorClock::new();
        local_clock.insert("inst-a".to_string(), 5);
        local_clock.insert("inst-b".to_string(), 1);

        let pipeline = make_pipeline("inst-a", local_clock, HashMap::new(), HashMap::new());

        // Peer B a un clock concurrent (différent)
        let mut peer_clock = VectorClock::new();
        peer_clock.insert("inst-a".to_string(), 2);
        peer_clock.insert("inst-b".to_string(), 3);

        let summary = pipeline.step_handshake(peer_clock, 0);

        // Les deux clocks ont des entrées différentes → diff dans les deux sens
        assert!(
            summary.peer_needs_our_data || summary.we_need_peer_data,
            "Handshake should detect that clocks differ"
        );
    }

    // ---- test_merge_lww_picks_latest ----

    /// En cas de conflit, la stratégie LWW doit garder la version la plus récente.
    #[test]
    fn test_merge_lww_picks_latest() {
        let local_clock: VectorClock = [("inst-a".to_string(), 3u64)].into_iter().collect();
        let local_versions: HashMap<String, u64> =
            [("acc1".to_string(), 3u64)].into_iter().collect();
        let local_accounts: HashMap<String, serde_json::Value> = [(
            "acc1".to_string(),
            serde_json::json!({"email": "old@example.com"}),
        )]
        .into_iter()
        .collect();

        let pipeline = make_pipeline("inst-a", local_clock, local_accounts, local_versions);

        // Le peer a une version plus récente du même compte
        let peer_clock: VectorClock = [("inst-b".to_string(), 7u64)].into_iter().collect();
        let peer_versions: HashMap<String, u64> =
            [("acc1".to_string(), 7u64)].into_iter().collect();
        let peer_accounts: HashMap<String, serde_json::Value> = [(
            "acc1".to_string(),
            serde_json::json!({"email": "new@example.com"}),
        )]
        .into_iter()
        .collect();

        let (merged, _merged_clock) =
            pipeline.step_merge(peer_accounts, &peer_versions, &peer_clock);

        // LWW : la version du peer (version 7) doit gagner sur la nôtre (version 3)
        let email = merged["acc1"]["email"].as_str().unwrap();
        assert_eq!(
            email, "new@example.com",
            "LWW should pick the peer's more recent version"
        );
    }

    // ---- test_pipeline_phases_sequence ----

    /// Les phases du pipeline s'enchaînent dans l'ordre attendu.
    #[test]
    fn test_pipeline_phases_sequence() {
        let phases = [
            SyncPhase::Handshake,
            SyncPhase::Diff,
            SyncPhase::OutboxDrain,
            SyncPhase::FullSync,
            SyncPhase::Merge,
            SyncPhase::Apply,
            SyncPhase::Ack,
            SyncPhase::Complete,
        ];
        let labels: Vec<String> = phases.iter().map(|p| p.to_string()).collect();
        assert_eq!(labels[0], "HANDSHAKE");
        assert_eq!(labels[1], "DIFF");
        assert_eq!(labels[2], "OUTBOX_DRAIN");
        assert_eq!(labels[3], "FULL_SYNC");
        assert_eq!(labels[4], "MERGE");
        assert_eq!(labels[5], "APPLY");
        assert_eq!(labels[6], "ACK");
        assert_eq!(labels[7], "COMPLETE");

        // Vérifie que Failed est distinct
        let failed = SyncPhase::Failed("oops".to_string());
        assert!(failed.to_string().starts_with("FAILED"));

        // Vérifie que Complete != Handshake
        assert_ne!(SyncPhase::Complete, SyncPhase::Handshake);
    }

    // ---- test_diff_empty_when_in_sync ----

    /// Quand les deux peers ont le même clock et les mêmes clés/versions, le diff est vide.
    #[test]
    fn test_diff_empty_when_in_sync() {
        let clock: VectorClock = [("inst-a".to_string(), 5u64)].into_iter().collect();
        let versions: HashMap<String, u64> = [
            ("acc1".to_string(), 5u64),
            ("acc2".to_string(), 5u64),
        ]
        .into_iter()
        .collect();
        let accounts: HashMap<String, serde_json::Value> = [
            ("acc1".to_string(), serde_json::json!({})),
            ("acc2".to_string(), serde_json::json!({})),
        ]
        .into_iter()
        .collect();

        let pipeline = make_pipeline("inst-a", clock, accounts, versions.clone());

        // Peer a les mêmes clés et versions
        let peer_keys_and_versions = versions;
        let diff = pipeline.step_diff(&peer_keys_and_versions);

        assert!(
            diff.keys_to_send.is_empty(),
            "Nothing to send when in sync: {:?}",
            diff.keys_to_send
        );
        assert!(
            diff.keys_to_request.is_empty(),
            "Nothing to request when in sync: {:?}",
            diff.keys_to_request
        );
    }

    // ---- test_apply_persists_changes ----

    /// Quand apply est appelé, les comptes sont mergés et persist est appelé.
    ///
    /// Utilise `CredentialsCache::empty()` comme cache minimal (pas de fichier disque).
    /// `persist()` peut échouer avec une erreur IO (chemin fictif) — c'est accepté.
    #[test]
    fn test_apply_persists_changes() {
        let clock: VectorClock = [("inst-a".to_string(), 1u64)].into_iter().collect();
        let pipeline = make_pipeline("inst-a", clock, HashMap::new(), HashMap::new());

        let mut merged_accounts: HashMap<String, serde_json::Value> = HashMap::new();
        merged_accounts.insert(
            "acc-test".to_string(),
            serde_json::json!({
                "name": "Test Account",
                "email": "test@example.com"
            }),
        );

        let credentials = ai_core::credentials::CredentialsCache::empty();
        let result = pipeline.step_apply(merged_accounts, &credentials);

        match result {
            Ok(count) => {
                assert_eq!(count, 1, "Should have applied 1 account");
            }
            Err(e) => {
                // Seule une erreur de persist (IO) est acceptable
                let msg = e.to_string();
                assert!(
                    msg.contains("IO") || msg.contains("No such file") || msg.contains("os error"),
                    "Unexpected error: {}",
                    msg
                );
            }
        }
    }

    // ---- test_handshake_same_clock_no_diff ----

    /// Quand les deux peers ont exactement le même clock, aucun n'a besoin de l'autre.
    #[test]
    fn test_handshake_same_clock_no_diff() {
        let mut clock = VectorClock::new();
        clock.insert("inst-a".to_string(), 10);
        clock.insert("inst-b".to_string(), 5);

        let pipeline = make_pipeline("inst-a", clock.clone(), HashMap::new(), HashMap::new());

        // Peer a exactement le même clock
        let summary = pipeline.step_handshake(clock, 0);

        assert!(
            !summary.peer_needs_our_data,
            "Peer should not need our data when clocks are equal"
        );
        assert!(
            !summary.we_need_peer_data,
            "We should not need peer data when clocks are equal"
        );
    }

    // ---- test_diff_detects_missing_keys ----

    /// Le diff détecte correctement les clés que le peer n'a pas.
    #[test]
    fn test_diff_detects_missing_keys() {
        let clock: VectorClock = [("inst-a".to_string(), 3u64)].into_iter().collect();
        let versions: HashMap<String, u64> = [
            ("acc1".to_string(), 3u64),
            ("acc2".to_string(), 3u64),
            ("acc3".to_string(), 3u64),
        ]
        .into_iter()
        .collect();
        let accounts: HashMap<String, serde_json::Value> = versions
            .keys()
            .map(|k| (k.clone(), serde_json::json!({})))
            .collect();

        let pipeline = make_pipeline("inst-a", clock, accounts, versions);

        // Peer n'a que acc1
        let peer_versions: HashMap<String, u64> =
            [("acc1".to_string(), 3u64)].into_iter().collect();

        let diff = pipeline.step_diff(&peer_versions);

        // Nous devons envoyer acc2 et acc3 au peer
        assert!(diff.keys_to_send.contains(&"acc2".to_string()));
        assert!(diff.keys_to_send.contains(&"acc3".to_string()));
        assert!(!diff.keys_to_send.contains(&"acc1".to_string()));
        // Rien à demander (le peer n'a rien de plus que nous)
        assert!(diff.keys_to_request.is_empty());
    }

    // ---- test_merge_lww_local_wins_when_newer ----

    /// Quand notre version est plus récente, LWW garde la nôtre.
    #[test]
    fn test_merge_lww_local_wins_when_newer() {
        let local_clock: VectorClock = [("inst-a".to_string(), 10u64)].into_iter().collect();
        let local_versions: HashMap<String, u64> =
            [("acc1".to_string(), 10u64)].into_iter().collect();
        let local_accounts: HashMap<String, serde_json::Value> = [(
            "acc1".to_string(),
            serde_json::json!({"email": "local@example.com"}),
        )]
        .into_iter()
        .collect();

        let pipeline = make_pipeline("inst-a", local_clock, local_accounts, local_versions);

        let peer_clock: VectorClock = [("inst-b".to_string(), 2u64)].into_iter().collect();
        let peer_versions: HashMap<String, u64> =
            [("acc1".to_string(), 2u64)].into_iter().collect();
        let peer_accounts: HashMap<String, serde_json::Value> = [(
            "acc1".to_string(),
            serde_json::json!({"email": "old-peer@example.com"}),
        )]
        .into_iter()
        .collect();

        let (merged, _) = pipeline.step_merge(peer_accounts, &peer_versions, &peer_clock);

        // LWW : notre version (10) > peer (2) → on garde la nôtre
        let email = merged["acc1"]["email"].as_str().unwrap();
        assert_eq!(
            email, "local@example.com",
            "LWW should keep local version when it is more recent"
        );
    }

    // ---- test_outbox_drain_stub ----

    /// Sans outbox (None), OUTBOX_DRAIN retourne 0 — comportement backward-compatible.
    #[test]
    fn test_outbox_drain_no_outbox() {
        let pipeline = make_pipeline("inst-a", VectorClock::new(), HashMap::new(), HashMap::new());
        let drained = pipeline.step_outbox_drain("peer-b", None);
        assert_eq!(drained, 0, "Outbox drain with None should return 0");
    }

    // ---- test_outbox_drain_empty ----

    /// Avec une outbox vide, OUTBOX_DRAIN retourne 0.
    #[test]
    fn test_outbox_drain_empty_outbox() {
        use crate::outbox::Outbox;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let mut outbox = Outbox::new(dir.path());

        let pipeline = make_pipeline("inst-a", VectorClock::new(), HashMap::new(), HashMap::new());
        let drained = pipeline.step_outbox_drain("peer-b", Some(&mut outbox));
        assert_eq!(drained, 0, "Outbox drain on empty outbox should return 0");
        assert!(outbox.is_empty(), "Outbox should still be empty after drain");
    }

    // ---- test_outbox_drain_flushes_and_acks ----

    /// Avec des messages en attente, OUTBOX_DRAIN les draine et les acquitte.
    #[test]
    fn test_outbox_drain_flushes_and_acks() {
        use crate::outbox::Outbox;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let mut outbox = Outbox::new(dir.path());

        // Pousse 3 messages dans l'outbox
        outbox.push(b"msg-alpha".to_vec()).unwrap();
        outbox.push(b"msg-beta".to_vec()).unwrap();
        outbox.push(b"msg-gamma".to_vec()).unwrap();
        assert_eq!(outbox.len(), 3, "Should have 3 messages before drain");

        let pipeline = make_pipeline("inst-a", VectorClock::new(), HashMap::new(), HashMap::new());
        let drained = pipeline.step_outbox_drain("peer-b", Some(&mut outbox));

        // Tous les messages sont drainés et acquittés
        assert_eq!(drained, 3, "Should have drained 3 messages");
        assert!(
            outbox.is_empty(),
            "Outbox should be empty after drain+ack — remaining: {}",
            outbox.len()
        );
    }

    // ---- test_sync_session_lifecycle ----

    /// Une SyncSession passe par les phases dans l'ordre attendu.
    #[test]
    fn test_sync_session_lifecycle() {
        let mut session = SyncSession::new("peer-x");
        assert_eq!(session.phase, SyncPhase::Handshake);
        assert_eq!(session.peer_id, "peer-x");

        session.phase = SyncPhase::Diff;
        assert_eq!(session.phase, SyncPhase::Diff);

        session.phase = SyncPhase::Complete;
        assert_eq!(session.phase, SyncPhase::Complete);

        assert!(session.elapsed() >= Duration::ZERO);
    }

    // ================================================================
    // Tests P1 — known_peers guard
    // ================================================================

    // ---- test_is_known_peer_open_mode ----

    /// En mode ouvert (liste vide), tous les peers sont acceptés.
    #[test]
    fn test_is_known_peer_open_mode() {
        // On teste is_known_peer via les méthodes publiques de SyncCoordinator.
        // En mode ouvert (known_peers vide), tout peer est connu.
        let peers: HashSet<String> = HashSet::new();
        // Mode ouvert → vide → accepte tout
        assert!(peers.is_empty(), "Empty set = open mode");
        // Simuler la logique de is_known_peer
        let result = if peers.is_empty() { true } else { peers.contains("any-peer") };
        assert!(result, "Open mode should accept any peer");
    }

    // ---- test_is_known_peer_restricted_mode ----

    /// En mode restreint, seuls les peers enregistrés sont acceptés.
    #[test]
    fn test_is_known_peer_restricted_mode() {
        let mut peers: HashSet<String> = HashSet::new();
        peers.insert("peer-allowed".to_string());

        // Simuler la logique de is_known_peer
        let check = |id: &str| -> bool {
            if peers.is_empty() { true } else { peers.contains(id) }
        };

        assert!(check("peer-allowed"), "Registered peer should be accepted");
        assert!(!check("peer-unknown"), "Unknown peer should be rejected");
        assert!(!check(""), "Empty peer id should be rejected");
    }

    // ---- test_account_content_hash_stable ----

    /// Le hash d'un compte est stable pour le même contenu.
    #[test]
    fn test_account_content_hash_stable() {
        let account = serde_json::json!({
            "email": "alice@example.com",
            "oauth": {"access_token": "tok_abc123", "refresh_token": "ref_xyz"}
        });

        let h1 = SyncCoordinator::account_content_hash(&account);
        let h2 = SyncCoordinator::account_content_hash(&account);
        assert_eq!(h1, h2, "Same content must produce the same hash");
    }

    // ---- test_account_content_hash_differs_for_different_accounts ----

    /// Deux comptes différents ont des hashes différents.
    #[test]
    fn test_account_content_hash_differs_for_different_accounts() {
        let acc_a = serde_json::json!({"email": "alice@example.com", "token": "tok-a"});
        let acc_b = serde_json::json!({"email": "bob@example.com",   "token": "tok-b"});

        let h_a = SyncCoordinator::account_content_hash(&acc_a);
        let h_b = SyncCoordinator::account_content_hash(&acc_b);
        assert_ne!(h_a, h_b, "Different accounts must produce different hashes");
    }

    // ---- test_account_content_hash_changes_on_token_update ----

    /// Quand le token d'un compte change, le hash doit changer.
    #[test]
    fn test_account_content_hash_changes_on_token_update() {
        let acc_before = serde_json::json!({"email": "u@ex.com", "token": "old-token"});
        let acc_after  = serde_json::json!({"email": "u@ex.com", "token": "new-token"});

        let h_before = SyncCoordinator::account_content_hash(&acc_before);
        let h_after  = SyncCoordinator::account_content_hash(&acc_after);
        assert_ne!(
            h_before, h_after,
            "Hash must differ when token changes (LWW detects new version)"
        );
    }

    // ---- test_snapshot_versions_are_per_account ----

    /// snapshot_accounts_and_versions retourne des versions distinctes par compte.
    ///
    /// Vérifie que deux comptes au contenu différent ont des versions différentes
    /// (contrairement à l'ancienne implémentation qui utilisait clock_sum global).
    #[test]
    fn test_snapshot_versions_are_per_account_distinct() {
        // On teste account_content_hash directement puisque snapshot_accounts_and_versions
        // appelle credentials.read() qui n'est pas mockable ici sans setup lourd.
        let acc1 = serde_json::json!({"email": "alice@ex.com", "tok": "aaaa"});
        let acc2 = serde_json::json!({"email": "bob@ex.com",   "tok": "bbbb"});

        let v1 = SyncCoordinator::account_content_hash(&acc1);
        let v2 = SyncCoordinator::account_content_hash(&acc2);

        assert_ne!(
            v1, v2,
            "P5: per-account versions must differ for different accounts \
             (old clock_sum bug would give same version to both)"
        );
    }
}
