extends Node3D
# T047d: owns the session, polls it once per frame, and feeds both the proxy
# view and the combat HUD. No rule is decided here (ADR-0033 §14).

const OPEN_SEED := 1

var session = null
var seen: int = 0
var view = null
var open_code: int = 0
var stop_code: int = 0

func _ready() -> void:
	session = ClassDB.instantiate("CrpgSession")
	if session == null:
		open_code = 2
		$Hud.show_error("CrpgSession is not available")
		return
	var code: int = session.open(_campaign_dir(), "", "", OPEN_SEED)
	if code != 0:
		open_code = code
		$Hud.show_error(session.last_error())
		return
	view = session.view()

func _process(_delta: float) -> void:
	if session == null or open_code != 0 or stop_code != 0:
		return
	var code: int = session.poll()
	if code == 30:
		# A refused intent is a notice, not a fence: show it and keep going.
		$Hud.show_notice(session.last_error())
	elif code != 0:
		# Only close ends a fence, so there is nothing left to poll.
		stop_code = code
		$Hud.show_error(session.last_error())
		return
	var next = session.view()
	view = next
	if next.revision() != seen:
		seen = next.revision()
		$Proxies.sync(next)
		$Hud.sync(next)

func choose(index: int) -> int:
	if session == null:
		return 1
	return session.submit_option(seen, index)

func _exit_tree() -> void:
	if session != null:
		session.close()
		session = null

func _campaign_dir() -> String:
	var from_env := OS.get_environment("CRPG_CAMPAIGN")
	if from_env != "":
		return from_env
	return ProjectSettings.globalize_path("res://").path_join("../../campaigns/fixtures/combat_basic").simplify_path()
