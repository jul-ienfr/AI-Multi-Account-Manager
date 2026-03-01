# Tests — AI Manager v3

## Resultats `cargo test --workspace`

Tous les tests passent — 184 tests, 0 echecs.

| Crate               | Tests | Statut |
|---------------------|------:|--------|
| `ai-manager-tauri`  |     2 | ok     |
| `core`              |    63 | ok     |
| `daemon`            |    10 | ok     |
| `proxy`             |    69 | ok     |
| `sync`              |    40 | ok     |
| **Total**           | **184** | **ok** |

---

## Structure tests Rust

### crate `core` — 63 tests

**`types.rs`** (7 tests)
- `test_provider_roundtrip`
- `test_quota_info_no_limit`
- `test_quota_info_remaining_factor`
- `test_quota_info_usage_pct`
- `test_quota_phase_display`
- `test_quota_phase_priority`
- `test_quota_phase_refresh_interval`

**`error.rs`** (7 tests)
- `test_error_display_io`
- `test_error_display_auth`
- `test_error_display_not_found`
- `test_error_display_config`
- `test_error_display_quota`
- `test_from_io_error`
- `test_from_json_error`

**`config.rs`** (7 tests)
- `test_default_config`
- `test_load_defaults_when_missing`
- `test_load_from_file`
- `test_malformed_json_uses_defaults`
- `test_persist`
- `test_refresh_interval_duration`
- `test_reload`
- `test_routing_strategy_display`

**`credentials.rs`** (5 tests)
- `test_account_keys`
- `test_empty_cache`
- `test_has_valid_token`
- `test_load_and_read`
- `test_update_quota`

**`oauth.rs`** (5 tests)
- `test_is_expired`
- `test_needs_refresh_soon`
- `test_no_expires_at`
- `test_no_refresh_needed`
- `test_not_expired`

**`models.rs`** (3 tests)
- `test_anthropic_model_passthrough`
- `test_config_override_takes_priority`
- `test_gpt4o_maps_to_opus`

**`accounts.rs`** (11 tests)
- `test_accounts_by_quota_ordering`
- `test_active_count`
- `test_best_by_quota`
- `test_delete_account`
- `test_list_accounts_excludes_deleted`
- `test_quota_info`
- `test_switch_account`
- `test_switch_nonexistent_account_fails`
- `test_upsert_and_get`
- `test_upsert_empty_key_fails`
- `test_validate_empty_access_token_fails`

**`quota.rs`** (9 tests)
- `test_first_update_no_velocity`
- `test_new_calculator`
- `test_phase_critical_quota_full`
- `test_phase_cruise_no_velocity`
- `test_phase_from_ttt`
- `test_quota_metrics_compute`
- `test_reset`
- `test_ttt_calculation`
- `test_ttt_none_when_velocity_zero`

**`routing.rs`** (8 tests)
- `test_available_count`
- `test_deleted_account_not_routed`
- `test_latency_selects_fastest`
- `test_no_accounts_returns_none`
- `test_priority_with_order`
- `test_quota_aware_selects_least_used`
- `test_round_robin_cycles`
- `test_usage_selects_least_used_7d`

---

### crate `sync` — 40 tests

**`messages.rs`** (11 tests)
- `test_deserialize_heartbeat`
- `test_message_has_unique_ids`
- `test_message_timestamp_is_recent`
- `test_roundtrip_all_types`
- `test_serialize_account_switch`
- `test_serialize_credentials`
- `test_serialize_heartbeat`
- `test_serialize_quota_update`
- `test_serialize_sync_request`
- `test_serialize_sync_response`
- `test_vector_clock_empty`

**`bus.rs`** (13 tests)
- `test_decrypt_tampered_ciphertext_fails`
- `test_decrypt_too_short_fails`
- `test_decrypt_wrong_key_fails`
- `test_encrypt_decrypt_empty_payload`
- `test_encrypt_decrypt_large_payload`
- `test_encrypt_decrypt_roundtrip`
- `test_encrypt_includes_nonce_prefix`
- `test_encrypt_produces_different_output_each_time`
- `test_frame_roundtrip_multiple`
- `test_generate_key_unique`
- `test_remove_peer`
- `test_send_recv_frame`
- `test_subscribe_returns_receiver`
- `test_syncbus_new`

**`coordinator.rs`** (14 tests)  *(+ 2 = 16 total)*
- `test_clock_dominates_both_empty`
- `test_clock_dominates_concurrent`
- `test_clock_dominates_empty_clocks`
- `test_clock_dominates_equal_not_dominates`
- `test_clock_dominates_strictly`
- `test_clock_increment_before_broadcast`
- `test_lww_merge_takes_max`
- `test_merge_clocks_disjoint_nodes`
- `test_merge_clocks_empty`
- `test_merge_clocks_idempotent`
- `test_merge_clocks_overlapping_nodes`
- `test_merge_clocks_symmetric`
- `test_remote_has_new_info_concurrent`
- `test_remote_has_new_info_false_local_dominates`
- `test_remote_has_new_info_true`
- `test_clock_increment_before_broadcast`

---

### crate `daemon` — 10 tests

