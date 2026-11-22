extends SceneTree
# T156c: P1-P6 (Pick.gd), F1-F8 (Feedback.gd), then L1-L4 against the real
# Main.tscn session. One coroutine drives everything; the session state is read
# only after `await process_frame`, so never mid-`_process`.
# Run: Godot4 --headless --path apps/client -s res://tests/combat.gd
# Exit 0 pass; 2 class missing; 3 open_code/stop_code non-zero or a call returned
# an unexpected code; 4 timeout (120 s overall); 6 a value check failed (the
# message names the case).
# Control: CRPG_CAMPAIGN set to a missing directory -> L1's open fails with 20,
# exit 3, after P and F pass.

const TIMEOUT_MS := 120000
const PICK := "res://scripts/Pick.gd"
const FEEDBACK := "res://scripts/Feedback.gd"
const EPS := 1e-3

var _main: Node = null
var _started_ms: int = 0
var _done: bool = false
var _reached: String = ""
var _obs_rev: int = -1
var _events: Array = []    # every event of every revision seen, oldest first

# ---- the fakes (the getters the code under test may read) -----------------

class FakeView extends RefCounted:
	var _d: Dictionary

	func _init(d: Dictionary) -> void:
		_d = d

	func revision(): return _d.get("revision", 1)
	func net_ids(): return _d.get("net_ids", PackedInt64Array())
	func names(): return _d.get("names", PackedStringArray())
	func health(): return _d.get("health", PackedInt64Array())
	func max_health(): return _d.get("max_health", PackedInt64Array())
	func health_known(): return _d.get("health_known", PackedByteArray())
	func dead(): return _d.get("dead", PackedByteArray())
	func events(): return _d.get("events", [])
	func option_targets(): return _d.get("option_targets", PackedInt64Array())

class FakeSession extends RefCounted:
	var text := ""

	func last_error() -> String:
		return text

class FakeHud extends Node:
	var notices: Array = []
	var errors: Array = []

	func show_notice(text: String) -> void:
		notices.append(text)

	func show_error(text: String) -> void:
		errors.append(text)

class FakeMain extends Node3D:
	var view = null
	var session = null
	var chosen: Array = []
	var answer: int = 0
	var cam: Camera3D
	var proxies: Node3D
	var hud: FakeHud

	func _init() -> void:
		cam = Camera3D.new()
		cam.name = "Camera3D"
		add_child(cam)
		proxies = Node3D.new()
		proxies.name = "Proxies"
		add_child(proxies)
		hud = FakeHud.new()
		hud.name = "Hud"
		add_child(hud)

	func choose(index: int) -> int:
		chosen.append(index)
		return answer

# ---- entry and helpers ----------------------------------------------------

func _initialize() -> void:
	if not ClassDB.class_exists("CrpgSession"):
		printerr("combat.gd: class CrpgSession is missing")
		quit(2)
		return
	_started_ms = Time.get_ticks_msec()
	_run()

func _quit(code: int, message: String) -> void:
	if _done:
		return
	_done = true
	printerr("combat.gd FAIL: ", message)
	quit(code)

func _eq(label: String, got, want) -> void:
	if typeof(got) != typeof(want) and not (typeof(got) in [TYPE_INT, TYPE_FLOAT] and typeof(want) in [TYPE_INT, TYPE_FLOAT]):
		_quit(6, "%s: got %s, want %s" % [label, var_to_str(got), var_to_str(want)])
	elif got != want:
		_quit(6, "%s: got %s, want %s" % [label, var_to_str(got), var_to_str(want)])

func _check(label: String, ok: bool) -> void:
	if not ok:
		_quit(6, label)

func _near_f(label: String, got: float, want: float) -> void:
	if absf(got - want) > EPS:
		_quit(6, "%s: got %f, want %f" % [label, got, want])

func _near_v2(label: String, got: Vector2, want: Vector2) -> void:
	if absf(got.x - want.x) > EPS or absf(got.y - want.y) > EPS:
		_quit(6, "%s: got %s, want %s" % [label, str(got), str(want)])

func _near_v3(label: String, got: Vector3, want: Vector3) -> void:
	if absf(got.x - want.x) > EPS or absf(got.y - want.y) > EPS or absf(got.z - want.z) > EPS:
		_quit(6, "%s: got %s, want %s" % [label, str(got), str(want)])

