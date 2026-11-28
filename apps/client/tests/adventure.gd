extends SceneTree
# AD1-AD10 (T157g): Adventure.tscn / Adventure.gd against the real walk-then-fight
# session over campaigns/demo/adventure (n(1) Hero (3,0,5), n(2) Hero (3,0,7),
# n(3) Goblin (12,0,6)). One coroutine drives the scene; every state is read only
# after `await process_frame`, so never mid-`_process`. A runtime error in the
# coroutine would end it silently, so success is only ever printed from its last
# line, after a check that every case reached its own last line.
# Run: Godot4 --headless --path apps/client -s res://tests/adventure.gd
# Exit 0 pass; 2 class missing; 3 open_code/stop_code non-zero or a call returned an
# unexpected code; 4 timeout (120 s overall); 6 a value check failed.
# Control: CRPG_CAMPAIGN set to a missing directory -> AD3's open fails with 20,
# exit 3, after AD1 and AD2 pass.

const TIMEOUT_MS := 120000
const ADVENTURE := "res://scenes/Adventure.tscn"
const ADVENTURE_GD := "res://scripts/Adventure.gd"
const ENGAGE_LINE := "You engage Goblin: the fight starts at the next round."
const REJECT_LINE := "Rejected: no longer legal (code 18)"
const UP := Vector3(0, 0.6, 0)
# The contract says -1.5; at that offset the click ray passes 0.51 m from hero 1's
# own centre (Pick's radius is 0.7), so it selects the hero instead of moving.
const STEP_Z := -2.0

var adv: Node = null
var cam: Camera3D = null
var _started_ms: int = 0
var _done: bool = false
var _reached: String = "none"
var _logged_rev: int = 0
var _revs: Array = []        # per observed revision: {"events", "receipts", "moves"}
var _targets_seen: int = -1

func _initialize() -> void:
	if not ClassDB.class_exists("CrpgSession"):
		printerr("adventure.gd: class CrpgSession is missing")
		quit(2)
		return
	if not _spawn():
		return
	_started_ms = Time.get_ticks_msec()
	_run()

func _process(_delta: float) -> bool:
	if not _done and _started_ms != 0 and Time.get_ticks_msec() - _started_ms >= TIMEOUT_MS:
		_quit(4, "not finished within 120 s (last case reached: %s)" % _reached)
	return false

# ---- failure and check helpers (each quits once and marks the run done) ----

func _quit(code: int, message: String) -> void:
	if _done:
		return
	_done = true
	printerr("adventure.gd FAIL: ", message)
	quit(code)

func _want(label: String, ok: bool, detail: String) -> void:
	if not ok:
		_quit(6, "%s: %s" % [label, detail])

# Loads, instantiates and adds a fresh scene as `adv`.
func _spawn() -> bool:
	var scene: PackedScene = load(ADVENTURE)
	if scene == null:
		_quit(6, "%s did not load" % ADVENTURE)
		return false
	adv = scene.instantiate()
	root.add_child(adv)
	cam = adv.get_node("Camera3D")
	_logged_rev = 0
	_revs = []
	return true

func _U(v: Vector3) -> Vector2:
	return cam.unproject_position(v)

func _P(i: int) -> Vector3:
	var node: Node3D = adv.get_node("Proxies").get_node_or_null("proxy_%d" % i)
	if node == null:
		_quit(6, "proxy_%d is missing" % i)
		return Vector3(1e9, 1e9, 1e9)
	return node.global_position

# Within 0.01 m horizontally.
func _near(a: Vector3, b: Vector3) -> bool:
	return Vector2(a.x - b.x, a.z - b.z).length() <= 0.01

func _log() -> String:
	return adv.get_node("Hud")._log.text

func _label() -> Label:
	return adv.get_node("MoveHud/MoveLabel")

func _mode() -> Label:
	return adv.get_node("MoveHud/ModeLabel")

func _waiting() -> Label:
	return adv.get_node("Hud").find_child("Waiting", true, false)

