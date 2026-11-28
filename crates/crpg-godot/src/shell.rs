//! The thin gdext shell (ADR-0033 §10): converts GDScript values, makes exactly
//! one [`BridgeCore`] call per `#[func]`, and converts the answer back.
//!
//! No rule, no panic policy and no indexing lives here; those belong to the
//! Godot-free core. The module is private so macro-generated items cannot trip
//! `missing_docs`. It imports items from `crate::core`, never the bare name
//! `core`, because that module shadows the `core` crate at the crate root.

use std::rc::Rc;
use std::time::Instant;

use godot::prelude::*;

use crate::core::{BridgeCore, SessionView, ViewValue};

/// Entry-point type for the `gdext_rust_init` symbol.
struct CrpgExtension;

// SAFETY: implementing `ExtensionLibrary` opts this library into godot-rust's
// safety model (godot-core-0.5.5/src/init/mod.rs:557). We uphold it: Godot
// 4.7.2 loads the library once (api-4-7); it registers only CrpgSession and
// CrpgView, which use gdext's checked conversions and no raw pointer;
// `experimental-threads` is off, so gdext asserts main-thread access; and this
// is the only hand-written `unsafe` in the crate.
#[gdextension]
unsafe impl ExtensionLibrary for CrpgExtension {}

/// One embedded single-player session, as GDScript sees it.
#[derive(GodotClass)]
#[class(base = RefCounted, init)]
struct CrpgSession {
    core: BridgeCore,
    opened_at: Option<Instant>,
}

#[godot_api]
impl CrpgSession {
    /// Opens a session; 0 on success, otherwise a bridge code.
    #[func]
    fn open(
        &mut self,
        campaign_dir: GString,
        encounter_id: GString,
        player_side: GString,
        seed: i64,
    ) -> i64 {
        let code = self.core.open(
            &campaign_dir.to_string(),
            &encounter_id.to_string(),
            &player_side.to_string(),
            seed,
        );
        let code = code.as_i64();
        if code == 0 {
            self.opened_at = Some(Instant::now());
        }
        code
    }

    /// Opens a protocol v4 session on the server run loop; 0 on success,
    /// otherwise a bridge code. `opened_at` is set exactly as `open` sets it.
    #[func]
    fn open_v4(
        &mut self,
        campaign_dir: GString,
        encounter_id: GString,
        player_side: GString,
        seed: i64,
    ) -> i64 {
        let code = self.core.open_v4(
            &campaign_dir.to_string(),
            &encounter_id.to_string(),
            &player_side.to_string(),
            seed,
        );
        let code = code.as_i64();
        if code == 0 {
            self.opened_at = Some(Instant::now());
        }
        code
    }

    /// Opens a real-time protocol v4 session; 0 on success, otherwise a
    /// bridge code. `opened_at` is set exactly as `open` sets it.
    #[func]
    fn open_v4_rt(
        &mut self,
        campaign_dir: GString,
        encounter_id: GString,
        player_side: GString,
        seed: i64,
    ) -> i64 {
        let code = self.core.open_v4_rt(
            &campaign_dir.to_string(),
            &encounter_id.to_string(),
            &player_side.to_string(),
            seed,
        );
        let code = code.as_i64();
        if code == 0 {
            self.opened_at = Some(Instant::now());
        }
        code
    }

    /// Opens a turn-based protocol v4 session whose non-player side the
    /// server's AI plays; 0 on success, otherwise a bridge code. `opened_at`
    /// is set exactly as `open` sets it.
    #[func]
    fn open_v4_ai(
        &mut self,
        campaign_dir: GString,
        encounter_id: GString,
        player_side: GString,
        seed: i64,
    ) -> i64 {
        let code = self.core.open_v4_ai(
            &campaign_dir.to_string(),
            &encounter_id.to_string(),
            &player_side.to_string(),
            seed,
        );
        let code = code.as_i64();
        if code == 0 {
            self.opened_at = Some(Instant::now());
        }
        code
    }

