extends SceneTree
# E1-E4 (T156b): the default scene's enemy is played by the server's AI
# (`open_v4_ai`). One coroutine drives everything; every session state is read
# only after `await process_frame`, so never mid-`_process`. A runtime error in
# the coroutine would end it silently, so success is only ever printed from its
# last line and the overall timeout (a `_process` of this script) catches the
# rest.
# Run: Godot4 --headless --path apps/client -s res://tests/enemy_ai.gd
# Exit 0 pass; 2 class missing; 3 open_code/stop_code non-zero or a call
# returned an unexpected code; 4 timeout (20 s overall); 6 a value check failed.
# Control: CRPG_CAMPAIGN set to a missing directory -> E3's open fails with 20,
# exit 3.
# No GDScript-visible fact tells `open_v4_ai` from `open_v4` (both play the same
# fight): E2 reads the one call in Main.gd, the core's A7 test proves the wrapper.

const TIMEOUT_MS := 20000
const D := 600                 # AI_STEP_DELAY_MS (D78b); copied, not read
const MAIN_GD := "res://scripts/Main.gd"
const MAIN_TSCN := "res://scenes/Main.tscn"

var _main: Node = null
var _started_ms: int = 0
var _done: bool = false
var _reached: String = "none"
var _logged_rev: int = 0
var _log: Array = []           # {"t": arrival ms, "e": event}, oldest first
var _goblin: int = 0           # member state: a lambda captures locals by value
var _hero: int = 0

func _initialize() -> void:
	if not ClassDB.class_exists("CrpgSession"):
		printerr("enemy_ai.gd: class CrpgSession is missing")
		quit(2)
		return
	_started_ms = Time.get_ticks_msec()
	_run()

func _process(_delta: float) -> bool:
	if not _done and Time.get_ticks_msec() - _started_ms >= TIMEOUT_MS:
		_quit(4, "not finished within 20 s (last case reached: %s)" % _reached)
	return false

# ---- failure and check helpers (each quits once and marks the run done) ----

func _quit(code: int, message: String) -> void:
	if _done:
		return
	_done = true
	printerr("enemy_ai.gd FAIL: ", message)
	quit(code)

func _want(label: String, ok: bool, detail: String) -> void:
	if not ok:
		_quit(6, "%s: %s" % [label, detail])

# ---- frames, waiting and the event log ----

# Logs the events of a revision not yet logged, stamped with the arrival time.
func _observe() -> void:
	if _main == null or _main.view == null:
		return
	var rev: int = _main.view.revision()
	if rev != _logged_rev:
		_logged_rev = rev
		var now: int = Time.get_ticks_msec()
		for event in _main.view.events():
			_log.append({"t": now, "e": event})

# One frame: a short sleep, the frame, then the codes check and the log.
func _frame(label: String) -> bool:
	if _done:
		return false
	if Time.get_ticks_msec() - _started_ms >= TIMEOUT_MS:
		_quit(4, "%s: not reached within 20 s" % label)
		return false
	OS.delay_msec(5)
	await process_frame
	if _main != null and (_main.open_code != 0 or _main.stop_code != 0):
		_quit(3, "%s: open_code=%d stop_code=%d" % [label, _main.open_code, _main.stop_code])
		return false
	_observe()
	return not _done

# Waits (one frame at a time) until `cond` is true; false after quitting.
func _wait(label: String, cond: Callable) -> bool:
	while not _done:
		if not await _frame(label):
			return false
		if _main.view != null and cond.call():
			return true
	return false

# The network id of the row named `who`, or 0.
func _id_of(view, who: String) -> int:
	var i: int = view.names().find(who)
	if i < 0:
		return 0
	return view.net_ids()[i]

# The first logged action_resolved by `actor`, or an empty dictionary.
func _first_action(actor: int) -> Dictionary:
	for entry in _log:
		var e: Dictionary = entry["e"]
		if e.get("kind", "") == "action_resolved" and e.get("actor", 0) == actor:
			return entry
	return {}

func _count_actions(actor: int) -> int:
	var n := 0
	for entry in _log:
		var e: Dictionary = entry["e"]
		if e.get("kind", "") == "action_resolved" and e.get("actor", 0) == actor:
			n += 1
	return n

# ---- the cases ----

func _run() -> void:
	_e1()
	if _done:
		return
	_e2()
	if _done:
		return
	if not await _e3():
		return
	if not await _e4():
		return
	_want("every case reached its last line", _reached == "E4", _reached)
	if _done:
		return
	print("enemy_ai.gd OK")
	_done = true
	quit(0)

func _e1() -> void:
	_want("E1 class exists", ClassDB.class_exists("CrpgSession"), "missing")
	_want("E1 open_v4_ai is a method", ClassDB.class_has_method("CrpgSession", "open_v4_ai"), "no such method")
	_reached = "E1"

