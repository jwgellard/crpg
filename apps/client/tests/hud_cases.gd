extends RefCounted
# T051: H1-H20 for the combat HUD, against a FakeView that has exactly the 14
# getters the HUD may read. run(root) returns "PASS" when every case returned
# "ok", otherwise "H<n>: <detail>" for the first that did not. A case that
# crashes makes its -> String function return "" (never "ok"), so it fails.
# H20 (press with main == null) is new in the T051-fix round; smoke.gd keeps its
# own real-session H20 check separately.

const HUD := "res://scripts/CombatHud.gd"
const MAIN := "res://scripts/Main.gd"
const OK := "ok"
const RED := Color(1.0, 0.35, 0.35)
const INT_KEYS := ["net_ids", "health", "max_health"]
const STR_KEYS := ["names", "sides", "option_labels"]
const BYTE_KEYS := ["health_known", "dead"]
const NODES := [
	["Panel", "PanelContainer", ""], ["Margin", "MarginContainer", "Panel"],
	["Column", "VBoxContainer", "Margin"], ["Round", "Label", "Column"],
	["Turn", "Label", "Column"], ["Health", "VBoxContainer", "Column"],
	["Options", "VBoxContainer", "Column"], ["Waiting", "Label", "Column"],
	["Log", "RichTextLabel", "Column"], ["Status", "Label", "Column"],
	["End", "Label", "Column"],
]
const COLUMN := ["Round", "Turn", "Health", "Options", "Waiting", "Log", "Status", "End"]
const VIEW_MEMBERS := [
	"revision", "events", "receipts", "net_ids", "names", "health", "max_health",
	"health_known", "dead", "round", "active", "ended", "sides", "option_labels",
]
const BANNED := [
	"placements", "option_targets", "submit_option", "poll", "ClassDB",
	"instantiate", "Time", "OS", "_process", "_physics_process", "_input",
	"_unhandled_input", "_gui_input",
]

class FakeView extends RefCounted:
	var _d: Dictionary

	func _init(d: Dictionary) -> void:
		_d = d

	func revision(): return _d.get("revision", 1)
	func active(): return _d.get("active", 0)
	func round(): return _d.get("round", 0)
	func ended(): return _d.get("ended", false)
	func events(): return _d.get("events", [])
	func receipts(): return _d.get("receipts", [])
	func net_ids(): return _d.get("net_ids", PackedInt64Array())
	func health(): return _d.get("health", PackedInt64Array())
	func max_health(): return _d.get("max_health", PackedInt64Array())
	func names(): return _d.get("names", PackedStringArray())
	func sides(): return _d.get("sides", PackedStringArray())
	func option_labels(): return _d.get("option_labels", PackedStringArray())
	func health_known(): return _d.get("health_known", PackedByteArray())
	func dead(): return _d.get("dead", PackedByteArray())

class FakeSession extends RefCounted:
	var text := ""

	func last_error() -> String:
		return text

class FakeMain extends Node:
	var session = FakeSession.new()
	var codes: Array = []
	var chosen: Array = []

	func choose(index: int) -> int:
		chosen.append(index)
		if codes.is_empty():
			return 0
		return codes.pop_front()

func run(root: Node):
	var script: Script = load(HUD)
	if script == null or not script.can_instantiate():
		return "H0: CombatHud.gd did not load"
	var cases: Array = [
		_h1, _h2, _h3, _h4, _h5, _h6, _h7, _h8, _h9, _h10,
		_h11, _h12, _h13, _h14, _h15, _h16, _h17, _h18, _h19, _h20,
	]
	for i in range(cases.size()):
		var detail = _run_case(root, cases[i])
		if not (detail is String) or detail != OK:
			return "H%d: %s" % [i + 1, "runtime error" if (detail == null or detail == "") else str(detail)]
	return "PASS"

func _run_case(root: Node, body: Callable):
	var host := FakeMain.new()
	root.add_child(host)
	var hud = _make_hud(host)
	var result = body.call(host, hud)
	root.remove_child(host)
	host.queue_free()
	return result

