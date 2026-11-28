extends SceneTree
# E1-E4 (T157f): the shell's four new functions, `CrpgSession.open_v4_walk` and
# `request_mode`, `CrpgView.movement` and `movement_max`, against the two-hero
# adventure campaign (n(1) Hero, n(2) Hero, n(3) Goblin; movement pool of 6).
# One coroutine drives everything; every session state is read only after
# `await process_frame`. A runtime error in the coroutine would end it silently,
# so success is only ever printed from its last line, after a check that every
# case reached its own last line, and the overall timeout (a `_process` of this
# script) catches the rest.
# Run: Godot4 --headless --path apps/client -s res://tests/adventure_session.gd
# Exit 0 pass; 2 class missing; 3 a call returned an unexpected code; 4 timeout
# (30 s overall); 6 a value check failed.
# The environment is not read: the campaign paths are fixed below.

const TIMEOUT_MS := 30000

var _started_ms: int = 0
var _done: bool = false
var _reached: String = "none"
var _dir: String = ""
var _basic: String = ""

func _initialize() -> void:
	if not ClassDB.class_exists("CrpgSession"):
		printerr("adventure_session.gd: class CrpgSession is missing")
		quit(2)
		return
	var res: String = ProjectSettings.globalize_path("res://")
	_dir = res.path_join("../../campaigns/demo/adventure").simplify_path()
	_basic = res.path_join("../../campaigns/fixtures/combat_basic").simplify_path()
	_started_ms = Time.get_ticks_msec()
	_run()

func _process(_delta: float) -> bool:
	if not _done and Time.get_ticks_msec() - _started_ms >= TIMEOUT_MS:
		_quit(4, "not finished within 30 s (last case reached: %s)" % _reached)
	return false

# ---- failure and check helpers (each quits once and marks the run done) ----

func _quit(code: int, message: String) -> void:
	if _done:
		return
	_done = true
	printerr("adventure_session.gd FAIL: ", message)
	quit(code)

func _want(label: String, ok: bool, detail: String) -> void:
	if not ok:
		_quit(6, "%s: %s" % [label, detail])

# A call that must return `want`; any other code is exit 3.
func _want_code(label: String, got: int, want: int, s) -> void:
	if got != want:
		_quit(3, "%s = %d, want %d (%s)" % [label, got, want, s.last_error()])

# Polls `s` one frame at a time until `cond` holds on its view; false after
# quitting (a poll code other than 0 is exit 3, no luck within `ms` is exit 4).
func _poll_until(label: String, s, ms: int, cond: Callable) -> bool:
	var until: int = Time.get_ticks_msec() + ms
	while not _done:
		if Time.get_ticks_msec() >= until:
			_quit(4, "%s: not reached within %d ms (%s)" % [label, ms, s.last_error()])
			return false
		OS.delay_msec(5)
		await process_frame
		var code: int = s.poll()
		if code != 0:
			_quit(3, "%s: poll = %d (%s)" % [label, code, s.last_error()])
			return false
		if cond.call(s.view()):
			return true
	return false

# ---- the cases ----

func _run() -> void:
	_e1()
	if _done:
		return
	if not await _e2():
		return
	if not await _e3():
		return
	if not await _e4():
		return
	_want("every case reached its last line", _reached == "E4", _reached)
	if _done:
		return
	print("adventure_session.gd OK")
	_done = true
	quit(0)

func _e1() -> void:
	_want("E1 class exists", ClassDB.class_exists("CrpgSession"), "missing")
	_want("E1 open_v4_walk is a method", ClassDB.class_has_method("CrpgSession", "open_v4_walk"), "no such method")
	_want("E1 request_mode is a method", ClassDB.class_has_method("CrpgSession", "request_mode"), "no such method")
	_want("E1 CrpgView.movement is a method", ClassDB.class_has_method("CrpgView", "movement"), "no such method")
	_want("E1 CrpgView.movement_max is a method", ClassDB.class_has_method("CrpgView", "movement_max"), "no such method")
	_reached = "E1"