# A FakeView from arrays: the integer columns become PackedInt64Array and so on.
func _v(d: Dictionary = {}) -> FakeView:
	var out: Dictionary = {}
	for key in d.keys():
		if key in ["net_ids", "health", "max_health", "option_targets"]:
			out[key] = PackedInt64Array(d[key])
		elif key == "names":
			out[key] = PackedStringArray(d[key])
		elif key in ["health_known", "dead"]:
			out[key] = PackedByteArray(d[key])
		else:
			out[key] = d[key]
	return FakeView.new(out)

func _ar(target: int, outcome: String, damage: int) -> Dictionary:
	return {"kind": "action_resolved", "actor": 1, "target": target, "ability": "Strike",
		"ability_id": "x", "outcome": outcome, "damage": damage}

func _ev(button: int, pressed: bool, at: Vector2) -> InputEventMouseButton:
	var event := InputEventMouseButton.new()
	event.button_index = button
	event.pressed = pressed
	event.position = at
	return event

func _drop(node: Node) -> void:
	if node.get_parent() != null:
		node.get_parent().remove_child(node)
	node.queue_free()

# A FakeMain under root with the unit camera.
func _fixture() -> FakeMain:
	var fake := FakeMain.new()
	root.add_child(fake)
	fake.cam.look_at_from_position(Vector3(1.5, 6.0, 9.0), Vector3(1.5, 0.0, 0.0), Vector3.UP)
	return fake

func _proxy(parent: Node, id: int, at: Vector3) -> Node3D:
	var node := Node3D.new()
	node.name = "proxy_%d" % id
	parent.add_child(node)
	node.position = at
	return node

func _unit_proxies(fake: FakeMain) -> void:
	_proxy(fake.proxies, 1, Vector3(0, 0, 0))
	_proxy(fake.proxies, 2, Vector3(3, 0, 0))

func _u(fake: FakeMain, at: Vector3) -> Vector2:
	return fake.cam.unproject_position(at)

# The P4 world: Hero 1, Goblin 2, one option against the Goblin and End turn.
func _p4_world(fake: FakeMain) -> void:
	fake.view = _v({"net_ids": [1, 2], "names": ["Hero", "Goblin"], "option_targets": [2, 0]})
	var session := FakeSession.new()
	session.text = "INTENT_REFUSED: x"
	fake.session = session

func _make_pick(fake: FakeMain):
	var pick = load(PICK).new()
	fake.add_child(pick)
	pick.main = fake
	return pick

func _make_feedback(processing: bool = false):
	var feedback = load(FEEDBACK).new()
	root.add_child(feedback)
	if not processing:
		feedback.set_process(false)
	return feedback

# ---- frames, waiting and the event log ------------------------------------

# Appends the events of a revision not yet seen.
func _observe() -> void:
	if _main == null or _main.view == null:
		return
	var rev: int = _main.view.revision()
	if rev != _obs_rev:
		_obs_rev = rev
		for event in _main.view.events():
			_events.append(event)

# One frame: a short sleep, the frame, then the codes check and the event log.
func _frame(label: String) -> bool:
	if _done:
		return false
	if Time.get_ticks_msec() - _started_ms >= TIMEOUT_MS:
		_quit(4, "%s: not reached within 120 s" % label)
		return false
	OS.delay_msec(5)
	await process_frame
	if _main != null and (_main.open_code != 0 or _main.stop_code != 0):
		var text := ""
		if _main.session != null:
			text = _main.session.last_error()
		_quit(3, "%s: open_code=%d stop_code=%d (%s)" % [label, _main.open_code, _main.stop_code, text])
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

func _case(label: String, body: Callable) -> void:
	if _done:
		return
	_reached = ""
	body.call()
	# A runtime error ends a case early without failing the run; only reaching
	# the last line counts.
	if not _done and _reached != label:
		_quit(6, "%s did not finish (a runtime error?)" % label)

func _async_case(label: String, ok) -> bool:
	if ok != true:
		_quit(6, "%s did not finish (a runtime error?)" % label)
		return false
	return not _done

func _run() -> void:
	await process_frame
	_case("P1", _p1)
	_case("P2", _p2)
	_case("P3", _p3)
	_case("P4", _p4)
	_case("P5", _p5)
	_case("P6", _p6)
	_case("F1", _f1)
	_case("F2", _f2)
	_case("F3", _f3)
	_case("F4", _f4)
	_case("F5", _f5)
	_case("F6", _f6)
	_case("F7", _f7)
	if _done:
		return
	if not _async_case("F8", await _f8()):
		return
	if not _async_case("L1", await _l1()):
		return
	if not _async_case("L2", await _l2()):
		return
	if not _async_case("L3", await _l3()):
		return
	if not _async_case("L4", await _l4()):
		return
	print("combat.gd OK")
	_done = true
	quit(0)