func _press(button: int, at: Vector2) -> void:
	var event := InputEventMouseButton.new()
	event.button_index = button
	event.pressed = true
	event.position = at
	adv._unhandled_input(event)

# ---- frames, waiting and the revision log ----

# Records the events, receipts and move receipts of a revision not yet logged.
func _observe() -> void:
	if adv == null or adv.view == null:
		return
	var rev: int = adv.view.revision()
	if rev != _logged_rev:
		_logged_rev = rev
		_revs.append({
			"events": Array(adv.view.events()),
			"receipts": Array(adv.view.receipts()),
			"moves": Array(adv.view.move_receipts()),
		})

# One frame: a short sleep, the frame, then the codes check and the log.
func _frame(label: String) -> bool:
	if _done:
		return false
	if Time.get_ticks_msec() - _started_ms >= TIMEOUT_MS:
		_quit(4, "%s: not reached within 120 s" % label)
		return false
	OS.delay_msec(5)
	await process_frame
	if adv != null and (adv.open_code != 0 or adv.stop_code != 0):
		_quit(3, "%s: open_code=%d stop_code=%d" % [label, adv.open_code, adv.stop_code])
		return false
	_observe()
	return not _done

# Waits (one frame at a time, at most `ms`) until `cond` is true; false after
# quitting.
func _wait(label: String, cond: Callable, ms: int = 20000) -> bool:
	var until: int = Time.get_ticks_msec() + ms
	while not _done:
		if Time.get_ticks_msec() >= until:
			_quit(4, "%s: not reached within %d ms" % [label, ms])
			return false
		if not await _frame(label):
			return false
		if adv.view != null and cond.call():
			return true
	return false

# `count` frames in which nothing new arrives: no revision holds a receipt or a
# move receipt and the HUD log is unchanged.
func _quiet(label: String, count: int) -> bool:
	var mark: int = _revs.size()
	var before: String = _log()
	for i in range(count):
		if not await _frame(label):
			return false
	for i in range(mark, _revs.size()):
		_want(label + " no receipt", _revs[i]["receipts"].is_empty() and _revs[i]["moves"].is_empty(), str(_revs[i]))
	_want(label + " log unchanged", _log() == before, _log())
	return not _done

# Is there a move receipt for `actor` with `outcome` in the revisions from `mark`?
func _has_move(mark: int, actor: int, outcome: String) -> bool:
	for i in range(mark, _revs.size()):
		for r in _revs[i]["moves"]:
			if r["actor"] == actor and r["outcome"] == outcome:
				return true
	return false

# Is there a revision from `mark` whose events hold `moved` by `entity`?
func _has_moved(mark: int, entity: int) -> bool:
	for i in range(mark, _revs.size()):
		for e in _revs[i]["events"]:
			if e.get("kind", "") == "moved" and e.get("entity", 0) == entity:
				return true
	return false

# Is there a revision from `mark` that holds `moved` by `entity` and a receipt
# with applied true?
func _has_applied_moved(mark: int, entity: int) -> bool:
	for i in range(mark, _revs.size()):
		var moved := false
		var applied := false
		for e in _revs[i]["events"]:
			if e.get("kind", "") == "moved" and e.get("entity", 0) == entity:
				moved = true
		for r in _revs[i]["receipts"]:
			if r.get("applied", false) == true:
				applied = true
		if moved and applied:
			return true
	return false

func _ready_and_known() -> bool:
	return adv.view.revision() >= 1 and adv.view.position_known() == PackedByteArray([1, 1, 1])

# ---- the cases ----

func _run() -> void:
	_ad1()
	if _done:
		return
	if not await _ad1_frame():
		return
	_ad2()
	if _done:
		return
	if not await _ad3():
		return
	if not await _ad4():
		return
	if not await _ad5():
		return
	if not await _ad6():
		return
	if not await _ad7():
		return
	if not await _ad8():
		return
	_ad9()
	if _done:
		return
	if not await _ad10():
		return
	_want("every case reached its last line", _reached == "AD10", _reached)
	if _done:
		return
	print("adventure.gd OK")
	_done = true
	quit(0)

