extends SceneTree
# M1-M5 (T151c), M6-M7 (fix round A): the v4 movement surface of CrpgSession and
# CrpgView, headless.
# It copies combat_basic (BASIC) to user://t151_move/ and inserts the T037e
# movement rules into the copy's minimal-d6 ruleset (MOVE), because no ruleset
# under campaigns/fixtures may declare movement. Every case then drives one
# CrpgSession from a coroutine, one poll per frame, under a 20 s timeout.
# Run: Godot4 --headless --path apps/client -s res://tests/movement.gd
# Exit 0 pass; 2 class missing; 3 a call returned an unexpected code; 4 timeout;
# 5 the MOVE edit or the copy failed; 6 a value check failed.

const TIMEOUT_MS := 20000
const MOVE_LINE := "  \"movement\": {\"speed\": {\"type\": \"constant\", \"value\": 65536}, \"turn\": {\"pool\": \"0000000000000000000000000P\", \"step\": {\"type\": \"constant\", \"value\": 65536}, \"step_cost\": 1}},"
const NEEDLE := "  \"name\": \"fixture.minimal-d6\","

var _started_ms: int = 0
var _done: bool = false

func _initialize() -> void:
	if not ClassDB.class_exists("CrpgSession"):
		printerr("movement.gd: class CrpgSession is missing")
		quit(2)
		return
	_started_ms = Time.get_ticks_msec()
	_run()

# ---- failure and check helpers (each quits once and marks the run done) ----

func _quit(code: int, message: String) -> void:
	if _done:
		return
	_done = true
	printerr("movement.gd FAIL: ", message)
	quit(code)

func _want_code(label: String, got: int, want: int, session) -> void:
	if got != want:
		_quit(3, "%s = %d, want %d (%s)" % [label, got, want, session.last_error()])

func _want(label: String, ok: bool, detail: String) -> void:
	if not ok:
		_quit(6, "%s: %s" % [label, detail])

# ---- the MOVE fixture ----

func _basic_dir() -> String:
	return ProjectSettings.globalize_path("res://").path_join("../../campaigns/fixtures/combat_basic").simplify_path()

func _remove_tree(path: String) -> void:
	var dir := DirAccess.open(path)
	if dir == null:
		return
	for sub in dir.get_directories():
		_remove_tree(path.path_join(sub))
	for file in dir.get_files():
		DirAccess.remove_absolute(path.path_join(file))
	DirAccess.remove_absolute(path)

func _copy_tree(from: String, to: String) -> bool:
	if DirAccess.make_dir_recursive_absolute(to) != OK:
		return false
	var dir := DirAccess.open(from)
	if dir == null:
		return false
	for sub in dir.get_directories():
		if not _copy_tree(from.path_join(sub), to.path_join(sub)):
			return false
	for file in dir.get_files():
		if DirAccess.copy_absolute(from.path_join(file), to.path_join(file)) != OK:
			return false
	return true

# Builds MOVE; returns its directory, or "" after quitting 5.
func _build_move() -> String:
	var target := ProjectSettings.globalize_path("user://t151_move")
	_remove_tree(target)
	if not _copy_tree(_basic_dir(), target):
		_quit(5, "could not copy BASIC to %s" % target)
		return ""
	var ruleset := target.path_join("rulesets/minimal-d6.json")
	var text := FileAccess.get_file_as_string(ruleset)
	if text.count(NEEDLE) != 1:
		_quit(5, "the name line appears %d times in %s, want 1" % [text.count(NEEDLE), ruleset])
		return ""
	var out := FileAccess.open(ruleset, FileAccess.WRITE)
	if out == null:
		_quit(5, "could not write %s" % ruleset)
		return ""
	out.store_string(text.replace(NEEDLE, MOVE_LINE + "\n" + NEEDLE))
	out.close()
	return target

# ---- polling ----

# Polls once per frame until `done` (a Callable taking the view) returns true.
# Returns false after quitting (bad poll code, timeout, or another failure).
func _poll_until(label: String, session, done: Callable) -> bool:
	while not _done:
		var code: int = session.poll()
		if code != 0:
			_quit(3, "%s: poll = %d (%s)" % [label, code, session.last_error()])
			return false
		if done.call(session.view()):
			return true
		if Time.get_ticks_msec() - _started_ms >= TIMEOUT_MS:
			_quit(4, "%s: not reached within 20 s" % label)
			return false
		OS.delay_msec(5)
		await process_frame
	return false

func _revision_one(view) -> bool:
	return view.revision() >= 1

func _has_options(view) -> bool:
	return view.option_labels().size() > 0

func _has_move_receipt(view) -> bool:
	return view.move_receipts().size() > 0

# ---- the cases ----

