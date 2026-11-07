extends SceneTree
# E2 (T047d): a headless playthrough of Main.tscn. Every frame the view has
# options and its revision has not been chosen yet, it chooses option 0; 4 and
# 5 retry. It runs the scene-sync cases first and quits 6 if one fails; it
# quits 0 on the ended view when the proxy count matches the two net ids, 5 on
# a proxy-count mismatch, 3 on a failed open/poll/choose, 2 when the class is
# missing, and 4 after 120 s.
# Run: Godot4 --headless --path apps/client -s res://tests/smoke.gd
# Control: CRPG_CAMPAIGN set to a missing directory -> open = 20 -> exit 3.

const TIMEOUT_MS := 120000

var _main: Node = null
var _chosen_revision: int = -1
var _started_ms: int = 0

func _initialize() -> void:
	if not ClassDB.class_exists("CrpgSession"):
		printerr("smoke.gd: class CrpgSession is missing")
		quit(2)
		return
	var cases_script: Script = load("res://tests/scene_sync_cases.gd")
	var cases = cases_script.new() if cases_script != null and cases_script.can_instantiate() else null
	var detail = cases.run(root) if cases != null and cases.has_method("run") else null
	if not (detail is String) or detail != "PASS":
		printerr("smoke.gd FAIL: scene sync %s" % str(detail))
		quit(6)
		return
	var packed := load("res://scenes/Main.tscn") as PackedScene
	if packed == null:
		printerr("smoke.gd FAIL: Main.tscn did not load")
		quit(3)
		return
	_main = packed.instantiate()
	if _main == null:
		printerr("smoke.gd FAIL: Main.tscn did not instantiate")
		quit(3)
		return
	root.add_child(_main)
	_started_ms = Time.get_ticks_msec()

func _process(_delta: float) -> bool:
	if _main == null:
		return false
	var open_code: int = _main.open_code
	if open_code != 0:
		printerr("smoke.gd FAIL: open = %d (%s)" % [open_code, _last_error()])
		quit(3)
		return false
	var stop_code: int = _main.stop_code
	if stop_code != 0:
		printerr("smoke.gd FAIL: poll = %d (%s)" % [stop_code, _last_error()])
		quit(3)
		return false
	var view = _main.view
	if view != null:
		if view.ended():
			var count: int = _main.get_node("Proxies").proxy_count()
			var ids = view.net_ids()
			if count == ids.size() and ids.size() == 2:
				print("smoke.gd OK")
				quit(0)
			else:
				printerr("smoke.gd FAIL: proxy_count = %d, net_ids = %d" % [count, ids.size()])
				quit(5)
			return false
		if view.option_labels().size() > 0 and view.revision() != _chosen_revision:
			var code: int = _main.choose(0)
			if code == 0:
				_chosen_revision = view.revision()
			elif code != 4 and code != 5:
				printerr("smoke.gd FAIL: choose = %d (%s)" % [code, _last_error()])
				quit(3)
				return false
	if Time.get_ticks_msec() - _started_ms >= TIMEOUT_MS:
		printerr("smoke.gd FAIL: the encounter did not end within 120 s")
		quit(4)
	return false

func _last_error() -> String:
	if _main == null or _main.session == null:
		return ""
	return _main.session.last_error()