func _make_hud(host: FakeMain):
	var hud = load(HUD).new()
	host.add_child(hud)
	hud.main = host
	return hud

# ---- helpers -------------------------------------------------------------

func _a(over: Dictionary = {}) -> FakeView:
	var d := {
		"revision": 1, "net_ids": [4, 9], "names": ["Hero", "Goblin"],
		"sides": ["Hero", "Goblin"], "health": [10, 6], "max_health": [10, 6],
		"health_known": [1, 1], "dead": [0, 0], "active": 4, "round": 0,
		"ended": false, "option_labels": ["Strike -> Goblin", "End turn"],
		"events": [], "receipts": [],
	}
	d.merge(over, true)
	for key in d.keys():
		if key in INT_KEYS:
			d[key] = PackedInt64Array(d[key])
		elif key in STR_KEYS:
			d[key] = PackedStringArray(d[key])
		elif key in BYTE_KEYS:
			d[key] = PackedByteArray(d[key])
	return FakeView.new(d)

func _ar(a: int, t: int, o: String, dmg: int) -> Dictionary:
	return {"kind": "action_resolved", "actor": a, "target": t, "ability": "Strike",
		"ability_id": "x", "outcome": o, "damage": dmg}

func _ts(a: int, r: int) -> Dictionary:
	return {"kind": "turn_started", "actor": a, "round": r}

func _di(e: int) -> Dictionary:
	return {"kind": "died", "entity": e}

func _ee(r: int) -> Dictionary:
	return {"kind": "encounter_ended", "encounter": "x", "round": r}

func _rc(s: int, ok: bool, c: int) -> Dictionary:
	return {"seq": s, "applied": ok, "code": c}

func _n(hud, node_name: String):
	return hud.find_child(node_name, true, false)

func _texts(box) -> Array:
	var result: Array = []
	for child in box.get_children():
		result.append(child.text)
	return result

func _lines(hud) -> Array:
	var text: String = _n(hud, "Log").text
	if text == "":
		return []
	return Array(text.split("\n"))

func _eq(tag: String, got, want) -> String:
	if typeof(got) != typeof(want) or got != want:
		return "%s: got %s, want %s" % [tag, var_to_str(got), var_to_str(want)]
	return OK

func _first(results: Array) -> String:
	for r in results:
		if not (r is String) or r != OK:
			return "runtime error" if (r == null or r == "") else str(r)
	return OK

func _sync_all(hud, views: Array) -> void:
	for v in views:
		hud.sync(v)

func _check_built(hud) -> String:
	var res: Array = []
	for spec in NODES:
		var found = _n(hud, spec[0])
		if found == null:
			return "node %s is missing" % spec[0]
		var want_parent = hud if spec[2] == "" else _n(hud, spec[2])
		res.append(_eq(spec[0] + " class", found.get_class(), spec[1]))
		res.append(_eq(spec[0] + " parent", found.get_parent() == want_parent, true))
	var column = _n(hud, "Column")
	var names: Array = []
	for child in column.get_children():
		names.append(String(child.name))
	res.append(_eq("Column children", names, COLUMN))
	res.append(_eq("Hud children", hud.get_child_count(), 1))
	res.append(_eq("Panel children", _n(hud, "Panel").get_child_count(), 1))
	res.append(_eq("Margin children", _n(hud, "Margin").get_child_count(), 1))
	var panel = _n(hud, "Panel")
	res.append(_eq("Panel.position", panel.position, Vector2(12, 12)))
	res.append(_eq("Panel.minimum", panel.custom_minimum_size, Vector2(420, 0)))
	res.append(_eq("Panel anchors", [panel.anchor_left, panel.anchor_top, panel.anchor_right, panel.anchor_bottom], [0.0, 0.0, 0.0, 0.0]))
	for side in ["left", "right", "top", "bottom"]:
		res.append(_eq("margin_" + side, _n(hud, "Margin").get_theme_constant("margin_" + side), 12))
	res.append(_eq("separation", column.get_theme_constant("separation"), 6))
	var log = _n(hud, "Log")
	res.append(_eq("Log.minimum", log.custom_minimum_size, Vector2(0, 120)))
	res.append(_eq("Log.scroll_following", log.scroll_following, true))
	res.append(_eq("Log.bbcode_enabled", log.bbcode_enabled, false))
	res.append(_eq("Log.text", log.text, ""))
	for label in ["Round", "Turn", "Waiting", "Status", "End"]:
		res.append(_eq(label + ".text", _n(hud, label).text, ""))
	for hidden in ["Waiting", "Status", "End"]:
		res.append(_eq(hidden + ".visible", _n(hud, hidden).visible, false))
	res.append(_eq("Health children", _n(hud, "Health").get_child_count(), 0))
	res.append(_eq("Options children", _n(hud, "Options").get_child_count(), 0))
	res.append(_eq("lines", _lines(hud), []))
	res.append(_eq("_lines", hud._lines.size(), 0))
	res.append(_eq("_logged_revision", hud._logged_revision, -1))
	return _first(res)