# AD1 runs after one frame.
func _ad1() -> void:
	_reached = "AD0"

func _ad1_frame() -> bool:
	# No code check here: the control run (a missing campaign) must pass AD1 and
	# AD2 and fail in AD3.
	if _done:
		return false
	await process_frame
	var names: Array = []
	for child in adv.get_children():
		names.append(String(child.name))
	var hud: int = names.find("Hud")
	_want("AD1 Hud exists", hud >= 0, str(names))
	if _done:
		return false
	_want("AD1 Pick, Feedback, MoveHud follow Hud", names.slice(hud + 1, hud + 4) == ["Pick", "Feedback", "MoveHud"], str(names))
	if _done:
		return false
	var pick: Node = adv.get_node("Pick")
	_want("AD1 Pick script", pick.get_script().resource_path == "res://scripts/Pick.gd", str(pick.get_script().resource_path))
	_want("AD1 Feedback script", adv.get_node("Feedback").get_script().resource_path == "res://scripts/Feedback.gd", "wrong script")
	_want("AD1 MoveHud is a CanvasLayer", adv.get_node("MoveHud") is CanvasLayer, "not a CanvasLayer")
	_want("AD1 MoveLabel is a Label", adv.get_node("MoveHud/MoveLabel") is Label, "not a Label")
	var move_hud: Node = adv.get_node("MoveHud")
	var move_hud_children: Array = []
	for child in move_hud.get_children():
		move_hud_children.append(String(child.name))
	_want("AD1 MoveHud children", move_hud_children == ["MoveLabel", "ModeLabel"], str(move_hud_children))
	_want("AD1 ModeLabel is a Label", adv.get_node("MoveHud/ModeLabel") is Label, "not a Label")
	_want("AD1 ModeLabel mouse_filter", _mode().mouse_filter == 2, str(_mode().mouse_filter))
	_want("AD11 main scene", ProjectSettings.get_setting("application/run/main_scene") == "res://scenes/Adventure.tscn", str(ProjectSettings.get_setting("application/run/main_scene")))
	_want("AD1 Pick's own input is off", pick.is_processing_unhandled_input() == false, "on")
	_want("AD1 base script is Walk.gd", adv.get_script().get_base_script().resource_path == "res://scripts/Walk.gd", str(adv.get_script().get_base_script().resource_path))
	_reached = "AD1"
	return not _done

# AD2: the source of Adventure.gd, comments stripped, scanned as hud_cases.gd does.
func _scan(path: String) -> Dictionary:
	var kept := PackedStringArray()
	for line in FileAccess.get_file_as_string(path).split("\n"):
		var cut: int = line.find("#")
		kept.append(line if cut < 0 else line.substr(0, cut))
	var source := "\n".join(kept)
	var out := {"opens": [], "session": [], "view": [], "pick": [], "tokens": []}
	for spec in [["opens", r"session\.(open[a-z0-9_]*)\("], ["session", r"session\.([a-z_]+)\("],
			["view", r"view\.([a-z_]+)\("], ["pick", r"\$Pick\.([a-z_]+)\("],
			["tokens", r"[A-Za-z_][A-Za-z0-9_]*"]]:
		var re := RegEx.new()
		re.compile(spec[1])
		for m in re.search_all(source):
			out[spec[0]].append(m.get_string(1 if spec[0] != "tokens" else 0))
	return out

