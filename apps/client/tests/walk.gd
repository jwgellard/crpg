extends SceneTree
# WK1-WK10 (T050b): the walk scene (Walk.tscn / Walk.gd) against the real-time
# v4 session over campaigns/demo/walk (WALK: n(1) Hero (3,0,5), n(2) Hero
# (3,0,7), n(3) Goblin (12,0,6)). One coroutine drives the scene; every state
# is read only after `await process_frame`, so never mid-`_process`.
# Run: Godot4 --headless --path apps/client -s res://tests/walk.gd
# Exit 0 pass; 2 class missing; 3 open_code/stop_code non-zero or a call
# returned an unexpected code; 4 timeout (20 s overall); 6 a value check failed.

const TIMEOUT_MS := 20000
const EPS := 1e-4
const CAMERA_AT := Vector3(8, 12, 16)
const LOOK_AT := Vector3(8, 0, 6)

var walk = null
var _started_ms: int = 0
var _done: bool = false
var _log: Array = []       # every move receipt seen, in revision order
var _logged_rev: int = 0

func _initialize() -> void:
	if not ClassDB.class_exists("CrpgSession"):
		printerr("walk.gd: class CrpgSession is missing")
		quit(2)
		return
	var scene: PackedScene = load("res://scenes/Walk.tscn")
	if scene == null:
		_quit(6, "res://scenes/Walk.tscn did not load")
		return
	walk = scene.instantiate()
	root.add_child(walk)
	_started_ms = Time.get_ticks_msec()
	_run()

# ---- failure and check helpers (each quits once and marks the run done) ----

func _quit(code: int, message: String) -> void:
	if _done:
		return
	_done = true
	printerr("walk.gd FAIL: ", message)
	quit(code)

func _want(label: String, ok: bool, detail: String) -> void:
	if not ok:
		_quit(6, "%s: %s" % [label, detail])

func _want_code(label: String, got: int, want: int) -> void:
	if got != want:
		var text := ""
		if walk != null and walk.session != null:
			text = walk.session.last_error()
		_quit(3, "%s = %d, want %d (%s)" % [label, got, want, text])

func _near(a: Vector3, b: Vector3) -> bool:
	return absf(a.x - b.x) <= EPS and absf(a.y - b.y) <= EPS and absf(a.z - b.z) <= EPS

# P(i): proxy_<i>'s position.
func _p(i: int) -> Vector3:
	var node: Node3D = walk.get_node("Proxies").get_node_or_null("proxy_%d" % i)
	if node == null:
		_quit(6, "proxy_%d is missing" % i)
		return Vector3(1e9, 1e9, 1e9)
	return node.position

# ---- frames, waiting and the receipt log ----

# Records the move receipts of a revision not yet logged.
func _observe() -> void:
	if walk.view == null:
		return
	var rev: int = walk.view.revision()
	if rev != _logged_rev:
		_logged_rev = rev
		for receipt in walk.view.move_receipts():
			_log.append(receipt)

# One frame: a short sleep, the frame, then the codes check and the log.
func _frame(label: String) -> bool:
	if _done:
		return false
	if Time.get_ticks_msec() - _started_ms >= TIMEOUT_MS:
		_quit(4, "%s: not reached within 20 s" % label)
		return false
	OS.delay_msec(5)
	await process_frame
	if walk.open_code != 0 or walk.stop_code != 0:
		_quit(3, "%s: open_code=%d stop_code=%d" % [label, walk.open_code, walk.stop_code])
		return false
	_observe()
	return not _done

# Waits (one frame at a time) until `cond` is true; false after quitting.
func _wait(label: String, cond: Callable) -> bool:
	while not _done:
		if not await _frame(label):
			return false
		if walk.view != null and cond.call():
			return true
	return false

# Is there a receipt in _log[mark:] with the actor, outcome and (if >= 0) code?
func _has_receipt(mark: int, actor: int, outcome: String, code: int) -> bool:
	for i in range(mark, _log.size()):
		var r: Dictionary = _log[i]
		if r["actor"] == actor and r["outcome"] == outcome and (code < 0 or r["code"] == code):
			return true
	return false

func _frames(label: String, count: int) -> bool:
	for i in range(count):
		if not await _frame(label):
			return false
	return true

# ---- the cases ----