# ---- cases ---------------------------------------------------------------

func _h1(_host, hud) -> String:
	var d := _check_built(hud)
	if d != OK:
		return d
	hud.sync(_a())
	return _eq("control Round.text", _n(hud, "Round").text, "Round 1")

func _h2(_host, hud) -> String:
	var round_label = _n(hud, "Round")
	var turn = _n(hud, "Turn")
	var res: Array = []
	hud.sync(_a())
	res.append(_eq("Round r0", round_label.text, "Round 1"))
	res.append(_eq("Turn hero", turn.text, "Turn: Hero"))
	hud.sync(_a({"revision": 2, "round": 2, "active": 9}))
	res.append(_eq("Round r2", round_label.text, "Round 3"))
	res.append(_eq("Turn goblin", turn.text, "Turn: Goblin"))
	hud.sync(_a({"revision": 3, "active": 0}))
	res.append(_eq("Turn id 0", turn.text, "Turn: -"))
	hud.sync(_a({"revision": 4, "active": 77}))
	res.append(_eq("Turn id 77", turn.text, "Turn: -"))
	return _first(res)

func _h3(_host, hud) -> String:
	var health = _n(hud, "Health")
	var res: Array = []
	hud.sync(_a())
	res.append(_eq("known", _texts(health), ["Hero: 10/10", "Goblin: 6/6"]))
	res.append(_eq("known count", health.get_child_count(), 2))
	hud.sync(_a({"revision": 2, "health": [10, 0], "max_health": [10, 0], "health_known": [1, 0]}))
	res.append(_eq("unknown", _texts(health), ["Hero: 10/10", "Goblin: ?"]))
	res.append(_eq("unknown count", health.get_child_count(), 2))
	hud.sync(_a({"revision": 3, "health": [10, 0], "max_health": [10, 6], "dead": [0, 1]}))
	res.append(_eq("dead", _texts(health), ["Hero: 10/10", "Goblin: 0/6 (dead)"]))
	res.append(_eq("dead count", health.get_child_count(), 2))
	hud.sync(_a({"revision": 4, "health": [10, 0], "health_known": [1, 0], "dead": [0, 1]}))
	res.append(_eq("unknown dead", _texts(health), ["Hero: 10/10", "Goblin: ? (dead)"]))
	return _first(res)

func _h4(_host, hud) -> String:
	var options = _n(hud, "Options")
	var res: Array = []
	hud.sync(_a())
	res.append(_eq("labels", _texts(options), ["Strike -> Goblin", "End turn"]))
	for child in options.get_children():
		res.append(_eq("is Button", child is Button, true))
		res.append(_eq("disabled", child.disabled, false))
	var b0 = options.get_child(0)
	hud.sync(_a({"revision": 2, "option_labels": ["End turn"]}))
	res.append(_eq("rebuilt", _texts(options), ["End turn"]))
	res.append(_eq("old parent is null", b0.get_parent() == null, true))
	res.append(_eq("old queued", b0.is_queued_for_deletion(), true))
	hud.sync(_a({"revision": 3, "option_labels": []}))
	res.append(_eq("none", options.get_child_count(), 0))
	return _first(res)