func _run() -> void:
	var move_dir := _build_move()
	if _done:
		return
	var basic_dir := _basic_dir()
	if not await _case_m1_m2(move_dir):
		return
	if not await _case_m3_m4(move_dir):
		return
	_case_m5(basic_dir)
	if _done:
		return
	if not await _case_m6(move_dir):
		return
	if not await _case_m7(move_dir):
		return
	print("movement.gd OK")
	_done = true
	quit(0)

# M1 and M2: one MOVE session, the Goblin (entity 2, net id 2) is the player.
func _case_m1_m2(move_dir: String) -> bool:
	var s = ClassDB.instantiate("CrpgSession")
	_want_code("M1 open_v4", s.open_v4(move_dir, "", "Goblin", 1), 0, s)
	if _done:
		return false
	if not await _poll_until("M1 revision >= 1", s, _revision_one):
		return false
	var v = s.view()
	_want("M1 positions", v.positions() == PackedInt64Array([0, 0, 0, 0, 0, 0]), str(v.positions()))
	_want("M1 position_known", v.position_known() == PackedByteArray([1, 1]), str(v.position_known()))
	_want("M1 moving", v.moving() == PackedByteArray([0, 0]), str(v.moving()))
	_want("M1 move_receipts", v.move_receipts().size() == 0, str(v.move_receipts()))
	if _done:
		return false

	# M2. The options are read between cycles, so wait until the view has them.
	if not await _poll_until("M2 options", s, _has_options):
		return false
	var rev: int = s.view().revision()
	_want_code("M2 declare_move(rev + 1) control", s.declare_move(rev + 1, 32768, 0, 32768), 4, s)
	_want_code("M2 declare_move(rev)", s.declare_move(rev, 32768, 0, 32768), 0, s)
	if _done:
		return false
	var moved := {"points": null}
	var saw_moved := func(view) -> bool:
		for event in view.events():
			if event["kind"] == "moved":
				moved["points"] = event["points"]
				moved["entity"] = event["entity"]
				moved["positions"] = view.positions()
				return true
		return false
	if not await _poll_until("M2 moved event", s, saw_moved):
		return false
	_want("M2 entity", moved["entity"] == 2, str(moved["entity"]))
	_want("M2 points", moved["points"] == PackedInt64Array([0, 0, 0, 32768, 0, 32768]), str(moved["points"]))
	var tail: PackedInt64Array = moved["positions"]
	_want("M2 positions tail", tail.size() == 6 and tail.slice(3) == PackedInt64Array([32768, 0, 32768]), str(tail))
	s.close()
	return not _done

# M3 and M4: a fresh MOVE session; the lane-1 receipts are seq 1 then seq 2.
func _case_m3_m4(move_dir: String) -> bool:
	var s = ClassDB.instantiate("CrpgSession")
	_want_code("M3 open_v4", s.open_v4(move_dir, "", "Goblin", 1), 0, s)
	if _done:
		return false
	if not await _poll_until("M3 revision >= 1", s, _revision_one):
		return false
	_want_code("M3 move_to(-1) control", s.move_to(-1, 0, 0, 0), 3, s)
	_want_code("M3 move_to(2)", s.move_to(2, 0, 0, 0), 0, s)
	if _done:
		return false
	if not await _poll_until("M3 receipt", s, _has_move_receipt):
		return false
	_check_receipt("M3", s.view().move_receipts(), 1)
	if _done:
		return false
	_want("M3 start_tick", s.view().move_receipts()[0]["start_tick"] == 0, str(s.view().move_receipts()))

	# A view keeps its receipts until the next revision, so wait for a later one.
	var m3_revision: int = s.view().revision()
	_want_code("M4 halt(2)", s.halt(2), 0, s)
	if _done:
		return false
	var later_receipt := func(view) -> bool:
		return view.revision() > m3_revision and view.move_receipts().size() > 0
	if not await _poll_until("M4 receipt", s, later_receipt):
		return false
	_check_receipt("M4", s.view().move_receipts(), 2)
	s.close()
	return not _done

func _check_receipt(label: String, receipts: Array, seq: int) -> void:
	_want(label + " receipt count", receipts.size() == 1, str(receipts))
	if _done:
		return
	var r: Dictionary = receipts[0]
	_want(label + " seq", r["seq"] == seq, str(r))
	_want(label + " actor", r["actor"] == 2, str(r))
	_want(label + " outcome", r["outcome"] == "rejected", str(r))
	_want(label + " code", r["code"] == 22, str(r))

# M5: a v2 session carries zeroed position columns and refuses move calls.
func _case_m5(basic_dir: String) -> void:
	var s = ClassDB.instantiate("CrpgSession")
	_want_code("M5 open", s.open(basic_dir, "", "", 1), 0, s)
	if _done:
		return
	_want_code("M5 poll", s.poll(), 0, s)
	if _done:
		return
	var v = s.view()
	_want("M5 positions size", v.positions().size() == 6, str(v.positions()))
	_want("M5 position_known", v.position_known() == PackedByteArray([0, 0]), str(v.position_known()))
	_want_code("M5 move_to on v2", s.move_to(2, 0, 0, 0), 3, s)
	s.close()