func _run() -> void:
	if not await _wk1():
		return
	if not await _wk2_wk3():
		return
	if not await _wk4():
		return
	if not await _wk5():
		return
	if not await _wk6():
		return
	if not await _wk7():
		return
	if not await _wk8():
		return
	if not await _wk9():
		return
	_wk10()
	if _done:
		return
	print("walk.gd OK")
	_done = true
	quit(0)

func _wk1() -> bool:
	var ready := func() -> bool:
		return walk.view.revision() >= 1 and walk.view.position_known() == PackedByteArray([1, 1, 1])
	if not await _wait("WK1 revision >= 1 and positions known", ready):
		return false
	_want("WK1 P(1)", _near(_p(1), Vector3(3, 0, 5)), str(_p(1)))
	_want("WK1 P(2)", _near(_p(2), Vector3(3, 0, 7)), str(_p(2)))
	_want("WK1 P(3)", _near(_p(3), Vector3(12, 0, 6)), str(_p(3)))
	return not _done

func _wk2_wk3() -> bool:
	# WK2
	_want("WK2 controlled", walk.controlled == PackedInt64Array([1, 2]), str(walk.controlled))
	_want("WK2 selected", walk.selected == 1, str(walk.selected))
	var marker: Node3D = walk.get_node("Marker")
	_want("WK2 marker visible", marker.visible, "hidden")
	_want("WK2 marker position", _near(marker.position, Vector3(3, 0.02, 5)), str(marker.position))
	if _done:
		return false
	# WK3
	var mark := _log.size()
	_want_code("WK3 click_ground(5,0,5)", walk.click_ground(Vector3(5, 0, 5)), 0)
	if _done:
		return false
	var xs: Array = []
	var arrived := func() -> bool:
		var v = walk.view
		xs.append(_p(1).x)
		return v.positions().slice(0, 3) == PackedInt64Array([327680, 0, 327680]) and v.moving()[0] == 0 and _near(_p(1), Vector3(5, 0, 5))
	if not await _wait("WK3 hero 1 arrives at (5,0,5)", arrived):
		return false
	var distinct: Array = []
	for i in range(xs.size()):
		if i > 0:
			_want("WK3 x never decreases", xs[i] >= xs[i - 1] - 1e-6, "%s then %s" % [xs[i - 1], xs[i]])
		if xs[i] > 3.0 and xs[i] < 5.0:
			var seen_before := false
			for d in distinct:
				if absf(d - xs[i]) <= 1e-6:
					seen_before = true
			if not seen_before:
				distinct.append(xs[i])
	_want("WK3 distinct values strictly between 3 and 5", distinct.size() >= 10, "%d: %s" % [distinct.size(), distinct])
	_want("WK3 accepted receipt", _has_receipt(mark, 1, "accepted", -1), str(_log))
	var first: Dictionary = {}
	if _log.size() > mark:
		first = _log[mark]
	_want("WK3 receipt seq and actor", first.get("seq", -1) == 1 and first.get("actor", -1) == 1, str(first))
	return not _done

func _wk4() -> bool:
	_want("WK4 P(1)", _near(_p(1), Vector3(5, 0, 5)), str(_p(1)))
	_want("WK4 P(2)", _near(_p(2), Vector3(3, 0, 7)), str(_p(2)))
	_want_code("WK4 click_ground(3.2,0,7.1)", walk.click_ground(Vector3(3.2, 0, 7.1)), walk.SELECTED)
	_want("WK4 selected", walk.selected == 2, str(walk.selected))
	if _done:
		return false
	var mark := _log.size()
	if not await _frames("WK4 20 quiet frames", 20):
		return false
	_want("WK4 nothing was sent", _log.size() == mark, str(_log.slice(mark)))
	# Control: 0.7 m from P(2) is a move, not a selection.
	_want_code("WK4 click_ground(3,0,7.7)", walk.click_ground(Vector3(3.0, 0, 7.7)), 0)
	if _done:
		return false
	var arrived := func() -> bool:
		return walk.view.positions().slice(3, 6) == PackedInt64Array([196608, 0, 504627]) and walk.view.moving()[1] == 0
	return await _wait("WK4 hero 2 arrives at (3,0,7.7)", arrived)