func _h5(host, hud) -> String:
	var options = _n(hud, "Options")
	var res: Array = []
	hud.sync(_a())
	options.get_child(1).pressed.emit()
	options.get_child(0).pressed.emit()
	res.append(_eq("chosen", host.chosen, [1, 0]))
	res.append(_eq("disabled 0", options.get_child(0).disabled, false))
	res.append(_eq("disabled 1", options.get_child(1).disabled, false))
	res.append(_eq("count", options.get_child_count(), 2))
	res.append(_eq("Status.visible", _n(hud, "Status").visible, false))
	res.append(_eq("lines", _lines(hud), []))
	return _first(res)

func _h6(host, hud) -> String:
	var status = _n(hud, "Status")
	var res: Array = []
	hud.sync(_a())
	var first = _n(hud, "Options").get_child(0)
	host.codes = [4]
	first.pressed.emit()
	res.append(_eq("4 lines", _lines(hud), []))
	res.append(_eq("4 status", status.visible, false))
	host.codes = [5]
	first.pressed.emit()
	res.append(_eq("5 lines", _lines(hud), []))
	res.append(_eq("5 status", status.visible, false))
	host.codes = [30]
	host.session.text = "INTENT_REFUSED: link full"
	first.pressed.emit()
	res.append(_eq("30 lines", _lines(hud), ["INTENT_REFUSED: link full"]))
	res.append(_eq("30 status", status.visible, false))
	host.codes = [6]
	host.session.text = "NO_SUCH_OPTION: index is out of range"
	first.pressed.emit()
	res.append(_eq("6 status", status.visible, true))
	res.append(_eq("6 text", status.text, "NO_SUCH_OPTION: index is out of range"))
	res.append(_eq("6 color", status.get_theme_color("font_color"), RED))
	res.append(_eq("6 last line", _lines(hud).back(), "ERROR: NO_SUCH_OPTION: index is out of range"))
	host.session = null
	host.codes = [30]
	first.pressed.emit()
	res.append(_eq("30 null last line", _lines(hud).back(), "NOT_OPEN"))
	res.append(_eq("30 null status text", status.text, "NO_SUCH_OPTION: index is out of range"))
	res.append(_eq("30 null status visible", status.visible, true))
	host.codes = [1]
	first.pressed.emit()
	res.append(_eq("null text", status.text, "NOT_OPEN"))
	res.append(_eq("null last line", _lines(hud).back(), "ERROR: NOT_OPEN"))
	res.append(_eq("chosen", host.chosen, [0, 0, 0, 0, 0, 0]))
	return _first(res)

func _h7(_host, hud) -> String:
	var waiting = _n(hud, "Waiting")
	var res: Array = []
	hud.sync(_a())
	res.append(_eq("control visible", waiting.visible, false))
	res.append(_eq("control text", waiting.text, "Waiting for Hero..."))
	hud.sync(_a({"revision": 2, "option_labels": [], "active": 9}))
	res.append(_eq("goblin visible", waiting.visible, true))
	res.append(_eq("goblin text", waiting.text, "Waiting for Goblin..."))
	res.append(_eq("goblin buttons", _n(hud, "Options").get_child_count(), 0))
	hud.sync(_a({"revision": 3, "option_labels": [], "active": 0}))
	res.append(_eq("anon visible", waiting.visible, true))
	res.append(_eq("anon text", waiting.text, "Waiting..."))
	hud.sync(_a({"revision": 4, "option_labels": [], "ended": true}))
	res.append(_eq("ended visible", waiting.visible, false))
	res.append(_eq("ended text", waiting.text, "Waiting for Hero..."))
	return _first(res)

func _h8(_host, hud) -> String:
	hud.sync(_a({"revision": 2, "events": [_ar(4, 9, "success", 2), _ts(9, 3), _di(9), _ee(3)]}))
	return _eq("lines", _lines(hud), [
		"Hero | Strike -> Goblin: hit (success), 2 damage",
		"Turn: Goblin (round 4)",
		"Goblin dies.",
		"Encounter ended (round 4)",
	])

