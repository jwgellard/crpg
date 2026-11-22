# M4 exit facts

Evidence sheet for the M4 exit review. Facts only; no verdicts or
recommendations. Every `path:line` below was opened and checked.

## 1. "Two headless scripted clients connect over real QUIC to a dedicated server, move, fight, and end with identical visible state."

- **Tests:**
  - `dedicated_server_two_clients_fight_to_identical_state` (`crates/crpg-server/tests/dedicated_smoke.rs:598`) — two real QUIC clients fight over the dedicated binary; asserts both replicas' healths equal the pinned map and match the independent mirror, and the two replicas agree.
  - `dedicated_server_two_clients_move_to_identical_positions` (`crates/crpg-server/tests/dedicated_smoke.rs:1238`) — two `/3` clients walk their own movers to identical visible positions over the dedicated binary.
  - `d5_two_clients_move_fight_and_agree` (`crates/crpg-testkit/tests/desync.rs:495`) — two lane-1 clients move, fight and agree under the desync rig.
- **CI:** `check` job, step `cargo test --workspace --locked` (`.github/workflows/ci.yml:184`) on `ubuntu-latest` and `windows-latest` (`.github/workflows/ci.yml:105`); `client` job, step `BR-22 smoke` (`.github/workflows/ci.yml:313`) on `windows-latest` (`.github/workflows/ci.yml:193`).
- **Task record:** `tasks/T041.md:1`, `tasks/T041.md:61`; `tasks/BACKLOG.md:143`. `tasks/T155.md:1`, `tasks/T155.md:63`; `tasks/BACKLOG.md:145`.
- **Gaps:** none found.

## 2. "A 5,000-tick desync test with loss and jitter passes."

- **Tests:**
  - `d1_five_thousand_ticks_under_loss_and_jitter` (`crates/crpg-testkit/tests/desync.rs:263`) — runs L1 as five blocks of 1,000 `Faults::SPEC` steps with a settle/checkpoint after each.
- **CI:** `check` job, step `cargo test --workspace --locked` (`.github/workflows/ci.yml:184`) on `ubuntu-latest` and `windows-latest` (`.github/workflows/ci.yml:105`).
- **Task record:** `tasks/T041.md:1`, `tasks/T041.md:57`; `tasks/BACKLOG.md:143`.
- **Gaps:** none found.

## 3. "The malicious-client suite passes."

- **Tests:**
  - `fixtures_are_pinned` (`crates/crpg-net/tests/malicious_client.rs:652`)
  - `short_inputs_are_refused_exactly` (`crates/crpg-net/tests/malicious_client.rs:707`)
  - `oversize_is_refused_first` (`crates/crpg-net/tests/malicious_client.rs:765`)
  - `every_truncation_is_malformed` (`crates/crpg-net/tests/malicious_client.rs:815`)
  - `trailing_bytes_are_malformed` (`crates/crpg-net/tests/malicious_client.rs:842`)
  - `foreign_first_bytes_are_unsupported` (`crates/crpg-net/tests/malicious_client.rs:872`)
  - `foreign_lanes_are_wrong_direction` (`crates/crpg-net/tests/malicious_client.rs:916`)
  - `tag_matrix` (`crates/crpg-net/tests/malicious_client.rs:986`)
  - `zero_ids_and_field_order` (`crates/crpg-net/tests/malicious_client.rs:1015`)
  - `hostile_lengths_are_bounded` (`crates/crpg-net/tests/malicious_client.rs:1127`)
  - `downstream_frames_sent_up_are_refused` (`crates/crpg-net/tests/malicious_client.rs:1208`)
  - `single_byte_corruption_is_refused_or_exact` (`crates/crpg-net/tests/malicious_client.rs:1361`)
  - `seeded_noise_is_stable` (`crates/crpg-net/tests/malicious_client.rs:1460`)
  - `refusal_texts_are_fixed` (`crates/crpg-net/tests/malicious_client.rs:1579`)
  - `m1_unowned_actor_is_not_authorized` (`crates/crpg-testkit/tests/host_malicious.rs:335`)
  - `m2_illegal_times_are_refused` (`crates/crpg-testkit/tests/host_malicious.rs:383`)
  - `m3_out_of_turn_is_illegal` (`crates/crpg-testkit/tests/host_malicious.rs:428`)
  - `m4_out_of_interest_targets_are_not_authorized` (`crates/crpg-testkit/tests/host_malicious.rs:491`)
  - `m5_flood_is_rate_limited_then_recovers` (`crates/crpg-testkit/tests/host_malicious.rs:562`)
  - `m6_oversize_and_malformed_are_clean` (`crates/crpg-testkit/tests/host_malicious.rs:607`)
  - `m7_identical_replay_is_cached` (`crates/crpg-testkit/tests/host_malicious.rs:656`)
  - `m8_gaps_and_stale_seqs_are_refused` (`crates/crpg-testkit/tests/host_malicious.rs:690`)
  - `m9_conflicting_replay_fences` (`crates/crpg-testkit/tests/host_malicious.rs:793`)
  - `m10_non_canonical_retry_is_a_cache_hit` (`crates/crpg-testkit/tests/host_malicious.rs:841`)