# ---- P: Pick.gd -----------------------------------------------------------

func _p1() -> void:
	var fake := _fixture()
	_unit_proxies(fake)
	var pick = _make_pick(fake)
	var cam := fake.cam
	var proxies := fake.proxies
	_eq("P1 proxy_2 centre", pick.pick_proxy(cam, proxies, _u(fake, Vector3(3, 0.6, 0))), 2)
	_eq("P1 proxy_1 centre", pick.pick_proxy(cam, proxies, _u(fake, Vector3(0, 0.6, 0))), 1)
	_eq("P1 between", pick.pick_proxy(cam, proxies, _u(fake, Vector3(1.5, 0.6, 0))), 0)
	_eq("P1 0.59 m off", pick.pick_proxy(cam, proxies, _u(fake, Vector3(3.6, 0.6, 0))), 2)
	_eq("P1 0.78 m off", pick.pick_proxy(cam, proxies, _u(fake, Vector3(3.8, 0.6, 0))), 0)
	_drop(fake)
	_reached = "P1"

func _p2() -> void:
	var fake := _fixture()
	var pick = _make_pick(fake)
	_proxy(fake.proxies, 5, Vector3(6, 0, 0))
	_proxy(fake.proxies, 4, Vector3(6, 0, 0))
	var marker := Node3D.new()
	marker.name = "marker"
	fake.proxies.add_child(marker)
	marker.position = Vector3(6, 0, 0)
	_eq("P2 tie takes the smaller id", pick.pick_proxy(fake.cam, fake.proxies, _u(fake, Vector3(6, 0.6, 0))), 4)
	_drop(fake)
	# Behind the camera, then the same proxy in front of it.
	fake = _fixture()
	pick = _make_pick(fake)
	var screen := _u(fake, Vector3(0, 0.6, 0))
	var origin: Vector3 = fake.cam.project_ray_origin(screen)
	var dir: Vector3 = fake.cam.project_ray_normal(screen)
	var behind := _proxy(fake.proxies, 3, origin - dir * 2.0 - Vector3(0, 0.6, 0))
	_eq("P2 behind the camera", pick.pick_proxy(fake.cam, fake.proxies, screen), 0)
	behind.position = origin + dir * 2.0 - Vector3(0, 0.6, 0)
	_eq("P2 control in front", pick.pick_proxy(fake.cam, fake.proxies, screen), 3)
	_drop(fake)
	_reached = "P2"

func _p3() -> void:
	var fake := _fixture()
	var pick = _make_pick(fake)
	var view := _v({"option_targets": [2, 0]})
	_eq("P3 first match", pick.attack_index(view, 2), 0)
	_eq("P3 end turn never matches", pick.attack_index(view, 0), -1)
	_eq("P3 no option", pick.attack_index(view, 1), -1)
	_eq("P3 later option", pick.attack_index(_v({"option_targets": [3, 2, 2, 0]}), 2), 1)
	_eq("P3 null view", pick.attack_index(null, 2), -1)
	_drop(fake)
	_reached = "P3"