func _ad2() -> void:
	var got := _scan(ADVENTURE_GD)
	_want("AD2 exactly one open call", got["opens"].size() == 1, str(got["opens"]))
	_want("AD2 the open is open_v4_walk", got["opens"] == ["open_v4_walk"], str(got["opens"]))
	for member in got["session"]:
		_want("AD2 session." + member, member in ["open_v4_walk", "controlled", "view", "request_mode", "declare_move", "last_error"], member)
	for member in got["view"]:
		_want("AD2 view." + member, member in ["active", "ended", "names", "net_ids", "movement", "movement_max"], member)
	for member in got["pick"]:
		_want("AD2 $Pick." + member, member in ["pick_proxy", "attack", "set_process_unhandled_input"], member)
	for token in got["tokens"]:
		_want("AD2 banned token " + token, not (token in ["submit_option", "move_to", "halt", "poll", "sides"]), token)
	# Control: the same scan does see the calls it is looking for.
	_want("AD2 control finds request_mode", "request_mode" in got["session"], str(got["session"]))
	_want("AD2 control finds pick_proxy", "pick_proxy" in got["pick"], str(got["pick"]))
	_reached = "AD2"

func _ad3() -> bool:
	if not await _wait("AD3 revision >= 1 and positions known", _ready_and_known, 10000):
		return false
	_want("AD3 P(1)", _near(_P(1), Vector3(3, 0, 5)), str(_P(1)))
	_want("AD3 P(2)", _near(_P(2), Vector3(3, 0, 7)), str(_P(2)))
	_want("AD3 P(3)", _near(_P(3), Vector3(12, 0, 6)), str(_P(3)))
	_want("AD3 controlled", adv.controlled == PackedInt64Array([1, 2]), str(adv.controlled))
	_want("AD3 selected", adv.selected == 1, str(adv.selected))
	_want("AD3 active", adv.view.active() == 0, str(adv.view.active()))
	_want("AD3 label hidden", _label().visible == false, "visible")
	_want("AD11 mode exploring", _mode().visible and _mode().text == "Exploring", "%s/%s" % [str(_mode().visible), _mode().text])
	_want("AD11 waiting hidden", _waiting().visible == false, "visible")
	_want("AD11 waiting_needs_turn", adv.get_node("Hud").waiting_needs_turn == true, str(adv.get_node("Hud").waiting_needs_turn))
	if _done:
		return false
	var log_before: String = _log()
	var s: Vector2 = _U(Vector3(3, 0, 3))
	var calls: int = adv.input_calls
	var mark: int = _revs.size()
	_press(MOUSE_BUTTON_LEFT, s)
	_want("AD3 input_calls", adv.input_calls == calls + 1, str(adv.input_calls))
	if _done:
		return false
	var arrived := func() -> bool:
		return _has_move(mark, 1, "accepted") and adv.view.moving()[0] == 0 and _near(_P(1), adv.ground_point(s))
	if not await _wait("AD3 hero 1 walks to the click", arrived):
		return false
	_want("AD3 log unchanged by the walk", _log() == log_before, _log())
	_press(MOUSE_BUTTON_LEFT, _U(_P(2) + UP))
	_want("AD3 selected 2", adv.selected == 2, str(adv.selected))
	if not await _quiet("AD3 select 2", 10):
		return false
	_press(MOUSE_BUTTON_LEFT, _U(_P(1) + UP))
	_want("AD3 selected 1", adv.selected == 1, str(adv.selected))
	_reached = "AD3"
	return not _done

func _count(text: String, line: String) -> int:
	return text.count(line)

func _ad4() -> bool:
	var engaged_before: int = _count(_log(), ENGAGE_LINE)
	_want("AD4 no engage line yet", engaged_before == 0, _log())
	_press(MOUSE_BUTTON_LEFT, _U(_P(3) + UP))
	_want("AD4 the engage line", _count(_log(), ENGAGE_LINE) == 1, _log())
	if _done:
		return false
	var turns := func() -> bool:
		return adv.view.active() == 3
	if not await _wait("AD4 the Goblin's turn", turns, 3000):
		return false
	_want("AD4 selected stays 1", adv.selected == 1, str(adv.selected))
	_reached = "AD4"
	return not _done