func _h9(_host, hud) -> String:
	var res: Array = []
	hud.sync(_a({"revision": 2, "events": [
		_ar(4, 9, "success", 0), _ar(4, 9, "critical_success", 1),
		_ar(9, 4, "failure", 2), _ar(9, 4, "critical_failure", 3),
		_ar(4, 9, "custom:7", 4), {"kind": "mystery"},
	]}))
	res.append(_eq("lines", _lines(hud), [
		"Hero | Strike -> Goblin: hit (success), 0 damage",
		"Hero | Strike -> Goblin: hit (critical success), 1 damage",
		"Goblin | Strike -> Hero: miss (failure), 2 damage",
		"Goblin | Strike -> Hero: miss (critical failure), 3 damage",
		"Hero | Strike -> Goblin: outcome (custom:7), 4 damage",
		"mystery",
	]))
	hud.clear()
	hud.sync(_a({"revision": 3, "events": [_ar(4, 9, "some_odd_thing", 5)]}))
	res.append(_eq("underscores", _lines(hud), ["Hero | Strike -> Goblin: outcome (some odd thing), 5 damage"]))
	return _first(res)

func _h10(_host, hud) -> String:
	var v5 := {"revision": 5, "events": [_ar(4, 9, "success", 2)], "receipts": [_rc(1, false, 18)]}
	var pair := [
		"Hero | Strike -> Goblin: hit (success), 2 damage",
		"Rejected: no longer legal (code 18)",
	]
	var res: Array = []
	hud.sync(_a(v5))
	res.append(_eq("first", _lines(hud), pair))
	hud.sync(_a(v5))
	res.append(_eq("same revision", _lines(hud), pair))
	var next := v5.duplicate()
	next["revision"] = 6
	hud.sync(_a(next))
	res.append(_eq("new revision", _lines(hud), pair + pair))
	return _first(res)

func _h11(_host, hud) -> String:
	hud.sync(_a({"revision": 2, "receipts": [
		_rc(1, true, 0), _rc(2, false, 18), _rc(3, false, 17), _rc(4, false, 2), _rc(5, false, 9),
	]}))
	return _eq("lines", _lines(hud), [
		"Rejected: no longer legal (code 18)",
		"Rejected: not allowed (code 17)",
		"Rejected: refused by the server (code 2)",
		"Rejected: refused by the server (code 9)",
	])

func _h12(host, hud) -> String:
	var res: Array = []
	for i in range(250):
		hud.show_notice("L%d" % i)
		var seen := _lines(hud)
		res.append(_eq("step %d size" % i, seen.size(), mini(i + 1, 200)))
		res.append(_eq("step %d first" % i, seen[0], "L%d" % maxi(0, i - 199)))
	var lines := _lines(hud)
	res.append(_eq("250 size", lines.size(), 200))
	res.append(_eq("250 first", lines[0], "L50"))
	res.append(_eq("250 last", lines[199], "L249"))
	res.append(_eq("250 text", _n(hud, "Log").text, "\n".join(PackedStringArray(lines))))
	var other = _make_hud(host)
	for i in range(200):
		other.show_notice("L%d" % i)
	lines = _lines(other)
	res.append(_eq("200 size", lines.size(), 200))
	res.append(_eq("200 first", lines[0], "L0"))
	other.sync(_a({"revision": 2, "events": [_di(4), _di(9)]}))
	lines = _lines(other)
	res.append(_eq("event size", lines.size(), 200))
	res.append(_eq("event first", lines[0], "L2"))
	res.append(_eq("event 198", lines[198], "Hero dies."))
	res.append(_eq("event 199", lines[199], "Goblin dies."))
	res.append(_eq("event text", _n(other, "Log").text, "\n".join(PackedStringArray(lines))))
	return _first(res)

func _h13(_host, hud) -> String:
	hud.show_notice("[b]Goblin[/b]")
	var log = _n(hud, "Log")
	return _first([
		_eq("text", log.text, "[b]Goblin[/b]"),
		_eq("parsed", log.get_parsed_text(), "[b]Goblin[/b]"),
	])