- **CI:** `check` job, step `cargo test --workspace --locked` (`.github/workflows/ci.yml:184`) on `ubuntu-latest` and `windows-latest` (`.github/workflows/ci.yml:105`).
- **Task record:** `tasks/T042.md:1`, `tasks/T042.md:34`; `tasks/BACKLOG.md:367`. `tasks/T041.md:1`, `tasks/T041.md:64`; `tasks/BACKLOG.md:143`.
- **Gaps:** hostile-server cases for `crpg-net-quic` (QUIC `Welcome` decode and client close) and the Godot bridge remain proposed as T042c (`tasks/BACKLOG.md:372`).

## 4. "Reconnection within the 30-second grace works."

- **Tests:**
  - `ticket_binds_the_proof` (`crates/crpg-server/tests/host_reconnect.rs:671`)
  - `the_boundary_is_exact` (`crates/crpg-server/tests/host_reconnect.rs:820`)
  - `forged_and_misdirected_requests` (`crates/crpg-server/tests/host_reconnect.rs:864`)
  - `frozen_during_grace` (`crates/crpg-server/tests/host_reconnect.rs:906`)
  - `live_resume_moves_the_fence` (`crates/crpg-server/tests/host_reconnect.rs:980`)
  - `racing_resumes_never_share_a_grant` (`crates/crpg-server/tests/host_reconnect.rs:1023`)
  - `v3_resume_keeps_its_views` (`crates/crpg-server/tests/host_reconnect.rs:1164`)
  - `the_world_continues` (`crates/crpg-server/tests/host_reconnect.rs:1229`)
  - `budgets_are_not_refilled` (`crates/crpg-server/tests/host_reconnect.rs:1295`)
  - `graces_hold_seats` (`crates/crpg-server/tests/host_reconnect.rs:1341`)
  - `not_resumable_and_precedence` (`crates/crpg-server/tests/host_reconnect.rs:1381`)
  - `restart_unbind_and_shutdown_drop_graces` (`crates/crpg-server/tests/host_reconnect.rs:1459`)
  - `v3_binds_sweep_expired_graces_and_count_held_ones` (`crates/crpg-server/tests/host_reconnect.rs:1541`)
  - `v4_binds_sweep_expired_graces_and_count_held_ones` (`crates/crpg-server/tests/host_reconnect.rs:1573`)
  - `grace_purges_only_its_own_commands` (`crates/crpg-server/tests/host_reconnect.rs:1606`)
  - `grace_keeps_other_peers_commands_in_order` (`crates/crpg-server/tests/host_reconnect.rs:1638`)
  - `grace_and_resume_record_the_callers_time` (`crates/crpg-server/tests/host_reconnect.rs:1682`)
  - `grace_deadline_saturates_at_the_ceiling` (`crates/crpg-server/tests/host_reconnect.rs:1713`)
  - `only_the_exact_proof_resumes` (`crates/crpg-server/tests/host_reconnect.rs:1733`)
  - `live_v3_resume_keeps_its_views` (`crates/crpg-server/tests/host_reconnect.rs:1779`)
  - `live_resume_retires_old_ordinals` (`crates/crpg-server/tests/host_reconnect.rs:1847`)
  - `v1_v4_sessions_take_a_ticket_and_a_grace` (`crates/crpg-server/tests/host_reconnect_v4.rs:576`)
  - `v2_unanswered_moves_are_unadmitted` (`crates/crpg-server/tests/host_reconnect_v4.rs:697`)
  - `v3_released_receipts_are_dropped_and_served_from_the_cache` (`crates/crpg-server/tests/host_reconnect_v4.rs:766`)
  - `v4_a_live_v4_resume_restages_and_reanchors` (`crates/crpg-server/tests/host_reconnect_v4.rs:822`)
  - `v5_the_world_moves_while_a_v4_session_is_graced` (`crates/crpg-server/tests/host_reconnect_v4.rs:889`)
  - `v6_v4_graces_expire_and_never_reach_a_checkpoint` (`crates/crpg-server/tests/host_reconnect_v4.rs:917`)
  - `v7_the_move_bucket_is_unchanged_by_grace_and_resume` (`crates/crpg-server/tests/host_reconnect_v4.rs:977`)
  - `v8_the_withheld_set_is_unchanged_by_grace_and_resume` (`crates/crpg-server/tests/host_reconnect_v4.rs:1039`)
  - `ticket_follows_the_welcome` (`crates/crpg-server/tests/host_quic_resume.rs:480`)
  - `silent_loss_then_resume` (`crates/crpg-server/tests/host_quic_resume.rs:570`)
  - `live_resume_fences_the_old_connection` (`crates/crpg-server/tests/host_quic_resume.rs:749`)
  - `refusals_hang_up_with_a_code` (`crates/crpg-server/tests/host_quic_resume.rs:838`)
  - `what_ends_a_grace` (`crates/crpg-server/tests/host_quic_resume.rs:1005`)
  - `start_is_unchanged` (`crates/crpg-server/tests/host_quic_resume.rs:1091`)
  - `v1_silent_loss_never_begins_a_grace` (`crates/crpg-server/tests/host_quic_resume.rs:1200`)
  - `a_resumed_binding_graces_again_on_silent_loss` (`crates/crpg-server/tests/host_quic_resume.rs:1233`)
  - `a_fresh_hello_supersedes_a_resumed_binding` (`crates/crpg-server/tests/host_quic_resume.rs:1308`)
  - `control_change_ends_a_grace` (`crates/crpg-server/tests/host_quic_resume.rs:1368`)
  - `grants_change_ends_a_grace_before_a_hello` (`crates/crpg-server/tests/host_quic_resume.rs:1381`)
  - `a_movement_hello_gets_a_ticket` (`crates/crpg-server/tests/host_quic_move_resume.rs:705`)
  - `movement_silent_loss_then_resume` (`crates/crpg-server/tests/host_quic_move_resume.rs:769`)
  - `live_movement_resume_fences_the_old_connection` (`crates/crpg-server/tests/host_quic_move_resume.rs:960`)
  - `refused_lane1_frames_are_rejections` (`crates/crpg-server/tests/host_quic_move_resume.rs:1102`)
  - `a_held_lane1_retry_is_not_fence_checked` (`crates/crpg-server/tests/host_quic_move_resume.rs:1218`)
  - `resume_identifiers_and_caps_are_pinned` (`crates/crpg-net/tests/reconnect_wire.rs:270`)
  - `resume_literal_byte_oracles` (`crates/crpg-net/tests/reconnect_wire.rs:360`)
  - `resume_frame_sizes_at_the_bounds` (`crates/crpg-net/tests/reconnect_wire.rs:447`)
  - `request_decode_precedence` (`crates/crpg-net/tests/reconnect_wire.rs:540`)
  - `down_decode_precedence` (`crates/crpg-net/tests/reconnect_wire.rs:623`)
  - `request_field_bounds` (`crates/crpg-net/tests/reconnect_wire.rs:716`)
  - `down_field_bounds` (`crates/crpg-net/tests/reconnect_wire.rs:870`)
  - `encode_mirrors_decode` (`crates/crpg-net/tests/reconnect_wire.rs:982`)
  - `proof_is_opaque_bounded_and_redacted` (`crates/crpg-net/tests/reconnect_wire.rs:1107`)
  - `resume_and_existing_codecs_refuse_each_other` (`crates/crpg-net/tests/reconnect_wire.rs:1274`)
  - `fence_starts_at_first_and_checks_frames` (`crates/crpg-net/tests/reconnect_wire.rs:1791`)
  - `resolve_reflects_an_explicit_decision` (`crates/crpg-net/tests/reconnect_wire.rs:1834`)
  - `resolve_refusal_precedence` (`crates/crpg-net/tests/reconnect_wire.rs:1918`)
  - `fence_window_retries_lost_grants_without_reissuing` (`crates/crpg-net/tests/reconnect_wire.rs:2016`)
  - `client_check_result_and_duplicates` (`crates/crpg-net/tests/reconnect_wire.rs:2098`)
  - `rebind_keeps_floors_and_fails_closed` (`crates/crpg-net/tests/reconnect_wire.rs:2166`)
  - `resume_cutover_installs_snapshot_before_post_cutover_deltas` (`crates/crpg-net/tests/reconnect_wire.rs:2469`)
  - `retry_identity_across_resume` (`crates/crpg-net/tests/reconnect_wire.rs:2729`)
  - `fenced_connection_frames_never_reach_admission` (`crates/crpg-net/tests/reconnect_wire.rs:2846`)
  - `obsolete_transfer_never_overwrites_the_resume_snapshot` (`crates/crpg-net/tests/reconnect_wire.rs:2950`)
  - `resume_snapshot_oracle_and_corruption` (`crates/crpg-net/tests/reconnect_wire.rs:3319`)