func _ad5() -> bool:
	var mark: int = 0
	var goblin_moved := func() -> bool:
		return _has_moved(mark, 3)
	if not await _wait("AD5 the Goblin moves", goblin_moved, 10000):
		return false
	var hero_turn := func() -> bool:
		return adv.view.active() == 1 and Array(adv.view.option_labels()) == ["End turn"]
	if not await _wait("AD5 hero 1's turn", hero_turn, 10000):
		return false
	_want("AD5 selected", adv.selected == 1, str(adv.selected))
	var marker: Node3D = adv.get_node("Marker")
	var p1: Vector3 = _P(1)
	_want("AD5 marker visible", marker.visible, "hidden")
	_want("AD5 marker under P(1)", _near(marker.global_position, p1), "%s vs %s" % [marker.global_position, p1])
	_want("AD5 label visible", _label().visible, "hidden")
	_want("AD5 label text", _label().text == "Movement: 6 / 6", _label().text)
	_want("AD11 mode fight", _mode().text == "Fight", _mode().text)
	_reached = "AD5"
	return not _done

func _ad6() -> bool:
	var h: Vector3 = _P(1)
	_want("AD6 hero 1 is at (3,0,3)", _near(h, Vector3(3, 0, 3)), str(h))
	if _done:
		return false
	var s: Vector2 = _U(h + Vector3(0, 0, STEP_Z))
	var mark: int = _revs.size()
	_press(MOUSE_BUTTON_LEFT, s)
	var applied := func() -> bool:
		return _has_applied_moved(mark, 1)
	if not await _wait("AD6 the move is applied", applied, 10000):
		return false
	var arrived := func() -> bool:
		return _near(_P(1), adv.ground_point(s))
	if not await _wait("AD6 hero 1 arrives", arrived, 10000):
		return false
	_want("AD6 label after the move", _label().text == "Movement: 4 / 6", _label().text)
	if _done:
		return false
	# Beyond the allowance: refused by the host, shown in the log, nothing moves.
	var before_pos: Vector3 = _P(1)
	var rejected_before: int = _count(_log(), REJECT_LINE)
	_press(MOUSE_BUTTON_LEFT, _U(h + Vector3(4.5, 0, STEP_Z)))
	var refused := func() -> bool:
		return _count(_log(), REJECT_LINE) == rejected_before + 1
	if not await _wait("AD6 the refusal line", refused, 5000):
		return false
	for i in range(10):
		if not await _frame("AD6 hold after the refusal"):
			return false
		_want("AD6 P(1) unchanged", _P(1) == before_pos, "%s vs %s" % [_P(1), before_pos])
	_want("AD6 label unchanged", _label().text == "Movement: 4 / 6", _label().text)
	if _done:
		return false
	_press(MOUSE_BUTTON_LEFT, _U(_P(2) + UP))
	if not await _quiet("AD6 own hero 2", 10):
		return false
	var calls: int = adv.input_calls
	_press(MOUSE_BUTTON_RIGHT, _U(h))
	_want("AD6 right press counted", adv.input_calls == calls + 1, str(adv.input_calls))
	if not await _quiet("AD6 right press", 10):
		return false
	_reached = "AD6"
	return not _done