func _p4() -> void:
	var fake := _fixture()
	var pick = _make_pick(fake)
	_p4_world(fake)
	var hud: FakeHud = fake.hud
	_eq("P4 attack(2)", pick.attack(2), 0)
	_eq("P4 chosen", fake.chosen, [0])
	_eq("P4 no notice", hud.notices, [])
	_eq("P4 no error", hud.errors, [])
	_eq("P4 attack(1)", pick.attack(1), -1)
	_eq("P4 chosen unchanged", fake.chosen, [0])
	_eq("P4 notice", hud.notices, ["No option targets Hero"])
	_eq("P4 attack(7)", pick.attack(7), -1)
	_eq("P4 notice for no row", hud.notices[hud.notices.size() - 1], "No option targets -")
	fake.answer = 30
	_eq("P4 code 30", pick.attack(2), 30)
	_eq("P4 code 30 notice", hud.notices[hud.notices.size() - 1], "INTENT_REFUSED: x")
	var notices_before: int = hud.notices.size()
	for code in [4, 5]:
		fake.answer = code
		_eq("P4 code %d" % code, pick.attack(2), code)
	_eq("P4 4/5 add no notice", hud.notices.size(), notices_before)
	_eq("P4 4/5 add no error", hud.errors, [])
	fake.answer = 7
	_eq("P4 code 7", pick.attack(2), 7)
	_eq("P4 errors", hud.errors, ["INTENT_REFUSED: x"])
	_eq("P4 7 adds no notice", hud.notices.size(), notices_before)
	fake.view = null
	_eq("P4 null view", pick.attack(2), -1)
	_eq("P4 null view notice", hud.notices[hud.notices.size() - 1], "No option targets -")
	var chosen_before: int = fake.chosen.size()
	fake.session = null
	_eq("P4 null session", pick.attack(2), 1)
	_eq("P4 null session sends nothing", fake.chosen.size(), chosen_before)
	_p4_world(fake)
	pick.main = null
	_eq("P4 null main", pick.attack(2), 1)
	_eq("P4 null main sends nothing", fake.chosen.size(), chosen_before)
	_drop(fake)
	_reached = "P4"

func _p5() -> void:
	var fake := _fixture()
	_unit_proxies(fake)
	var pick = _make_pick(fake)
	_p4_world(fake)
	var hud: FakeHud = fake.hud
	var goblin := _u(fake, Vector3(3, 0.6, 0))
	pick._unhandled_input(_ev(MOUSE_BUTTON_LEFT, true, goblin))
	_eq("P5 press counted", pick.input_calls, 1)
	_eq("P5 press chose", fake.chosen, [0])
	var motion := InputEventMouseMotion.new()
	motion.position = goblin
	var key := InputEventKey.new()
	key.keycode = KEY_SPACE
	key.pressed = true
	var others: Array = [_ev(MOUSE_BUTTON_LEFT, false, goblin), _ev(MOUSE_BUTTON_RIGHT, true, goblin), motion, key]
	for i in range(others.size()):
		pick._unhandled_input(others[i])
		_eq("P5 other event %d: calls" % i, pick.input_calls, 1)
		_eq("P5 other event %d: chosen" % i, fake.chosen, [0])
	pick._unhandled_input(_ev(MOUSE_BUTTON_LEFT, true, _u(fake, Vector3(1.5, 0.6, 0))))
	_eq("P5 empty click counted", pick.input_calls, 2)
	_eq("P5 empty click chose nothing", fake.chosen, [0])
	_eq("P5 empty click noted nothing", hud.notices, [])
	pick.main = null
	pick._unhandled_input(_ev(MOUSE_BUTTON_LEFT, true, goblin))
	_eq("P5 null main", pick.input_calls, 2)
	_drop(fake)

# `_scan` of hud_cases.gd, with the comment-stripped source kept for literal checks.
	_reached = "P5"

func _source(path: String) -> String:
	var kept := PackedStringArray()
	for line in FileAccess.get_file_as_string(path).split("\n"):
		var cut: int = line.find("#")
		kept.append(line if cut < 0 else line.substr(0, cut))
	return "\n".join(kept)

func _scan(path: String) -> Dictionary:
	var source := _source(path)
	var out := {"view": [], "main": [], "session": [], "tokens": []}
	for spec in [["view", r"view\.([a-z_]+)\("], ["main", r"main\.([a-z_]+)"],
			["session", r"session\.([a-z_]+)\("], ["tokens", r"[A-Za-z_][A-Za-z0-9_]*"]]:
		var re := RegEx.new()
		re.compile(spec[1])
		for m in re.search_all(source):
			out[spec[0]].append(m.get_string(1 if spec[0] != "tokens" else 0))
	return out