    /// The replica ids this session may move, ascending.
    #[func]
    fn controlled(&self) -> PackedInt64Array {
        PackedInt64Array::from(self.core.controlled().as_slice())
    }

    /// Three raw `Fx16_16` integers per row of `view()`: where to draw it now.
    #[func]
    fn render_positions(&self) -> PackedInt64Array {
        PackedInt64Array::from(self.core.render_positions().as_slice())
    }

    /// Turn-based move of the current options' actor to a raw `Fx16_16` point.
    #[func]
    fn declare_move(&mut self, revision: i64, x: i64, y: i64, z: i64) -> i64 {
        self.core.declare_move(revision, x, y, z).as_i64()
    }

    /// Real-time move of the entity with network id `actor` (lane 1).
    #[func]
    fn move_to(&mut self, actor: i64, x: i64, y: i64, z: i64) -> i64 {
        self.core.move_to(actor, x, y, z).as_i64()
    }

    /// Real-time halt of the entity with network id `actor` (lane 1).
    #[func]
    fn halt(&mut self, actor: i64) -> i64 {
        self.core.halt(actor).as_i64()
    }

    /// Advances the session by the wall-clock time since `open`.
    #[func]
    fn poll(&mut self) -> i64 {
        let now_ms = self.opened_at.map_or(0, |at| {
            u64::try_from(at.elapsed().as_millis()).unwrap_or(u64::MAX)
        });
        self.core.poll(now_ms).as_i64()
    }

    /// A new immutable snapshot of the current view.
    #[func]
    fn view(&self) -> Gd<CrpgView> {
        Gd::from_object(CrpgView {
            view: self.core.view(),
        })
    }

    /// Submits the option at `index` of the view with `revision`.
    #[func]
    fn submit_option(&mut self, revision: i64, index: i64) -> i64 {
        self.core.submit_option(revision, index).as_i64()
    }

    /// Closes the session; idempotent.
    #[func]
    fn close(&mut self) {
        self.core.close();
        self.opened_at = None;
    }

    /// The last error text, `NAME` or `NAME: detail`.
    #[func]
    fn last_error(&self) -> GString {
        GString::from(self.core.last_error())
    }

    /// Opens a real-time walk session (server AI for turn-based turns); 0 on
    /// success, otherwise a bridge code. `opened_at` is set exactly as `open`
    /// sets it.
    #[func]
    fn open_v4_walk(
        &mut self,
        campaign_dir: GString,
        encounter_id: GString,
        player_side: GString,
        seed: i64,
    ) -> i64 {
        let code = self.core.open_v4_walk(
            &campaign_dir.to_string(),
            &encounter_id.to_string(),
            &player_side.to_string(),
            seed,
        );
        let code = code.as_i64();
        if code == 0 {
            self.opened_at = Some(Instant::now());
        }
        code
    }

    /// Asks the host to switch the encounter mode (0 turn-based, 1 real
    /// time); 0 when the request was sent, otherwise a bridge code.
    #[func]
    fn request_mode(&mut self, mode: i64) -> i64 {
        self.core.request_mode(mode).as_i64()
    }
}

/// An immutable snapshot of one view revision.
#[derive(GodotClass)]
#[class(base = RefCounted, no_init)]
struct CrpgView {
    view: Rc<SessionView>,
}

#[godot_api]
impl CrpgView {
    /// The revision number; 0 is the empty view.
    #[func]
    fn revision(&self) -> i64 {
        self.view.revision()
    }

    /// Network ids, ascending.
    #[func]
    fn net_ids(&self) -> PackedInt64Array {
        PackedInt64Array::from(self.view.net_ids())
    }

    /// Display names, parallel to `net_ids`.
    #[func]
    fn names(&self) -> PackedStringArray {
        strings(self.view.names())
    }

    /// Side names, parallel to `net_ids`.
    #[func]
    fn sides(&self) -> PackedStringArray {
        strings(self.view.sides())
    }