- **CI:** `check` job, step `cargo test --workspace --locked` (`.github/workflows/ci.yml:184`) on `ubuntu-latest` and `windows-latest` (`.github/workflows/ci.yml:105`).
- **Task record:** `tasks/T025b.md:1`, `tasks/T025b.md:29`; `tasks/BACKLOG.md:117`, `tasks/BACKLOG.md:118`. `tasks/T155.md:1`; `tasks/BACKLOG.md:145`.
- **Gaps:** none found.

## 5. "Save/load equivalence holds (gate 10)."

- **Tests:**
  - `file_save_load_equivalence` (`crates/crpg-server/tests/host_save.rs:725`) — macro generates `v1`/`v2`.
  - `repeated_restarts_match_uninterrupted_run` (`crates/crpg-server/tests/host_save.rs:749`) — macro generates `v1`/`v2`.
  - `older_save_resumes_from_its_own_point` (`crates/crpg-server/tests/host_save.rs:798`) — macro generates `v1`/`v2`.
  - `resave_after_load_differs_only_in_incarnation` (`crates/crpg-server/tests/host_save.rs:904`) — macro generates `v1`/`v2`.
  - `save_refusals_leave_files_untouched` (`crates/crpg-server/tests/host_save.rs:1333`) — macro generates `v1`/`v2`.
  - `payload_layout_matches_hand_built_bytes` (`crates/crpg-server/tests/host_save.rs:805`)
  - `engine_version_is_valid_and_recorded` (`crates/crpg-server/tests/host_save.rs:828`)
  - `file_bytes_equal_encoded_envelope` (`crates/crpg-server/tests/host_save.rs:852`)
  - `identity_validation` (`crates/crpg-server/tests/host_save.rs:907`)
  - `payload_cap_boundaries` (`crates/crpg-server/tests/host_save.rs:964`)
  - `header_refusals` (`crates/crpg-server/tests/host_save.rs:997`)
  - `identity_refusals` (`crates/crpg-server/tests/host_save.rs:1098`)
  - `envelope_refusals_pass_through` (`crates/crpg-server/tests/host_save.rs:1139`)
  - `checkpoint_refusals_pass_through` (`crates/crpg-server/tests/host_save.rs:1218`)
  - `error_display_strings_are_pinned` (`crates/crpg-server/tests/host_save.rs:1336`)