func _has_two_move_receipts(view) -> bool:
	return view.move_receipts().size() >= 2

# M6 (fix round A): a turn-based move to an asymmetric destination (x != z), so
# an x/z swap in declare_move shows. A fresh session, because M2's move spent the
# turn pool (T037 K2: one step of 65,536 pays a route of 46,341, cost 1).
# Reasoned from T037 K2 (tasks/T037.md:1300-1306): the route is straight, from
# the start (0,0,0) to the destination, and the destination is within one step
# (length 36,636 < 65,536), so the moved event ends at the destination.
func _case_m6(move_dir: String) -> bool:
	var s = ClassDB.instantiate("CrpgSession")
	_want_code("M6 open_v4", s.open_v4(move_dir, "", "Goblin", 1), 0, s)
	if _done:
		return false
	if not await _poll_until("M6 revision >= 1", s, _revision_one):
		return false
	if not await _poll_until("M6 options", s, _has_options):
		return false
	var rev: int = s.view().revision()
	_want_code("M6 declare_move(rev)", s.declare_move(rev, 32768, 0, 16384), 0, s)
	if _done:
		return false
	var moved := {"points": null}
	var saw_moved := func(view) -> bool:
		for event in view.events():
			if event["kind"] == "moved":
				moved["points"] = event["points"]
				moved["entity"] = event["entity"]
				moved["positions"] = view.positions()
				return true
		return false
	if not await _poll_until("M6 moved event", s, saw_moved):
		return false
	_want("M6 entity", moved["entity"] == 2, str(moved["entity"]))
	var points: PackedInt64Array = moved["points"]
	_want("M6 points", points == PackedInt64Array([0, 0, 0, 32768, 0, 16384]), str(points))
	var tail: PackedInt64Array = moved["positions"]
	_want("M6 positions tail", tail.size() == 6 and tail.slice(3) == PackedInt64Array([32768, 0, 16384]), str(tail))
	s.close()
	return not _done

# M7 (fix round A): the lane-1 entry points' argument rules and receipt order.
# - move_to with x = COORD_LIMIT_RAW + 1 = 536870913 is 3 on any session, and halt
#   of a non-positive actor is 3 (B-B8 (1), T151.md:670-676; B11, :921-923). A
#   refused call consumes no seq (B-A9, :572-576).
# - Two lane-1 intents of one actor sent before any cycle fall in one decision
#   window: the earlier is Superseded, the last is decided (T037 D-6, D-7,
#   tasks/T037.md:770-781), here Rejected(WrongMode) 22 as in M3 (turn-based).
#   Receipts are released in seq order (D-3, :743-752) and the view lists them
#   oldest first (B-C2, T151.md:700-702); A4 (:778-780) pins "superseded" with
#   code 0 and start_tick 0.
func _case_m7(move_dir: String) -> bool:
	var s = ClassDB.instantiate("CrpgSession")
	_want_code("M7 open_v4", s.open_v4(move_dir, "", "Goblin", 1), 0, s)
	if _done:
		return false
	if not await _poll_until("M7 revision >= 1", s, _revision_one):
		return false
	_want_code("M7 move_to(x out of range)", s.move_to(2, 536870913, 0, 0), 3, s)
	_want_code("M7 halt(-1)", s.halt(-1), 3, s)
	_want_code("M7 halt(0)", s.halt(0), 3, s)
	if _done:
		return false
	_want_code("M7 move_to(2)", s.move_to(2, 32768, 0, 16384), 0, s)
	_want_code("M7 halt(2)", s.halt(2), 0, s)
	if _done:
		return false
	if not await _poll_until("M7 two receipts", s, _has_two_move_receipts):
		return false
	var receipts: Array = s.view().move_receipts()
	_want("M7 receipt count", receipts.size() == 2, str(receipts))
	if _done:
		return false
	var first: Dictionary = receipts[0]
	_want("M7 first seq", first["seq"] == 1, str(receipts))
	_want("M7 first actor", first["actor"] == 2, str(receipts))
	_want("M7 first outcome", first["outcome"] == "superseded", str(receipts))
	_want("M7 first code", first["code"] == 0, str(receipts))
	_want("M7 first start_tick", first["start_tick"] == 0, str(receipts))
	var second: Dictionary = receipts[1]
	_want("M7 second seq", second["seq"] == 2, str(receipts))
	_want("M7 second actor", second["actor"] == 2, str(receipts))
	_want("M7 second outcome", second["outcome"] == "rejected", str(receipts))
	_want("M7 second code", second["code"] == 22, str(receipts))
	_want("M7 second start_tick", second["start_tick"] == 0, str(receipts))
	s.close()
	return not _done