func _ad7() -> bool:
	var strikes: int = 0
	var handled: int = -1
	var until: int = Time.get_ticks_msec() + 90000
	while not _done and not adv.view.ended():
		if Time.get_ticks_msec() >= until:
			_quit(4, "AD7 the fight did not end within 90 s")
			return false
		if not await _frame("AD7 the fight"):
			return false
		var view = adv.view
		if view.ended() or view.revision() == handled:
			continue
		var active: int = int(view.active())
		var labels: Array = Array(view.option_labels())
		if active < 1 or active > 2 or labels.is_empty():
			continue
		handled = view.revision()
		var targets: Array = Array(view.option_targets())
		if targets.has(3):
			strikes += 1
			_press(MOUSE_BUTTON_LEFT, _U(_P(3) + UP))
		else:
			var end_index: int = targets.find(0)
			_want("AD7 an End turn option", end_index >= 0, str(targets))
			if _done:
				return false
			var code: int = adv.choose(end_index)
			if code != 0 and code != 4 and code != 5:
				_quit(3, "AD7 choose(%d) = %d (%s)" % [end_index, code, adv.session.last_error()])
				return false
	if _done:
		return false
	if not await _frame("AD7 one frame after the end"):
		return false
	_want("AD7 at least one strike by click", strikes >= 1, str(strikes))
	_want("AD7 numbers spawned", adv.get_node("Feedback").numbers_spawned >= strikes, str(adv.get_node("Feedback").numbers_spawned))
	_want("AD7 dead", Array(adv.view.dead()) == [0, 0, 1], str(adv.view.dead()))
	_want("AD7 no turn head", adv.view.active() == 0, str(adv.view.active()))
	_want("AD7 label hidden", _label().visible == false, "visible")
	_reached = "AD7"
	return not _done

func _ad8() -> bool:
	# `selected` stays on whichever hero acted last, so choose hero 1 by click.
	_press(MOUSE_BUTTON_LEFT, _U(_P(1) + UP))
	_want("AD8 selected 1", adv.selected == 1, str(adv.selected))
	if _done:
		return false
	var s: Vector2 = _U(_P(1) + Vector3(1, 0, 0))
	var mark: int = _revs.size()
	_press(MOUSE_BUTTON_LEFT, s)
	var arrived := func() -> bool:
		return _has_move(mark, 1, "accepted") and adv.view.moving()[0] == 0 and _near(_P(1), adv.ground_point(s))
	if not await _wait("AD8 hero 1 walks again", arrived, 15000):
		return false
	_want("AD11 mode exploring again", _mode().text == "Exploring", _mode().text)
	_want("AD11 waiting hidden again", _waiting().visible == false, "visible")
	_press(MOUSE_BUTTON_LEFT, _U(_P(2) + UP))
	_want("AD8 selected 2", adv.selected == 2, str(adv.selected))
	var s2: Vector2 = _U(_P(2) + Vector3(1, 0, 0))
	var mark2: int = _revs.size()
	_press(MOUSE_BUTTON_LEFT, s2)
	var arrived2 := func() -> bool:
		return _has_move(mark2, 2, "accepted") and adv.view.moving()[1] == 0 and _near(_P(2), adv.ground_point(s2))
	if not await _wait("AD8 hero 2 walks again", arrived2, 15000):
		return false
	var engaged: int = _count(_log(), "You engage")
	_press(MOUSE_BUTTON_LEFT, _U(_P(3) + UP))
	_want("AD8 no engage line for the dead Goblin", _count(_log(), "You engage") == engaged, _log())
	_reached = "AD8"
	return not _done

func _ad9() -> void:
	root.remove_child(adv)
	_want("AD9 session is null", adv.session == null, str(adv.session))
	adv.free()
	adv = null
	_reached = "AD9"

# AD10: the proximity rule through the scene, with no click on the Goblin.
func _ad10() -> bool:
	if not _spawn():
		return false
	if not await _wait("AD10 revision >= 1", func() -> bool: return adv.view.revision() >= 1, 10000):
		return false
	_press(MOUSE_BUTTON_LEFT, _U(Vector3(9, 0, 6)))
	var turns := func() -> bool:
		return adv.view.active() == 3
	if not await _wait("AD10 the fight starts by itself", turns, 10000):
		return false
	_want("AD10 no engage line", _count(_log(), "You engage") == 0, _log())
	var gap: float = Vector2(_P(1).x - _P(3).x, _P(1).z - _P(3).z).length()
	_want("AD10 hero 1 is within 4 m of the Goblin", gap <= 4.0 + 0.01, str(gap))
	if _done:
		return false
	root.remove_child(adv)
	adv.free()
	adv = null
	_reached = "AD10"
	return not _done