func _h14(_host, hud) -> String:
	var res: Array = []
	var end = _n(hud, "End")
	var base := {"revision": 2, "round": 4, "health": [10, 0], "dead": [0, 1], "option_labels": []}
	var live := base.duplicate()
	live["ended"] = false
	hud.sync(_a(live))
	res.append(_eq("control visible", end.visible, false))
	res.append(_eq("control text", end.text, ""))
	res.append(_eq("control waiting", _n(hud, "Waiting").text, "Waiting for Hero..."))
	res.append(_eq("control waiting visible", _n(hud, "Waiting").visible, true))
	var done := base.duplicate()
	done["ended"] = true
	hud.sync(_a(done))
	res.append(_eq("visible", end.visible, true))
	res.append(_eq("text", end.text, "Encounter ended\nRounds played: 5\nWinner: Hero"))
	res.append(_eq("waiting", _n(hud, "Waiting").visible, false))
	res.append(_eq("health", _texts(_n(hud, "Health"))[1], "Goblin: 0/6 (dead)"))
	hud.sync(_a({"revision": 3, "ended": true, "round": 4, "dead": [1, 0], "option_labels": []}))
	res.append(_eq("goblin wins", end.text, "Encounter ended\nRounds played: 5\nWinner: Goblin"))
	return _first(res)

func _h15(_host, hud) -> String:
	var rows := {
		"net_ids": [4, 9, 12], "names": ["Hero", "Goblin", "Ally"],
		"sides": ["Hero", "Goblin", "Hero"], "health_known": [1, 1, 1],
		"health": [1, 1, 1], "max_health": [1, 1, 1], "ended": true, "round": 0,
	}
	var res: Array = []
	var cases := [
		[[1, 0, 0], [1, 1, 1], "draw"], [[1, 1, 0], [1, 1, 1], "Hero"],
		[[1, 1, 1], [1, 1, 1], "draw"], [[0, 1, 0], [1, 1, 0], "Hero"],
		[[1, 1, 0], [1, 1, 0], "Hero"],
	]
	var revision := 2
	for c in cases:
		var over := rows.duplicate()
		over["revision"] = revision
		over["dead"] = c[0]
		over["health_known"] = c[1]
		hud.sync(_a(over))
		res.append(_eq("dead %s" % str(c[0]), _n(hud, "End").text,
			"Encounter ended\nRounds played: 1\nWinner: %s" % c[2]))
		revision += 1
	return _first(res)

func _h16(_host, hud) -> String:
	var end = _n(hud, "End")
	var res: Array = []
	hud.sync(_a({"revision": 7, "ended": true, "dead": [0, 1], "events": [_ee(2)]}))
	var text: String = end.text
	res.append(_eq("visible", end.visible, true))
	res.append(_eq("text", text, "Encounter ended\nRounds played: 1\nWinner: Hero"))
	res.append(_eq("lines", _lines(hud), ["Encounter ended (round 3)"]))
	hud.sync(_a({"revision": 8, "ended": true, "dead": [0, 1]}))
	res.append(_eq("persist visible", end.visible, true))
	res.append(_eq("persist text", end.text, text))
	res.append(_eq("persist lines", _lines(hud), ["Encounter ended (round 3)"]))
	hud.sync(_a({"revision": 9}))
	res.append(_eq("hidden", end.visible, false))
	res.append(_eq("hidden text", end.text, ""))
	return _first(res)

func _h17(_host, hud) -> String:
	var res: Array = []
	hud.sync(_a({"events": [_di(9)]}))
	var waiting = _n(hud, "Waiting")
	var status = _n(hud, "Status")
	var end = _n(hud, "End")
	var before := [_n(hud, "Round").text, _n(hud, "Turn").text, _texts(_n(hud, "Health")),
		_texts(_n(hud, "Options")), _lines(hud),
		waiting.text, waiting.visible, status.text, status.visible,
		end.text, end.visible, hud._logged_revision]
	hud.sync(null)
	var after := [_n(hud, "Round").text, _n(hud, "Turn").text, _texts(_n(hud, "Health")),
		_texts(_n(hud, "Options")), _lines(hud),
		waiting.text, waiting.visible, status.text, status.visible,
		end.text, end.visible, hud._logged_revision]
	res.append(_eq("before", before, ["Round 1", "Turn: Hero", ["Hero: 10/10", "Goblin: 6/6"],
		["Strike -> Goblin", "End turn"], ["Goblin dies."],
		"Waiting for Hero...", false, "", false, "", false, 1]))
	res.append(_eq("null sync", after, before))
	hud.sync(FakeView.new({"revision": 2}))
	res.append(_eq("Health", _n(hud, "Health").get_child_count(), 0))
	res.append(_eq("Options", _n(hud, "Options").get_child_count(), 0))
	res.append(_eq("Turn", _n(hud, "Turn").text, "Turn: -"))
	res.append(_eq("Waiting text", _n(hud, "Waiting").text, "Waiting..."))
	res.append(_eq("Waiting visible", _n(hud, "Waiting").visible, true))
	return _first(res)