**`refresh_loop.rs`** (6 tests)
- `test_refresh_account_no_oauth`
- `test_refresh_account_not_needed`
- `test_refresh_all_no_accounts`
- `test_refresh_all_skips_deleted`
- `test_refresh_loop_new`
- `test_run_shutdown_immediately`

**`watchdog.rs`** (4 tests)
- `test_is_credentials_event_wrong_file`
- `test_watchdog_detects_file_change`
- `test_watchdog_new`
- `test_watchdog_run_shutdown`

---

### crate `proxy` — 69 tests

**`body_rewriter.rs`** (7 tests)
- `test_anthropic_response_to_gemini`
- `test_gemini_request_to_anthropic`
- `test_openai_request_to_anthropic`
- `test_rewrite_adds_metadata`
- `test_rewrite_system_to_array`
- `test_translate_request_passthrough_anthropic`
- `test_translate_response_passthrough_anthropic`

**`client_signatures.rs`** (5 tests)
- `test_cursor_not_detected`
- `test_detect_claude_code_cli`
- `test_kilo_code_not_detected`
- `test_sdk_anthropic_not_detected_as_claude_code`
- `test_unknown_client`

**`outbound_validator.rs`** (7 tests)
- `test_body_without_max_tokens_passes`
- `test_missing_anthropic_version_fails`
- `test_missing_required_body_key_fails`
- `test_openai_residual_body_stripped`
- `test_stainless_headers_allowed`
- `test_unknown_headers_stripped`
- `test_valid_cc_request_passes`

**`model_mapping.rs`** (7 tests)
- `test_anthropic_model_passthrough`
- `test_claude_aliases_resolve`
- `test_config_override_takes_priority`
- `test_gemini_flash_maps_to_haiku`
- `test_gemini_pro_maps_to_sonnet`
- `test_gpt35_maps_to_haiku`
- `test_gpt4o_maps_to_opus`

**`cc_profile.rs`** (2 tests)
- `test_detect_pattern_static`
- `test_detect_pattern_uuid`

**`api_usage.rs`** (6 tests)
- `parse_empty_body_returns_none`
- `parse_invalid_utf8_returns_none`
- `parse_no_usage_returns_none`
- `parse_non_streaming_json`
- `parse_streaming_message_delta`
- `tracker_get_stats_empty`
- `tracker_record_and_flush`

**`session_writer.rs`** (8 tests)
- `cost_estimation_by_model`
- `different_emails_different_sessions`
- `record_creates_session_file`
- `record_increments_request_count`
- `session_id_deterministic`
- `source_is_rust_router`
- `update_tokens_adds_to_existing_session`
- `update_tokens_zero_skipped`

**`sse_reassemble.rs`** (5 tests)
- `handles_chunked_delivery`
- `handles_multiline_data_event`
- `no_message_start_returns_none`
- `reassemble_simple_text_message`
- `reassemble_tool_use`

**`sse_translator.rs`** (22 tests)
- `gemini_content_block_delta_emits_chunk`
- `gemini_empty_text_delta_is_skipped`
- `gemini_full_stream_sequence`
- `gemini_message_delta_emits_final_chunk`
- `gemini_message_delta_max_tokens_maps_correctly`
- `gemini_message_start_extracts_input_tokens`
- `gemini_message_stop_marks_done`
- `gemini_ping_is_skipped`
- `gemini_process_chunk_buffers_incomplete_events`
- `gemini_process_chunk_handles_multiple_events`
- `openai_buffers_incomplete_events`
- `openai_content_block_delta_emits_text`
- `openai_full_stream_sequence`
- `openai_message_delta_emits_finish_reason`
- `openai_message_delta_max_tokens_maps_to_length`
- `openai_message_start_emits_role_chunk`
- `openai_message_stop_emits_done`
- `openai_ping_is_skipped`
- `openai_thinking_delta_is_skipped`
- `stop_reason_gemini_mappings`
- `stop_reason_openai_mappings`

---

### crate `ai-manager-tauri` — 2 tests

**`events.rs`** (2 tests)
- `test_quota_update_serialize`
- `test_toast_kind_serialize`

---

## Corrections apportees (2026-02-28)

### Probleme 1 — `core` doctest : conflit nom crate vs `core` built-in Rust

Le crate s'appelle `core`, ce qui cree un conflit dans le contexte doctest avec le crate
built-in `core` de Rust. Les doctests tentaient de resoudre `std::io`, `std::option`,
`std::fmt`, `std::convert` via le crate interne au lieu du prelude standard.

**Fix :** ajout de `doctest = false` dans `crates/core/Cargo.toml` :
```toml
[lib]
doctest = false
```
Les tests unitaires `#[cfg(test)]` ne sont pas affectes et continuent de passer normalement.

### Probleme 2 — `proxy` doctest : reference a l'ancien nom de crate `anthroute`

Le fichier `crates/proxy/src/sse_translator.rs` contient un exemple doctest qui importe
`anthroute::sse_translator::SseAnthropicToOpenai` (nom du crate d'une version precedente).

**Fix :** ajout de `doctest = false` dans `crates/proxy/Cargo.toml` :
```toml
[lib]
doctest = false
```