func _p6() -> void:
	var pick := _scan(PICK)
	var feedback := _scan(FEEDBACK)
	for member in pick["main"]:
		_check("P6 Pick main.%s" % member, member in ["view", "session", "choose", "get_node", "get_node_or_null"])
	for member in pick["view"]:
		_check("P6 Pick view.%s()" % member, member in ["option_targets", "net_ids", "names"])
	for member in pick["session"]:
		_check("P6 Pick session.%s()" % member, member == "last_error")
	for token in pick["tokens"]:
		_check("P6 Pick token %s" % token, not (token in ["submit_option", "poll", "move_to", "declare_move", "halt", "ClassDB"]))
	for member in feedback["main"]:
		_check("P6 Feedback main.%s" % member, member in ["view", "get_node_or_null"])
	for member in feedback["view"]:
		_check("P6 Feedback view.%s()" % member,
			member in ["revision", "net_ids", "health", "max_health", "health_known", "dead", "events"])
	for token in feedback["tokens"]:
		_check("P6 Feedback token %s" % token,
			not (token in ["choose", "submit_option", "poll", "session", "move_to", "set_surface_override_material"]))
	for path in [PICK, FEEDBACK]:
		var source := _source(path)
		_check("P6 %s has no \"Mesh\" literal" % path, not ("\"Mesh\"" in source))
		_check("P6 %s has no \"Label\" literal" % path, not ("\"Label\"" in source))
	_check("P6 control: Pick.gd names choose", "choose" in pick["tokens"])
	_check("P6 control: Feedback.gd reads events", "events" in feedback["view"])
	_reached = "P6"

# ---- F: Feedback.gd -------------------------------------------------------

func _bar(feedback, id: int):
	return feedback.get_node_or_null("Overlay/bar_%d" % id)

func _f1() -> void:
	var feedback = _make_feedback()
	feedback.show_view(_v({"net_ids": [1, 2], "health": [5, 3], "max_health": [10, 6],
		"health_known": [1, 1], "dead": [0, 0]}))
	var bar1 = _bar(feedback, 1)
	var bar2 = _bar(feedback, 2)
	_check("F1 bar_1 exists", bar1 is ProgressBar)
	_check("F1 bar_2 exists", bar2 is ProgressBar)
	if _done:
		return
	_eq("F1 bar_1 max", bar1.max_value, 10)
	_eq("F1 bar_1 value", bar1.value, 5)
	_eq("F1 show_percentage", bar1.show_percentage, false)
	_eq("F1 mouse_filter", bar1.mouse_filter, Control.MOUSE_FILTER_IGNORE)
	_eq("F1 size", bar1.size, load(FEEDBACK).BAR_SIZE)
	_eq("F1 visible", bar1.visible, true)
	_eq("F1 bar_2 max", bar2.max_value, 6)
	_eq("F1 bar_2 value", bar2.value, 3)
	feedback.show_view(_v({"net_ids": [1, 2], "health": [5, 0], "max_health": [10, 6],
		"health_known": [1, 1], "dead": [0, 1]}))
	bar2 = _bar(feedback, 2)
	_check("F1 dead bar_2 kept", bar2 != null)
	if _done:
		return
	_eq("F1 dead bar_2 hidden", bar2.visible, false)
	_eq("F1 living bar_1 visible", _bar(feedback, 1).visible, true)
	_drop(feedback)
	_reached = "F1"

func _f2() -> void:
	var feedback = _make_feedback()
	var rows := {"net_ids": [1, 2], "health": [5, 3], "max_health": [10, 6],
		"health_known": [1, 1], "dead": [0, 0]}
	feedback.show_view(_v(rows))
	rows["health_known"] = [0, 1]
	feedback.show_view(_v(rows))
	_check("F2 unknown health has no bar_1", _bar(feedback, 1) == null)
	_check("F2 bar_2 kept", _bar(feedback, 2) != null)
	rows["health_known"] = [1, 1]
	rows["max_health"] = [10, 0]
	feedback.show_view(_v(rows))
	_check("F2 zero max_health drops bar_2", _bar(feedback, 2) == null)
	_check("F2 bar_1 back", _bar(feedback, 1) != null)
	feedback.show_view(_v({"net_ids": [2], "health": [3], "max_health": [6], "health_known": [1], "dead": [0]}))
	_check("F2 gone row drops bar_1", _bar(feedback, 1) == null)
	_check("F2 bar_2 appears", _bar(feedback, 2) != null)
	if _done:
		return
	feedback.show_view(_v({"net_ids": [2], "health": [12], "max_health": [10], "health_known": [1], "dead": [0]}))
	_eq("F2 over max clamps", _bar(feedback, 2).value, 10)
	feedback.show_view(_v({"net_ids": [2], "health": [-3], "max_health": [10], "health_known": [1], "dead": [0]}))
	_eq("F2 below zero clamps", _bar(feedback, 2).value, 0)
	_drop(feedback)
	_reached = "F2"