func _wk5() -> bool:
	_want_code("WK5 click_ground(3,0,10)", walk.click_ground(Vector3(3, 0, 10)), 0)
	if _done:
		return false
	var arrived := func() -> bool:
		return walk.view.positions().slice(3, 6) == PackedInt64Array([196608, 0, 655360]) and walk.view.moving()[1] == 0
	if not await _wait("WK5 hero 2 arrives at (3,0,10)", arrived):
		return false
	_want("WK5 P(2)", _near(_p(2), Vector3(3, 0, 10)), str(_p(2)))
	_want("WK5 P(1)", _near(_p(1), Vector3(5, 0, 5)), str(_p(1)))
	return not _done

func _wk6() -> bool:
	var mark := _log.size()
	_want_code("WK6 move_to(3,0,0,0)", walk.session.move_to(3, 0, 0, 0), 0)
	if _done:
		return false
	var rejected := func() -> bool:
		return _has_receipt(mark, 3, "rejected", 17)
	if not await _wait("WK6 goblin move rejected with 17", rejected):
		return false
	_want("WK6 P(3)", _near(_p(3), Vector3(12, 0, 6)), str(_p(3)))
	return not _done

func _wk7() -> bool:
	var mark := _log.size()
	_want_code("WK7 click_ground(14,0,2)", walk.click_ground(Vector3(14, 0, 2)), 0)
	if _done:
		return false
	var moving := func() -> bool:
		return _has_receipt(mark, 2, "accepted", -1) and walk.view.moving()[1] == 1
	if not await _wait("WK7 hero 2 accepted and moving", moving):
		return false
	var halt_mark := _log.size()
	_want_code("WK7 halt_selected", walk.halt_selected(), 0)
	if _done:
		return false
	var stopped := func() -> bool:
		return _has_receipt(halt_mark, 2, "accepted", -1) and walk.view.moving()[1] == 0
	if not await _wait("WK7 hero 2 halted", stopped):
		return false
	_want("WK7 stopped short of 14 m", walk.view.positions()[3] < 917504, str(walk.view.positions()))
	return not _done

func _wk8() -> bool:
	var size: Vector2 = root.get_visible_rect().size
	if size.x <= 0.0 or size.y <= 0.0:
		_quit(6, "WK8: the headless viewport has no size (%s): the contract says stop and report" % size)
		return false
	var hit = walk.ground_point(size / 2.0)
	_want("WK8 is a Vector3", typeof(hit) == TYPE_VECTOR3, str(hit))
	if _done:
		return false
	_want("WK8 y == 0.0", hit.y == 0.0, str(hit))
	_want("WK8 centre hits the look-at point", Vector2(hit.x - 8.0, hit.z - 6.0).length() < 0.01, str(hit))
	return not _done

func _event(button: int, pressed: bool, at: Vector2) -> InputEventMouseButton:
	var event := InputEventMouseButton.new()
	event.button_index = button
	event.pressed = pressed
	event.position = at
	return event