- **CI:** `check` job, step `cargo test --workspace --locked` (`.github/workflows/ci.yml:184`) on `ubuntu-latest` and `windows-latest` (`.github/workflows/ci.yml:105`).
- **Task record:** `tasks/T039.md:1`, `tasks/T039.md:119`; `tasks/BACKLOG.md:132`, `tasks/BACKLOG.md:536`.
- **Gaps:** campaign-wide breadth (every fixture campaign) deferred to T040/T041 (`tasks/BACKLOG.md:536`); no cross-target load test, equivalence is per-target in one process (`tasks/BACKLOG.md:536`).

## 6. "Windows and Linux dedicated-server artifacts build (gate 12)."

- **Tests:**
  - `help_and_version` (`crates/crpg-server/tests/server_binary.rs:344`)
  - `serves_logs_and_saves_on_quit` (`crates/crpg-server/tests/server_binary.rs:627`)
  - `movement_endpoint_serves_v4_and_issues_a_ticket` (`crates/crpg-server/tests/server_binary_movement.rs:492`)
- **CI:** `server-artifacts` job (`.github/workflows/ci.yml:362`), matrix `windows-latest`/`ubuntu-latest` (`.github/workflows/ci.yml:366-371`), step `cargo build --release --locked -p crpg-server --bin crpg-server` (`.github/workflows/ci.yml:413`), step "The built binary runs" (`.github/workflows/ci.yml:415`) on both OS.
- **Task record:** `tasks/T040.md:1`, `tasks/T040.md:44`; `tasks/BACKLOG.md:137`.
- **Gaps:** none found.