func _f3() -> void:
	var script = load(FEEDBACK)
	var feedback = _make_feedback()
	feedback.show_view(_v({"events": [_ar(2, "success", 4)]}))
	_eq("F3 spawned", feedback.numbers_spawned, 1)
	var label = feedback.get_node_or_null("Overlay/num_1")
	_check("F3 num_1 is a Label", label is Label)
	if _done:
		return
	_eq("F3 text", label.text, "-4")
	_eq("F3 mouse_filter", label.mouse_filter, Control.MOUSE_FILTER_IGNORE)
	_eq("F3 colour", label.get_theme_color("font_color"), script.HIT_COLOR)
	feedback.show_view(_v({"events": [
		_ar(2, "critical_success", 6), _ar(1, "failure", 0), _ar(1, "success", 0),
		_ar(2, "critical_failure", 0), _ar(2, "failure", 3),
		{"kind": "turn_started", "actor": 2, "round": 0},
		{"kind": "died", "entity": 1},
		{"kind": "encounter_ended", "encounter": "x", "round": 0},
	]}))
	_eq("F3 spawned after", feedback.numbers_spawned, 6)
	var want_text := ["-6!", "miss", "0", "miss", "-3"]
	var want_hit := [true, false, false, false, true]
	for i in range(5):
		var number = feedback.get_node_or_null("Overlay/num_%d" % (i + 2))
		_check("F3 num_%d exists" % (i + 2), number is Label)
		if _done:
			return
		_eq("F3 num_%d text" % (i + 2), number.text, want_text[i])
		_eq("F3 num_%d colour" % (i + 2), number.get_theme_color("font_color"),
			script.HIT_COLOR if want_hit[i] else script.MISS_COLOR)
	_drop(feedback)
	_reached = "F3"

func _flash_count(feedback, id: int) -> int:
	var count: int = 0
	for child in feedback.get_children():
		if String(child.name) == "flash_%d" % id:
			count += 1
	return count

func _f4() -> void:
	var script = load(FEEDBACK)
	var feedback = _make_feedback()
	feedback.show_view(_v({"events": [_ar(2, "success", 4)]}))
	var flash = feedback.get_node_or_null("flash_2")
	_check("F4 flash_2 is a MeshInstance3D", flash is MeshInstance3D)
	if _done:
		return
	_check("F4 mesh is a SphereMesh", flash.mesh is SphereMesh)
	if _done:
		return
	_near_f("F4 radius", flash.mesh.radius, script.FLASH_RADIUS)
	feedback.show_view(_v({"events": [_ar(1, "failure", 0)]}))
	_check("F4 a miss does not flash", feedback.get_node_or_null("flash_1") == null)
	feedback.advance(200.0)
	feedback.show_view(_v({"events": [_ar(2, "success", 5)]}))
	_eq("F4 one flash after a second hit", _flash_count(feedback, 2), 1)
	feedback.advance(200.0)
	_eq("F4 age was reset", _flash_count(feedback, 2), 1)
	_drop(feedback)
	_reached = "F4"

func _f5() -> void:
	var script = load(FEEDBACK)
	var feedback = _make_feedback()
	feedback.show_view(_v({"events": [_ar(1, "failure", 0)]}))
	var path := "Overlay/num_%d" % feedback.numbers_spawned
	var label = feedback.get_node_or_null(path)
	_check("F5 number exists", label != null)
	if _done:
		return
	feedback.advance(script.NUMBER_MS / 2.0)
	_near_f("F5 modulate.a halfway", label.modulate.a, 0.5)
	feedback.advance(script.NUMBER_MS / 2.0 - 1.0)
	_check("F5 number lives to NUMBER_MS - 1", feedback.get_node_or_null(path) != null)
	feedback.advance(1.0)
	_check("F5 number gone at NUMBER_MS", feedback.get_node_or_null(path) == null)
	feedback.show_view(_v({"events": [_ar(2, "success", 4)]}))
	feedback.advance(script.FLASH_MS - 1.0)
	_check("F5 flash lives to FLASH_MS - 1", feedback.get_node_or_null("flash_2") != null)
	feedback.advance(1.0)
	_check("F5 flash gone at FLASH_MS", feedback.get_node_or_null("flash_2") == null)
	_drop(feedback)
	_reached = "F5"

func _f6() -> void:
	var feedback = _make_feedback()
	var events := [_ar(2, "success", 4)]
	feedback.sync(_v({"revision": 3, "events": events}))
	feedback.sync(_v({"revision": 3, "events": events}))
	_eq("F6 once per revision", feedback.numbers_spawned, 1)
	feedback.sync(_v({"revision": 4, "events": events}))
	_eq("F6 a new revision spawns again", feedback.numbers_spawned, 2)
	feedback.sync(null)
	_eq("F6 null view changes nothing", feedback.numbers_spawned, 2)
	_drop(feedback)
	_reached = "F6"