# The four failing opens, then the one that works and its repeat.
func _e2() -> bool:
	var s = ClassDB.instantiate("CrpgSession")
	_want("E2 instantiated", s != null, "null")
	if _done:
		return false
	_want_code("E2 missing campaign", s.open_v4_walk("/no/such/dir", "", "", 1), 20, s)
	_want_code("E2 negative seed", s.open_v4_walk(_dir, "", "", -1), 3, s)
	_want_code("E2 unknown side", s.open_v4_walk(_dir, "", "Nobody", 1), 24, s)
	_want_code("E2 combat_basic has no round_ticks", s.open_v4_walk(_basic, "", "", 1), 25, s)
	if _done:
		return false
	_want_code("E2 open", s.open_v4_walk(_dir, "", "", 1), 0, s)
	_want_code("E2 second open", s.open_v4_walk(_dir, "", "", 1), 2, s)
	if _done:
		return false
	s.close()
	_reached = "E2"
	return true

# The walk session: the two heroes walk (no fight), the readout, the mode
# request, the AI's turn and the player's turn.
func _e3() -> bool:
	var s = ClassDB.instantiate("CrpgSession")
	_want("E3 instantiated", s != null, "null")
	if _done:
		return false
	_want_code("E3 open", s.open_v4_walk(_dir, "", "", 1), 0, s)
	if _done:
		return false
	var first_view := func(v) -> bool:
		return v.revision() >= 1
	if not await _poll_until("E3 the first revision", s, 3000, first_view):
		return false
	var v = s.view()
	_want("E3 no turn head while walking", v.active() == 0, str(v.active()))
	_want("E3 names", Array(v.names()) == ["Hero", "Hero", "Goblin"], str(v.names()))
	_want("E3 controlled", Array(s.controlled()) == [1, 2], str(s.controlled()))
	_want("E3 movement", Array(v.movement()) == [6, 6, -1], str(v.movement()))
	_want("E3 movement_max", Array(v.movement_max()) == [6, 6, -1], str(v.movement_max()))
	if _done:
		return false
	_want_code("E3 request_mode(2)", s.request_mode(2), 3, s)
	_want_code("E3 request_mode(0)", s.request_mode(0), 0, s)
	if _done:
		return false
	var goblin_turn := func(view) -> bool:
		return view.active() == 3
	if not await _poll_until("E3 the Goblin's turn", s, 5000, goblin_turn):
		return false
	var hero_turn := func(view) -> bool:
		return view.active() == 1 and Array(view.option_labels()) == ["End turn"]
	if not await _poll_until("E3 hero 1's turn", s, 5000, hero_turn):
		return false
	var turn = s.view()
	_want("E3 movement at the hero's turn", turn.movement()[0] == 6, str(turn.movement()))
	if _done:
		return false
	s.close()
	_want_code("E3 request_mode after close", s.request_mode(0), 1, s)
	if _done:
		return false
	_reached = "E3"
	return true

# Control: the plain real-time open grants neither pause right nor pool facts.
func _e4() -> bool:
	var u = ClassDB.instantiate("CrpgSession")
	_want("E4 instantiated", u != null, "null")
	if _done:
		return false
	_want_code("E4 open_v4_rt", u.open_v4_rt(_dir, "", "", 1), 0, u)
	if _done:
		return false
	var first_view := func(v) -> bool:
		return v.revision() >= 1
	if not await _poll_until("E4 the first revision", u, 3000, first_view):
		return false
	_want("E4 movement is undisclosed", Array(u.view().movement()) == [-1, -1, -1], str(u.view().movement()))
	if _done:
		return false
	_want_code("E4 request_mode(0)", u.request_mode(0), 0, u)
	if _done:
		return false
	var refused := false
	var until: int = Time.get_ticks_msec() + 2000
	while not _done and Time.get_ticks_msec() < until:
		OS.delay_msec(5)
		await process_frame
		var code: int = u.poll()
		if code != 0:
			_quit(3, "E4 poll = %d (%s)" % [code, u.last_error()])
			return false
		var view = u.view()
		_want("E4 no turn head in any frame", view.active() == 0, str(view.active()))
		for receipt in view.receipts():
			if receipt.get("applied", true) == false and receipt.get("code", 0) == 17:
				refused = true
	_want("E4 the request was refused 17", refused, "no {applied: false, code: 17} receipt in 2 s")
	if _done:
		return false
	u.close()
	_reached = "E4"
	return true