func _wk9() -> bool:
	var centre: Vector2 = root.get_visible_rect().size / 2.0
	var s: int = walk.selected
	var camera: Camera3D = walk.get_node("Camera3D")
	# (a) a left press on the ground moves the selected Hero.
	var before: int = walk.input_calls
	var mark := _log.size()
	walk._unhandled_input(_event(MOUSE_BUTTON_LEFT, true, centre))
	_want("WK9a input_calls", walk.input_calls == before + 1, "%d, was %d" % [walk.input_calls, before])
	if _done:
		return false
	var moving := func() -> bool:
		return _has_receipt(mark, s, "accepted", -1) and walk.view.moving()[s - 1] == 1
	if not await _wait("WK9a accepted and moving", moving):
		return false
	# (b) a release, a middle press and a key press change nothing.
	before = walk.input_calls
	walk._unhandled_input(_event(MOUSE_BUTTON_LEFT, false, centre))
	_want("WK9b left release unchanged", walk.input_calls == before, str(walk.input_calls))
	walk._unhandled_input(_event(MOUSE_BUTTON_MIDDLE, true, centre))
	_want("WK9b middle press unchanged", walk.input_calls == before, str(walk.input_calls))
	var key := InputEventKey.new()
	key.keycode = KEY_SPACE
	key.pressed = true
	walk._unhandled_input(key)
	_want("WK9b key press unchanged", walk.input_calls == before, str(walk.input_calls))
	if _done:
		return false
	# (c) a right press halts it.
	var halt_mark := _log.size()
	walk._unhandled_input(_event(MOUSE_BUTTON_RIGHT, true, centre))
	_want("WK9c input_calls", walk.input_calls == before + 1, "%d, was %d" % [walk.input_calls, before])
	if _done:
		return false
	var stopped := func() -> bool:
		return _has_receipt(halt_mark, s, "accepted", -1) and walk.view.moving()[s - 1] == 0
	if not await _wait("WK9c halted", stopped):
		return false
	# A halt stops short of the clicked point; a move to it would arrive.
	var target = walk.ground_point(centre)
	var stopped_at: Vector3 = _p(s)
	_want("WK9c stopped short of the clicked point", Vector2(stopped_at.x - target.x, stopped_at.z - target.z).length() > 0.5, "%s vs %s" % [stopped_at, target])
	# (d) rays that miss the ground.
	before = walk.input_calls
	camera.look_at_from_position(CAMERA_AT, CAMERA_AT + Vector3(0, 1, -1), Vector3.UP)
	_want("WK9d aimed up", walk.ground_point(centre) == null, str(walk.ground_point(centre)))
	walk._unhandled_input(_event(MOUSE_BUTTON_LEFT, true, centre))
	_want("WK9d aimed up, click unchanged", walk.input_calls == before, str(walk.input_calls))
	camera.look_at_from_position(CAMERA_AT, CAMERA_AT + Vector3(0, 0, -1), Vector3.UP)
	_want("WK9d aimed level", walk.ground_point(centre) == null, str(walk.ground_point(centre)))
	camera.look_at_from_position(CAMERA_AT, LOOK_AT, Vector3.UP)
	return not _done

# WK10 runs without a frame between a call and its check: the scene redraws
# every frame.
func _wk10() -> void:
	var q: Vector3 = _p(1)
	# (a) draw_rows: inconsistent sizes change nothing; a known row is drawn.
	walk.draw_rows(PackedInt64Array([1]), PackedByteArray([1]), PackedInt64Array([0, 0]))
	_want("WK10a short drawn", _p(1) == q, str(_p(1)))
	walk.draw_rows(PackedInt64Array([1]), PackedByteArray([]), PackedInt64Array([0, 0, 0]))
	_want("WK10a short known", _p(1) == q, str(_p(1)))
	walk.draw_rows(PackedInt64Array([1]), PackedByteArray([1]), PackedInt64Array([65536, 0, 131072, 0, 0, 0]))
	_want("WK10a long drawn", _p(1) == q, str(_p(1)))
	walk.draw_rows(PackedInt64Array([1]), PackedByteArray([1, 1]), PackedInt64Array([65536, 0, 131072]))
	_want("WK10a long known", _p(1) == q, str(_p(1)))
	walk.draw_rows(PackedInt64Array([1]), PackedByteArray([1]), PackedInt64Array([65536, 0, 131072]))
	_want("WK10a known row drawn", _near(_p(1), Vector3(1, 0, 2)), str(_p(1)))
	walk.draw_rows(PackedInt64Array([1]), PackedByteArray([0]), PackedInt64Array([0, 0, 0]))
	_want("WK10a unknown row kept", _near(_p(1), Vector3(1, 0, 2)), str(_p(1)))
	# (b) place_marker.
	var marker: Node3D = walk.get_node("Marker")
	walk.selected = 99
	walk.place_marker()
	_want("WK10b no such proxy hides the marker", marker.visible == false, "visible")
	walk.selected = 2
	walk.place_marker()
	var p2: Vector3 = _p(2)
	_want("WK10b marker follows P(2)", marker.visible and _near(marker.position, Vector3(p2.x, 0.02, p2.z)), "%s vs %s" % [marker.position, p2])
	# (c) leaving the tree closes the session.
	var held = walk.session
	root.remove_child(walk)
	_want("WK10c session is null", walk.session == null, str(walk.session))
	_want("WK10c the held session is closed", held.poll() == 1, str(held.poll()))
	_want("WK10c click_ground", walk.click_ground(Vector3(5, 0, 5)) == 1, "not 1")
	_want("WK10c halt_selected", walk.halt_selected() == 1, "not 1")
	walk.free()
	walk = null