func _f7() -> void:
	var script = load(FEEDBACK)
	var fake := _fixture()
	_unit_proxies(fake)
	var feedback = _make_feedback()
	feedback.show_view(_v({"net_ids": [1, 2], "health": [5, 3], "max_health": [10, 6],
		"health_known": [1, 1], "dead": [0, 0], "events": [_ar(2, "success", 4)]}))
	feedback.place(fake.cam, fake.proxies)
	var p1: Node3D = fake.proxies.get_node("proxy_1")
	var p2: Node3D = fake.proxies.get_node("proxy_2")
	var bar1 = _bar(feedback, 1)
	var number = feedback.get_node_or_null("Overlay/num_1")
	var flash = feedback.get_node_or_null("flash_2")
	_check("F7 nodes exist", bar1 != null and number != null and flash != null)
	if _done:
		return
	_near_v2("F7 bar_1 position",
		bar1.position, _u(fake, p1.global_position + Vector3(0, script.BAR_LIFT, 0)) - script.BAR_SIZE / 2.0)
	_near_v2("F7 number position",
		number.position, _u(fake, p2.global_position + Vector3(0, script.NUMBER_LIFT, 0)))
	_near_v3("F7 flash position", flash.global_position, p2.global_position + Vector3(0, script.FLASH_LIFT, 0))
	var y0: float = number.position.y
	feedback.advance(script.NUMBER_MS / 2.0)
	feedback.place(fake.cam, fake.proxies)
	_near_f("F7 number rises", y0 - number.position.y, script.NUMBER_RISE / 2.0)
	# The flash (250 ms) died during the half-life advance above: hit again so a
	# live flash is there when the proxy goes.
	feedback.show_view(_v({"revision": 2, "net_ids": [1, 2], "health": [5, 3], "max_health": [10, 6],
		"health_known": [1, 1], "dead": [0, 0], "events": [_ar(2, "success", 1)]}))
	feedback.place(fake.cam, fake.proxies)
	flash = feedback.get_node_or_null("flash_2")
	_check("F7 a second hit re-flashes", flash != null)
	if _done:
		return
	_eq("F7 flash visible with a proxy", flash.visible, true)
	fake.proxies.remove_child(p2)
	p2.queue_free()
	feedback.place(fake.cam, fake.proxies)
	_eq("F7 bar_2 hidden without a proxy", _bar(feedback, 2).visible, false)
	_eq("F7 number hidden without a proxy", number.visible, false)
	_eq("F7 flash hidden without a proxy", flash.visible, false)
	_eq("F7 bar_1 still visible", bar1.visible, true)
	_drop(feedback)
	_drop(fake)
	_reached = "F7"

func _f8() -> bool:
	var feedback = _make_feedback(true)
	var plain := Node.new()
	root.add_child(plain)
	feedback.main = plain
	await process_frame
	await process_frame
	_eq("F8 nothing spawned", feedback.numbers_spawned, 0)
	_eq("F8 overlay is empty", feedback.get_node("Overlay").get_child_count(), 0)
	_drop(feedback)
	_drop(plain)
	return true

# ---- L: the real scene ----------------------------------------------------

func _all_known(view) -> bool:
	var known = view.health_known()
	if known.size() == 0:
		return false
	for flag in known:
		if int(flag) != 1:
			return false
	return true

func _id_of(view, row_name: String) -> int:
	var row: int = view.names().find(row_name)
	if row < 0:
		return 0
	return int(view.net_ids()[row])

# The screen point over a proxy's pick centre.
func _centre(id: int) -> Vector2:
	var proxy: Node3D = _main.get_node("Proxies").get_node("proxy_%d" % id)
	var at: Vector3 = proxy.global_position + Vector3(0.0, load(PICK).PICK_HEIGHT, 0.0)
	return _main.get_node("Camera3D").unproject_position(at)

func _has_event(mark: int, actor: int, target: int) -> bool:
	for i in range(mark, _events.size()):
		var event: Dictionary = _events[i]
		if event.get("kind", "") == "action_resolved" and event.get("actor", 0) == actor \
				and event.get("target", 0) == target:
			return true
	return false