## 7. "Smoke tests pass for Windows embedded, Windows dedicated and Linux dedicated (gate 13)."

- **Tests:**
  - `smoke.gd` (`apps/client/tests/smoke.gd:1`) — headless playthrough of `Main.tscn` (BR-22).
  - `dedicated_server_two_clients_fight_to_identical_state` (`crates/crpg-server/tests/dedicated_smoke.rs:598`)
  - `dedicated_server_two_clients_move_to_identical_positions` (`crates/crpg-server/tests/dedicated_smoke.rs:1238`)
- **CI:** `client` job, step "BR-22 smoke" (`.github/workflows/ci.yml:313`) on `windows-latest` (`.github/workflows/ci.yml:193`); `check` job, step `cargo test --workspace --locked` (`.github/workflows/ci.yml:184`) on `ubuntu-latest` and `windows-latest` (`.github/workflows/ci.yml:105`).
- **Task record:** `tasks/T041.md:1`, `tasks/T041.md:67`; `tasks/BACKLOG.md:143`. `tasks/T047.md:1`, `tasks/T047c.md:1`; `tasks/BACKLOG.md:271`, `tasks/BACKLOG.md:272`.
- **Gaps:** none found.

## Summary table

| # | Test count found | CI job | Gaps |
|---|---|---|---|
| 1 | 3 | `check` + `client` | no |
| 2 | 1 | `check` | no |
| 3 | 24 | `check` | yes |
| 4 | 66 | `check` | no |
| 5 | 15 | `check` | yes |
| 6 | 3 | `server-artifacts` | no |
| 7 | 3 | `client` + `check` | no |

## Agent log

- 2026-11-22 (UTC) · opencode-go/beam-free · M4 exit facts · Collected path:line evidence for the seven M4 exit criteria from the test, CI and task records.