func _h18(_host, hud) -> String:
	var res: Array = []
	var e := {"revision": 3, "ended": true, "dead": [0, 1], "option_labels": [], "events": [_di(9)]}
	hud.show_error("X")
	hud.sync(_a(e))
	res.append(_eq("before lines", _lines(hud), ["ERROR: X", "Goblin dies."]))
	res.append(_eq("before End", _n(hud, "End").visible, true))
	res.append(_eq("Status visible", _n(hud, "Status").visible, true))
	res.append(_eq("Status text", _n(hud, "Status").text, "X"))
	res.append(_eq("Status color", _n(hud, "Status").get_theme_color("font_color"), RED))
	hud.sync(_a(e))
	res.append(_eq("gate holds", _lines(hud), ["ERROR: X", "Goblin dies."]))
	hud.clear()
	res.append(_check_built(hud))
	res.append(_eq("override", _n(hud, "Status").has_theme_color_override("font_color"), false))
	hud.sync(_a(e))
	res.append(_eq("re-log", _lines(hud), ["Goblin dies."]))
	res.append(_eq("End again", _n(hud, "End").visible, true))
	return _first(res)

func _scan(path: String) -> Dictionary:
	var kept := PackedStringArray()
	for line in FileAccess.get_file_as_string(path).split("\n"):
		var cut: int = line.find("#")
		kept.append(line if cut < 0 else line.substr(0, cut))
	var source := "\n".join(kept)
	var out := {"view": [], "main": [], "session": [], "tokens": []}
	for spec in [["view", r"view\.([a-z_]+)\("], ["main", r"main\.([a-z_]+)"],
			["session", r"session\.([a-z_]+)\("], ["tokens", r"[A-Za-z_][A-Za-z0-9_]*"]]:
		var re := RegEx.new()
		re.compile(spec[1])
		for m in re.search_all(source):
			out[spec[0]].append(m.get_string(1 if spec[0] != "tokens" else 0))
	return out

func _h19(_host, _hud) -> String:
	var res: Array = []
	var got := _scan(HUD)
	for member in got["view"]:
		res.append(_eq("view." + member, member in VIEW_MEMBERS, true))
	for member in ["events", "receipts", "revision"]:
		res.append(_eq("uses " + member, member in got["view"], true))
	for member in got["main"]:
		res.append(_eq("main." + member, member in ["choose", "session"], true))
	for member in got["session"]:
		res.append(_eq("session." + member, member, "last_error"))
	for token in got["tokens"]:
		res.append(_eq("banned " + token, token in BANNED, false))
	var control := _scan(MAIN)
	for token in ["submit_option", "poll", "ClassDB", "OS", "_process"]:
		res.append(_eq("control " + token, token in control["tokens"], true))
	return _first(res)

func _h20(host, hud) -> String:
	var res: Array = []
	hud.sync(_a())
	hud.main = null
	var options = _n(hud, "Options")
	options.get_child(0).pressed.emit()
	res.append(_eq("log", _lines(hud), []))
	res.append(_eq("status visible", _n(hud, "Status").visible, false))
	res.append(_eq("status text", _n(hud, "Status").text, ""))
	res.append(_eq("chosen", host.chosen, []))
	res.append(_eq("options count", options.get_child_count(), 2))
	return _first(res)