func _e2() -> void:
	var kept := PackedStringArray()
	for line in FileAccess.get_file_as_string(MAIN_GD).split("\n"):
		var cut: int = line.find("#")
		kept.append(line if cut < 0 else line.substr(0, cut))
	var source := "\n".join(kept)
	var opens := RegEx.new()
	opens.compile(r"session\.(open[a-z0-9_]*)\(")
	var found := opens.search_all(source)
	_want("E2 exactly one open call", found.size() == 1, str(found.size()))
	if _done:
		return
	_want("E2 the call is open_v4_ai", found[0].get_string(1) == "open_v4_ai", found[0].get_string(1))
	# Control: the same scan does see the other session call.
	var polls := RegEx.new()
	polls.compile(r"session\.poll\(")
	_want("E2 control finds session.poll(", polls.search(source) != null, "not found")
	_reached = "E2"

func _e3() -> bool:
	var packed := load(MAIN_TSCN) as PackedScene
	if packed == null:
		_quit(6, "E3 %s did not load" % MAIN_TSCN)
		return false
	_main = packed.instantiate()
	root.add_child(_main)
	var t0: int = Time.get_ticks_msec()
	# The Goblin (the opening actor) attacks, about D after the open.
	var attacked := func() -> bool:
		_goblin = _id_of(_main.view, "Goblin")
		_hero = _id_of(_main.view, "Hero")
		return _goblin != 0 and not _first_action(_goblin).is_empty()
	if not await _wait("E3 the Goblin's action_resolved", attacked):
		return false
	var first: Dictionary = _first_action(_goblin)
	var waited: int = first["t"] - t0
	_want("E3 the Goblin waited about D", waited >= D - 100, "%d ms after open, want >= %d" % [waited, D - 100])
	_want("E3 the Goblin attacked the Hero", first["e"].get("target", 0) == _hero and _hero != 0, str(first["e"]))
	_want("E3 the attack is a Strike", first["e"].get("ability", "") == "Strike", str(first["e"]))
	if _done:
		return false
	# Contract erratum: the Goblin's Strike ends its turn, so the attack and the
	# Hero's turn arrive together (one AI step), not one delay apart.
	var hero_turn := func() -> bool:
		return _main.view.active() == _hero and _main.view.option_labels().size() > 0
	if not await _wait("E3 the Hero's options", hero_turn):
		return false
	var gap: int = Time.get_ticks_msec() - first["t"]
	_want("E3 the Hero's turn came with the attack, not a delay later", gap < D - 100, "%d ms after the attack" % gap)
	print("enemy_ai.gd: the Goblin acted %d ms after open, the Hero's options came %d ms later" % [waited, gap])
	_want("E3 the Goblin acted exactly once", _count_actions(_goblin) == 1, str(_count_actions(_goblin)))
	_want("E3 the Hero holds the options", _main.view.active() == _hero, str(_main.view.active()))
	if _done:
		return false
	# Close the scene's session before E4 opens its own.
	root.remove_child(_main)
	_main.free()
	_main = null
	_reached = "E3"
	return true

func _e4() -> bool:
	var s = ClassDB.instantiate("CrpgSession")
	_want("E4 instantiated", s != null, "null")
	if _done:
		return false
	var dir: String = ProjectSettings.globalize_path("res://").path_join("../../campaigns/fixtures/combat_basic").simplify_path()
	var bad: int = s.open_v4_ai("/no/such/dir", "", "", 1)
	_want("E4 missing campaign is 20", bad == 20, "%d (%s)" % [bad, s.last_error()])
	var neg: int = s.open_v4_ai(dir, "", "", -1)
	_want("E4 negative seed is 3", neg == 3, "%d (%s)" % [neg, s.last_error()])
	var nobody: int = s.open_v4_ai(dir, "", "Nobody", 1)
	_want("E4 unknown side is 24", nobody == 24, "%d (%s)" % [nobody, s.last_error()])
	if _done:
		return false
	var ok: int = s.open_v4_ai(dir, "", "Goblin", 1)
	_want("E4 open as Goblin is 0", ok == 0, "%d (%s)" % [ok, s.last_error()])
	if _done:
		return false
	# Poll until the player (the Goblin, who opens) has options.
	var until: int = Time.get_ticks_msec() + 3000
	while not _done and s.view().option_labels().size() == 0:
		if Time.get_ticks_msec() >= until:
			_quit(6, "E4 no options within 3 s (%s)" % s.last_error())
			return false
		OS.delay_msec(5)
		await process_frame
		var code: int = s.poll()
		if code != 0:
			_quit(3, "E4 poll = %d (%s)" % [code, s.last_error()])
			return false
	if _done:
		return false
	var view = s.view()
	var row: int = view.net_ids().find(view.active())
	_want("E4 the active row exists", row >= 0, str(view.active()))
	if _done:
		return false
	_want("E4 the active row is the Goblin", view.names()[row] == "Goblin", view.names()[row])
	var again: int = s.open_v4_ai(dir, "", "", 1)
	_want("E4 a second open is 2", again == 2, "%d (%s)" % [again, s.last_error()])
	s.close()
	_reached = "E4"
	return not _done