    /// Placement ULID text, parallel to `net_ids`.
    #[func]
    fn placements(&self) -> PackedStringArray {
        strings(self.view.placements())
    }

    /// Current health, parallel to `net_ids`.
    #[func]
    fn health(&self) -> PackedInt64Array {
        PackedInt64Array::from(self.view.health())
    }

    /// Maximum health, parallel to `net_ids`.
    #[func]
    fn max_health(&self) -> PackedInt64Array {
        PackedInt64Array::from(self.view.max_health())
    }

    /// 1 where health is known, else 0.
    #[func]
    fn health_known(&self) -> PackedByteArray {
        PackedByteArray::from(self.view.health_known())
    }

    /// 1 where the entity is dead, else 0.
    #[func]
    fn dead(&self) -> PackedByteArray {
        PackedByteArray::from(self.view.dead())
    }

    /// The active entity's network id, or 0.
    #[func]
    fn active(&self) -> i64 {
        self.view.active()
    }

    /// The turn round.
    #[func]
    fn round(&self) -> i64 {
        self.view.round()
    }

    /// Whether the encounter has ended.
    #[func]
    fn ended(&self) -> bool {
        self.view.ended()
    }

    /// The player's option labels, in host order.
    #[func]
    fn option_labels(&self) -> PackedStringArray {
        strings(self.view.option_labels())
    }

    /// The target id of each option, 0 for end turn.
    #[func]
    fn option_targets(&self) -> PackedInt64Array {
        PackedInt64Array::from(self.view.option_targets())
    }

    /// Raw `Fx16_16` positions, three integers per row, parallel to `net_ids`.
    #[func]
    fn positions(&self) -> PackedInt64Array {
        PackedInt64Array::from(self.view.positions())
    }

    /// 1 where the row's position is known, else 0.
    #[func]
    fn position_known(&self) -> PackedByteArray {
        PackedByteArray::from(self.view.position_known())
    }

    /// 1 where the row is moving, else 0.
    #[func]
    fn moving(&self) -> PackedByteArray {
        PackedByteArray::from(self.view.moving())
    }

    /// This revision's move receipts, oldest first.
    #[func]
    fn move_receipts(&self) -> Array<VarDictionary> {
        self.view
            .move_receipts()
            .iter()
            .map(|receipt| dictionary(receipt.entries()))
            .collect()
    }

    /// This revision's events, oldest first, each a dictionary with a `kind`.
    #[func]
    fn events(&self) -> Array<VarDictionary> {
        self.view
            .events()
            .iter()
            .map(|event| dictionary(event.entries()))
            .collect()
    }

    /// This revision's receipts, oldest first.
    #[func]
    fn receipts(&self) -> Array<VarDictionary> {
        self.view
            .receipts()
            .iter()
            .map(|receipt| dictionary(receipt.entries()))
            .collect()
    }

    /// Movement-pool points left per row, parallel to `net_ids`, -1 when
    /// undisclosed.
    #[func]
    fn movement(&self) -> PackedInt64Array {
        PackedInt64Array::from(self.view.movement())
    }

    /// Movement-pool maximum per row, parallel to `net_ids`, -1 when
    /// undisclosed.
    #[func]
    fn movement_max(&self) -> PackedInt64Array {
        PackedInt64Array::from(self.view.movement_max())
    }
}

/// A packed string array copy of `items`.
fn strings(items: &[String]) -> PackedStringArray {
    items.iter().map(GString::from).collect()
}

/// A dictionary built from one `entries()` list.
fn dictionary(entries: Vec<(&'static str, ViewValue<'_>)>) -> VarDictionary {
    let mut out = VarDictionary::new();
    for (key, value) in entries {
        let value = match value {
            ViewValue::Int(number) => number.to_variant(),
            ViewValue::Text(text) => GString::from(text).to_variant(),
            ViewValue::Bool(flag) => flag.to_variant(),
            ViewValue::Ints(values) => PackedInt64Array::from(values).to_variant(),
        };
        out.set(key, &value);
    }
    out
}