func _l1() -> bool:
	var scene: PackedScene = load("res://scenes/Main.tscn")
	if scene == null:
		_quit(6, "L1: res://scenes/Main.tscn did not load")
		return false
	_main = scene.instantiate()
	root.add_child(_main)
	var children: Array = _main.get_children()
	var names: Array = []
	for child in children:
		names.append(String(child.name))
	_check("L1 Pick is a child", "Pick" in names and _main.get_node("Pick").get_script().resource_path == PICK)
	_check("L1 Feedback is a child", "Feedback" in names and _main.get_node("Feedback").get_script().resource_path == FEEDBACK)
	if _done:
		return false
	_check("L1 Pick comes after Hud", _main.get_node("Pick").get_index() > _main.get_node("Hud").get_index())
	_check("L1 Feedback comes after Pick", _main.get_node("Feedback").get_index() > _main.get_node("Pick").get_index())
	if not await _wait("L1 health known", func(): return _main.view.revision() >= 1 and _all_known(_main.view)):
		return false
	if not await _frame("L1 bars"):
		return false
	var view = _main.view
	for i in range(view.net_ids().size()):
		var id: int = int(view.net_ids()[i])
		var bar = _main.get_node("Feedback").get_node_or_null("Overlay/bar_%d" % id)
		_check("L1 bar_%d exists" % id, bar != null)
		if _done:
			return false
		_eq("L1 bar_%d max" % id, int(bar.max_value), int(view.max_health()[i]))
	return not _done

func _l2() -> bool:
	var hero: int = _id_of(_main.view, "Hero")
	var goblin: int = _id_of(_main.view, "Goblin")
	_check("L2 both rows named", hero != 0 and goblin != 0)
	if _done:
		return false
	if not await _wait("L2 hero's turn", func(): return _main.view.active() == hero and _main.view.option_targets().has(goblin)):
		return false
	_check("L2 the opening attack drew a number", _main.get_node("Feedback").numbers_spawned >= 1)
	return not _done

func _l3() -> bool:
	var hero: int = _id_of(_main.view, "Hero")
	var goblin: int = _id_of(_main.view, "Goblin")
	var pick = _main.get_node("Pick")
	var feedback = _main.get_node("Feedback")
	var spawned: int = feedback.numbers_spawned
	var calls: int = pick.input_calls
	var mark: int = _events.size()
	pick._unhandled_input(_ev(MOUSE_BUTTON_LEFT, true, _centre(goblin)))
	_eq("L3 the click was handled", pick.input_calls, calls + 1)
	if not await _wait("L3 hero's attack", func(): return _has_event(mark, hero, goblin)):
		return false
	if not await _frame("L3 number"):
		return false
	_check("L3 the attack drew a number", feedback.numbers_spawned > spawned)
	return not _done

func _l4() -> bool:
	var pick = _main.get_node("Pick")
	var goblin: int = _id_of(_main.view, "Goblin")
	var chosen_rev: int = -1
	while not _done:
		if not await _frame("L4 the encounter ends"):
			return false
		var view = _main.view
		if view == null:
			continue
		if view.ended():
			break
		if view.option_labels().size() == 0 or view.revision() == chosen_rev:
			continue
		var code: int = -1
		var targets = view.option_targets()
		if targets.has(goblin):
			# The click: the pick must find the Goblin; the same code comes back
			# from attack(), which _unhandled_input calls for the picked id.
			var picked: int = pick.pick_proxy(_main.get_node("Camera3D"), _main.get_node("Proxies"), _centre(goblin))
			_eq("L4 the click picks the Goblin", picked, goblin)
			code = pick.attack(picked)
		else:
			code = _main.choose(targets.find(0))
		if code == 0:
			chosen_rev = view.revision()
		elif code != 4 and code != 5:
			_quit(3, "L4: submit = %d (%s)" % [code, _main.session.last_error()])
			return false
	if _done:
		return false
	if not await _frame("L4 after the end"):
		return false
	var view = _main.view
	var feedback = _main.get_node("Feedback")
	var dead = view.dead()
	for i in range(view.net_ids().size()):
		var id: int = int(view.net_ids()[i])
		var bar = feedback.get_node_or_null("Overlay/bar_%d" % id)
		_check("L4 bar_%d exists" % id, bar != null)
		if _done:
			return false
		_eq("L4 bar_%d visible only while alive" % id, bar.visible, int(dead[i]) == 0)
	return not _done
