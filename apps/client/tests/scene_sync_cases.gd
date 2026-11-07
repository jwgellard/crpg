extends RefCounted
# T048: S1-S13 for the SceneSync reconcile. run(host) returns "PASS" when every
# case passes, otherwise "S<n>: <detail>" for the first failure. A case that
# crashes returns "" (the default of a -> String function), never "PASS", so it
# fails. S14 (deferred frees) runs in tests/scene_sync.gd. Each case gets a
# fresh SceneSync under host; the freed and kept lists feed S14.

const SCENE_SYNC := "res://scripts/SceneSync.gd"
const TINT := Color(0.35, 0.35, 0.35)
const WHITE := Color(1.0, 1.0, 1.0)
const BANNED := [
	"session", "submit_option", "events", "receipts", "health", "max_health",
	"health_known", "revision", "active", "option_labels", "option_targets",
	"round", "ended", "placements", "Time", "OS",
]

var freed: Array = []
var kept: Array = []

class FakeView:
	var _ids: PackedInt64Array
	var _names: PackedStringArray
	var _sides: PackedStringArray
	var _dead: PackedByteArray

	func _init(ids: PackedInt64Array, names: PackedStringArray, sides: PackedStringArray, dead: PackedByteArray) -> void:
		_ids = ids
		_names = names
		_sides = sides
		_dead = dead

	func net_ids() -> PackedInt64Array:
		return _ids

	func names() -> PackedStringArray:
		return _names

	func sides() -> PackedStringArray:
		return _sides

	func dead() -> PackedByteArray:
		return _dead

func run(host: Node) -> String:
	var script: Script = load(SCENE_SYNC)
	if script == null:
		return "S0: SceneSync.gd did not load"
	var cases: Array = [
		["S1", _case_s1], ["S2", _case_s2], ["S3", _case_s3], ["S4", _case_s4],
		["S5", _case_s5], ["S6", _case_s6], ["S7", _case_s7], ["S8", _case_s8],
		["S9", _case_s9], ["S10", _case_s10], ["S11", _case_s11],
		["S12", _case_s12], ["S13", _case_s13],
	]
	for entry in cases:
		var detail = entry[1].call(host, script)
		if detail == null or detail == "":
			return "%s: runtime error" % entry[0]
		if detail != "PASS":
			return "%s: %s" % [entry[0], detail]
	return "PASS"

func _new_sync(host: Node, script: Script):
	var sync = script.new()
	host.add_child(sync)
	return sync

func _view(ids: Array, names: Array, sides: Array, dead: Array) -> FakeView:
	return FakeView.new(PackedInt64Array(ids), PackedStringArray(names), PackedStringArray(sides), PackedByteArray(dead))

func _a() -> FakeView:
	return _view([1, 2], ["Hero", "Goblin"], ["Player", "Monster"], [0, 0])

func _child_names(node) -> Array:
	var result: Array = []
	for child in node.get_children():
		result.append(String(child.name))
	return result

func _proxy(sync, net_id: int):
	return sync.get_node_or_null("proxy_%d" % net_id)

func _mesh(proxy) -> MeshInstance3D:
	return proxy.get_node("Mesh") as MeshInstance3D

func _label(proxy) -> Label3D:
	return proxy.get_node("Label") as Label3D

func _expect_plan(when: String, got, despawn: Array, spawn: Array, refresh: Array) -> String:
	if not (got is Dictionary):
		return "%s: plan returned %s" % [when, str(got)]
	if got.get("despawn") != despawn:
		return "%s: despawn %s != %s" % [when, str(got.get("despawn")), str(despawn)]
	if got.get("spawn") != spawn:
		return "%s: spawn %s != %s" % [when, str(got.get("spawn")), str(spawn)]
	if got.get("refresh") != refresh:
		return "%s: refresh %s != %s" % [when, str(got.get("refresh")), str(refresh)]
	return "PASS"

func _expect_count(sync, want: int) -> String:
	if sync.proxy_count() != want:
		return "proxy_count = %d, want %d" % [sync.proxy_count(), want]
	return "PASS"

func _expect_same(sync, proxy, net_id: int, tag: String) -> String:
	if _proxy(sync, net_id) != proxy:
		return "%s is not the same object" % tag
	return "PASS"

func _expect_pos(proxy, at: Vector3, tag: String) -> String:
	if proxy.position != at:
		return "%s.position = %s" % [tag, str(proxy.position)]
	return "PASS"

func _expect_mesh(proxy, capsule: bool, tag: String) -> String:
	if (_mesh(proxy).mesh is CapsuleMesh) != capsule:
		return "%s mesh is not a %s" % [tag, "CapsuleMesh" if capsule else "BoxMesh"]
	return "PASS"

func _expect_text(proxy, text: String, tag: String) -> String:
	if _label(proxy).text != text:
		return "%s label = %s" % [tag, _label(proxy).text]
	return "PASS"

func _expect_dead(proxy, dead: bool, tag: String) -> String:
	var override = _mesh(proxy).material_override
	if dead:
		if not (override is StandardMaterial3D):
			return "%s material_override is not a StandardMaterial3D" % tag
		if (override as StandardMaterial3D).albedo_color != TINT:
			return "%s albedo = %s" % [tag, str((override as StandardMaterial3D).albedo_color)]
		if _label(proxy).modulate != TINT:
			return "%s modulate = %s" % [tag, str(_label(proxy).modulate)]
	elif override != null:
		return "%s material_override is not null" % tag
	elif _label(proxy).modulate != WHITE:
		return "%s modulate = %s" % [tag, str(_label(proxy).modulate)]
	return "PASS"

# A -> String helper that crashes returns "" (its type default), never null.
# Map a missing detail to "runtime error" so an empty result never reads as PASS.
func _detail(prefix: String, detail) -> String:
	if detail == null or detail == "":
		return prefix + "runtime error"
	return prefix + detail

func _snapshot(proxy) -> Array:
	return [proxy, String(proxy.name), proxy.position, _mesh(proxy).mesh.get_class(),
		_label(proxy).text, _mesh(proxy).material_override, _label(proxy).modulate]

func _case_s1(host: Node, script: Script) -> String:
	var sync = _new_sync(host, script)
	if not sync.has_method("plan"):
		return "plan() is missing"
	var first = sync.plan(PackedInt64Array([1, 2]))
	var detail := _expect_plan("fresh", first, [], [1, 2], [])
	if detail != "PASS": return _detail("", detail)
	var again = sync.plan(PackedInt64Array([1, 2]))
	if again != first:
		return "plan is not stable: %s != %s" % [str(again), str(first)]
	var empty = _new_sync(host, script)
	detail = _expect_count(sync, 0)
	if detail != "PASS": return _detail("", detail)
	detail = _expect_count(empty, 0)
	if detail != "PASS": return _detail("", detail)
	sync.sync(null)
	empty.sync(_view([], [], [], []))
	detail = _expect_count(sync, 0)
	if detail != "PASS": return _detail("sync(null): ", detail)
	detail = _expect_count(empty, 0)
	if detail != "PASS": return _detail("empty view: ", detail)
	sync.sync(_a())
	detail = _expect_count(sync, 2)
	if detail != "PASS": return _detail("", detail)
	if _child_names(sync) != ["proxy_1", "proxy_2"]:
		return "child names = %s" % str(_child_names(sync))
	# Interface: proxy_count() counts proxy_* children only, not every child.
	var marker := Node3D.new()
	marker.name = "marker"
	sync.add_child(marker)
	detail = _expect_count(sync, 2)
	if detail != "PASS": return _detail("with a marker: ", detail)
	if sync.get_child_count() != 3:
		return "with a marker, child count = %d" % sync.get_child_count()
	sync.remove_child(marker)
	marker.queue_free()
	var p1 = _proxy(sync, 1)
	var p2 = _proxy(sync, 2)
	if p1 == null or p2 == null:
		return "a proxy is missing"
	detail = _expect_pos(p1, Vector3(0, 0, 0), "proxy_1")
	if detail != "PASS": return _detail("", detail)
	detail = _expect_mesh(p1, true, "proxy_1")
	if detail != "PASS": return _detail("", detail)
	var capsule: CapsuleMesh = _mesh(p1).mesh
	# Mesh dimensions are 32-bit, so the decimal literals round-trip through
	# `is_equal_approx`; the Vector3 fields below compare exactly.
	if not is_equal_approx(capsule.radius, 0.4) or not is_equal_approx(capsule.height, 1.2):
		return "capsule = %s x %s" % [str(capsule.radius), str(capsule.height)]
	detail = _expect_text(p1, "Hero", "proxy_1")
	if detail != "PASS": return _detail("", detail)
	detail = _expect_pos(p2, Vector3(3, 0, 0), "proxy_2")
	if detail != "PASS": return _detail("", detail)
	detail = _expect_mesh(p2, false, "proxy_2")
	if detail != "PASS": return _detail("", detail)
	var box: BoxMesh = _mesh(p2).mesh
	if box.size != Vector3(0.8, 1.0, 0.8):
		return "box size = %s" % str(box.size)
	detail = _expect_text(p2, "Goblin", "proxy_2")
	if detail != "PASS": return _detail("", detail)
	sync.sync(null)
	detail = _expect_same(sync, p1, 1, "proxy_1")
	if detail != "PASS": return _detail("sync(null): ", detail)
	detail = _expect_same(sync, p2, 2, "proxy_2")
	if detail != "PASS": return _detail("sync(null): ", detail)
	for proxy in [p1, p2]:
		if not (proxy is Node3D):
			return "%s is not a Node3D" % String(proxy.name)
		if proxy.get_script() != null or not proxy.get_meta_list().is_empty():
			return "%s carries state" % String(proxy.name)
		if proxy.get_child_count() != 2 or _child_names(proxy) != ["Mesh", "Label"]:
			return "%s children = %s" % [String(proxy.name), str(_child_names(proxy))]
		if _label(proxy).position != Vector3(0, 1.2, 0):
			return "%s label position = %s" % [String(proxy.name), str(_label(proxy).position)]
		if _label(proxy).billboard != BaseMaterial3D.BILLBOARD_ENABLED:
			return "%s label billboard = %d" % [String(proxy.name), _label(proxy).billboard]
	return "PASS"

func _case_s2(host: Node, script: Script) -> String:
	var sync = _new_sync(host, script)
	sync.sync(_a())
	var p1 = _proxy(sync, 1)
	var p2 = _proxy(sync, 2)
	var at1 = p1.position
	var at2 = p2.position
	var detail := _expect_plan("appear", sync.plan(PackedInt64Array([1, 2, 3])), [], [3], [1, 2])
	if detail != "PASS": return _detail("", detail)
	sync.sync(_view([1, 2, 3], ["Hero", "Goblin", "Ally"], ["Player", "Monster", "Player"], [0, 0, 0]))
	var p3 = _proxy(sync, 3)
	detail = _expect_count(sync, 3)
	if detail != "PASS": return _detail("", detail)
	if p3 == null: return "proxy_3 is missing"
	detail = _expect_pos(p3, Vector3(0, 0, 1.5), "proxy_3")
	if detail != "PASS": return _detail("", detail)
	detail = _expect_mesh(p3, true, "proxy_3")
	if detail != "PASS": return _detail("", detail)
	detail = _expect_same(sync, p1, 1, "proxy_1")
	if detail != "PASS": return _detail("", detail)
	detail = _expect_same(sync, p2, 2, "proxy_2")
	if detail != "PASS": return _detail("", detail)
	if p1.position != at1 or p2.position != at2:
		return "a kept proxy moved"
	return "PASS"

func _case_s3(host: Node, script: Script) -> String:
	var sync = _new_sync(host, script)
	sync.sync(_a())
	sync.sync(_view([1, 2, 3], ["Hero", "Goblin", "Ally"], ["Player", "Monster", "Player"], [0, 0, 0]))
	var old1 = _proxy(sync, 1)
	var p2 = _proxy(sync, 2)
	var p3 = _proxy(sync, 3)
	if old1 == null or p2 == null or p3 == null:
		return "the S2 scene is incomplete"
	if p3.position != Vector3(0, 0, 1.5):
		return "proxy_3.position before = %s" % str(p3.position)
	freed.append(old1)
	var detail := _expect_plan("leave", sync.plan(PackedInt64Array([2, 3])), [1], [], [2, 3])
	if detail != "PASS": return _detail("", detail)
	sync.sync(_view([2, 3], ["Goblin", "Ally"], ["Monster", "Player"], [0, 0]))
	if sync.get_node_or_null("proxy_1") != null:
		return "proxy_1 was not removed"
	if old1.get_parent() != null:
		return "the freed proxy_1 still has a parent"
	if not old1.is_queued_for_deletion():
		return "the freed proxy_1 is not queued for deletion"
	detail = _expect_count(sync, 2)
	if detail != "PASS": return _detail("", detail)
	detail = _expect_pos(p3, Vector3(0, 0, 0), "proxy_3")
	if detail != "PASS": return _detail("", detail)
	detail = _expect_pos(p2, Vector3(3, 0, 0), "proxy_2")
	if detail != "PASS": return _detail("", detail)
	detail = _expect_same(sync, p2, 2, "proxy_2")
	if detail != "PASS": return _detail("", detail)
	detail = _expect_same(sync, p3, 3, "proxy_3")
	if detail != "PASS": return _detail("", detail)
	if p2.is_queued_for_deletion() or p3.is_queued_for_deletion():
		return "a surviving proxy is queued for deletion"
	return "PASS"

func _case_s4(host: Node, script: Script) -> String:
	var sync = _new_sync(host, script)
	sync.sync(_a())
	sync.sync(_view([1, 2, 3], ["Hero", "Goblin", "Ally"], ["Player", "Monster", "Player"], [0, 0, 0]))
	sync.sync(_view([2, 3], ["Goblin", "Ally"], ["Monster", "Player"], [0, 0]))
	freed.append(_proxy(sync, 2))
	freed.append(_proxy(sync, 3))
	var detail := _expect_plan("despawn all", sync.plan(PackedInt64Array([])), [2, 3], [], [])
	if detail != "PASS": return _detail("", detail)
	sync.sync(_view([], [], [], []))
	detail = _expect_count(sync, 0)
	if detail != "PASS": return _detail("", detail)
	if sync.get_child_count() != 0:
		return "child count = %d" % sync.get_child_count()
	var empty = _new_sync(host, script)
	return _detail("", _expect_plan("empty plan", empty.plan(PackedInt64Array([])), [], [], []))

func _case_s5(host: Node, script: Script) -> String:
	var sync = _new_sync(host, script)
	sync.sync(_a())
	var old2 = _proxy(sync, 2)
	if old2 == null: return "proxy_2 is missing after the first sync"
	freed.append(old2)
	sync.sync(_view([1], ["Hero"], ["Player"], [0]))
	if _proxy(sync, 2) != null:
		return "proxy_2 did not leave after [1]"
	var detail := _expect_plan("reappear", sync.plan(PackedInt64Array([1, 2])), [], [2], [1])
	if detail != "PASS": return _detail("", detail)
	sync.sync(_view([1, 2], ["Hero", "Goblin B"], ["Player", "Monster"], [0, 0]))
	var new2 = _proxy(sync, 2)
	if new2 == null: return "proxy_2 is missing after reappearing"
	if new2 == old2: return "proxy_2 was revived instead of replaced"
	if String(new2.name) != "proxy_2":
		return "proxy_2 name = %s" % String(new2.name)
	detail = _expect_pos(new2, Vector3(3, 0, 0), "proxy_2")
	if detail != "PASS": return _detail("", detail)
	detail = _expect_mesh(new2, false, "proxy_2")
	if detail != "PASS": return _detail("", detail)
	detail = _expect_text(new2, "Goblin B", "proxy_2")
	if detail != "PASS": return _detail("", detail)
	if not old2.is_queued_for_deletion():
		return "the old proxy_2 is not queued for deletion"
	var control = _new_sync(host, script)
	control.sync(_a())
	var kept2 = _proxy(control, 2)
	control.sync(_a())
	return _detail("", _expect_same(control, kept2, 2, "proxy_2"))

func _case_s6(host: Node, script: Script) -> String:
	var sync = _new_sync(host, script)
	sync.sync(_a())
	var p1 = _proxy(sync, 1)
	var p2 = _proxy(sync, 2)
	var at1 = p1.position
	var at2 = p2.position
	sync.sync(_view([1, 2], ["Hero", "Goblin"], ["Player", "Monster"], [0, 1]))
	var detail := _expect_count(sync, 2)
	if detail != "PASS": return _detail("", detail)
	if _proxy(sync, 1) != p1 or _proxy(sync, 2) != p2:
		return "a dead row changed object"
	detail = _expect_dead(p2, true, "proxy_2")
	if detail != "PASS": return _detail("", detail)
	detail = _expect_dead(p1, false, "proxy_1")
	if detail != "PASS": return _detail("", detail)
	if p1.position != at1 or p2.position != at2:
		return "a dead row moved"
	sync.sync(_view([1, 2], ["Hero", "Goblin"], ["Player", "Monster"], [0, 0]))
	detail = _expect_dead(p2, false, "proxy_2")
	if detail != "PASS": return _detail("", detail)
	detail = _expect_dead(p1, false, "proxy_1")
	if detail != "PASS": return _detail("", detail)
	detail = _expect_count(sync, 2)
	if detail != "PASS": return _detail("", detail)
	if p1.position != at1 or p2.position != at2:
		return "a row moved after un-dead"
	# D8 tests dead[i] != 0, not dead[i] == 1: byte 2 greys just like 1.
	sync.sync(_view([1, 2], ["Hero", "Goblin"], ["Player", "Monster"], [2, 0]))
	detail = _expect_dead(p1, true, "proxy_1 (dead 2)")
	if detail != "PASS": return _detail("", detail)
	detail = _expect_dead(p2, false, "proxy_2 (dead 2)")
	if detail != "PASS": return _detail("", detail)
	detail = _expect_count(sync, 2)
	if detail != "PASS": return _detail("", detail)
	if p1.position != at1 or p2.position != at2:
		return "a row moved after dead byte 2"
	sync.sync(_view([1, 2], ["Hero", "Goblin"], ["Player", "Monster"], [0, 0]))
	return _detail("", _expect_dead(p1, false, "proxy_1 (dead 2 cleared)"))

func _case_s7(host: Node, script: Script) -> String:
	var sync = _new_sync(host, script)
	sync.sync(_a())
	var p1 = _proxy(sync, 1)
	var p2 = _proxy(sync, 2)
	var detail := _expect_plan("refresh", sync.plan(PackedInt64Array([1, 2])), [], [], [1, 2])
	if detail != "PASS": return _detail("", detail)
	sync.sync(_view([1, 2], ["Hero", "Goblin Chief"], ["Player", "Monster"], [0, 0]))
	detail = _expect_same(sync, p2, 2, "proxy_2")
	if detail != "PASS": return _detail("", detail)
	detail = _expect_text(p2, "Goblin Chief", "proxy_2")
	if detail != "PASS": return _detail("", detail)
	return _detail("", _expect_text(p1, "Hero", "proxy_1"))

func _case_s8(host: Node, script: Script) -> String:
	var sync = _new_sync(host, script)
	sync.sync(_a())
	var p2 = _proxy(sync, 2)
	sync.sync(_view([2], ["Goblin"], ["Monster"], [0]))
	var detail := _expect_pos(p2, Vector3(3, 0, 0), "proxy_2")
	if detail != "PASS": return _detail("", detail)
	detail = _expect_mesh(p2, false, "proxy_2")
	if detail != "PASS": return _detail("", detail)
	sync.sync(_view([2, 3], ["Goblin", "Ally"], ["Monster", "Player"], [0, 0]))
	var p3 = _proxy(sync, 3)
	if p3 == null: return "proxy_3 is missing"
	detail = _expect_pos(p3, Vector3(0, 0, 0), "proxy_3")
	if detail != "PASS": return _detail("", detail)
	detail = _expect_mesh(p3, true, "proxy_3")
	if detail != "PASS": return _detail("", detail)
	var fresh = _new_sync(host, script)
	fresh.sync(_view([2], ["Goblin"], ["Monster"], [0]))
	var fresh2 = _proxy(fresh, 2)
	detail = _expect_pos(fresh2, Vector3(0, 0, 0), "fresh proxy_2")
	if detail != "PASS": return _detail("", detail)
	detail = _expect_mesh(fresh2, true, "fresh proxy_2")
	if detail != "PASS": return _detail("", detail)
	var cleared = _new_sync(host, script)
	cleared.sync(_a())
	cleared.clear()
	cleared.sync(_view([2], ["Goblin"], ["Monster"], [0]))
	var cleared2 = _proxy(cleared, 2)
	detail = _expect_pos(cleared2, Vector3(0, 0, 0), "after clear proxy_2")
	if detail != "PASS": return _detail("", detail)
	return _detail("", _expect_mesh(cleared2, true, "after clear proxy_2"))

func _case_s9(host: Node, script: Script) -> String:
	var sync = _new_sync(host, script)
	sync.sync(_a())
	var p1 = _proxy(sync, 1)
	var p2 = _proxy(sync, 2)
	sync.sync(_view([1, 2], ["Hero", "Goblin"], ["Monster", "Monster"], [0, 0]))
	var detail := _expect_same(sync, p1, 1, "proxy_1")
	if detail != "PASS": return _detail("", detail)
	detail = _expect_same(sync, p2, 2, "proxy_2")
	if detail != "PASS": return _detail("", detail)
	detail = _expect_pos(p1, Vector3(3, 0, 0), "proxy_1")
	if detail != "PASS": return _detail("", detail)
	detail = _expect_mesh(p1, false, "proxy_1")
	if detail != "PASS": return _detail("", detail)
	detail = _expect_pos(p2, Vector3(3, 0, 1.5), "proxy_2")
	if detail != "PASS": return _detail("", detail)
	sync.sync(_a())
	detail = _expect_same(sync, p1, 1, "proxy_1")
	if detail != "PASS": return _detail("", detail)
	detail = _expect_same(sync, p2, 2, "proxy_2")
	if detail != "PASS": return _detail("", detail)
	detail = _expect_pos(p1, Vector3(0, 0, 0), "proxy_1")
	if detail != "PASS": return _detail("", detail)
	detail = _expect_mesh(p1, true, "proxy_1")
	if detail != "PASS": return _detail("", detail)
	return _detail("", _expect_pos(p2, Vector3(3, 0, 0), "proxy_2"))

func _case_s10(host: Node, script: Script) -> String:
	var sync = _new_sync(host, script)
	var detail := _expect_plan("unsorted", sync.plan(PackedInt64Array([3, 1])), [], [1, 3], [])
	if detail != "PASS": return _detail("", detail)
	sync.sync(_view([1, 3], ["Hero", "Ally"], ["Player", "Player"], [0, 0]))
	if _child_names(sync) != ["proxy_1", "proxy_3"]:
		return "names after [1,3] = %s" % str(_child_names(sync))
	sync.sync(_view([1, 2, 3], ["Hero", "Goblin", "Ally"], ["Player", "Monster", "Player"], [0, 0, 0]))
	if _child_names(sync) != ["proxy_1", "proxy_2", "proxy_3"]:
		return "names after [1,2,3] = %s" % str(_child_names(sync))
	# D3: despawn and refresh sort ascending even when the input ids are
	# unsorted and _proxies was filled in non-ascending order (3, then 2, then 1).
	var packed = _new_sync(host, script)
	packed.sync(_view([3], ["Ally"], ["Player"], [0]))
	packed.sync(_view([2, 3], ["Goblin", "Ally"], ["Monster", "Player"], [0, 0]))
	packed.sync(_view([1, 2, 3], ["Hero", "Goblin", "Ally"], ["Player", "Monster", "Player"], [0, 0, 0]))
	detail = _expect_plan("unsorted ids", packed.plan(PackedInt64Array([3, 2, 1])), [], [], [1, 2, 3])
	if detail != "PASS": return _detail("", detail)
	return _detail("", _expect_plan("unsorted despawn", packed.plan(PackedInt64Array([2])), [1, 3], [], [2]))

func _case_s11(host: Node, script: Script) -> String:
	var sync = _new_sync(host, script)
	sync.sync(_a())
	var p1 = _proxy(sync, 1)
	var p2 = _proxy(sync, 2)
	freed.append(p1)
	freed.append(p2)
	sync.clear()
	var detail := _expect_count(sync, 0)
	if detail != "PASS": return _detail("", detail)
	if sync.get_child_count() != 0:
		return "child count after clear = %d" % sync.get_child_count()
	if not p1.is_queued_for_deletion() or not p2.is_queued_for_deletion():
		return "a cleared proxy is not queued for deletion"
	detail = _expect_plan("after clear", sync.plan(PackedInt64Array([1, 2])), [], [1, 2], [])
	if detail != "PASS": return _detail("", detail)
	sync.sync(_a())
	if _proxy(sync, 1) == p1 or _proxy(sync, 2) == p2:
		return "clear() revived an old proxy"
	detail = _expect_count(sync, 2)
	if detail != "PASS": return _detail("", detail)
	sync.clear()
	detail = _expect_count(sync, 0)
	if detail != "PASS": return _detail("second clear: ", detail)
	sync.clear()
	detail = _expect_count(sync, 0)
	if detail != "PASS": return _detail("clear on the empty scene: ", detail)
	var control = _new_sync(host, script)
	control.sync(_a())
	var kept1 = _proxy(control, 1)
	control.sync(_a())
	return _detail("", _expect_same(control, kept1, 1, "proxy_1"))

func _case_s12(host: Node, script: Script) -> String:
	var sync = _new_sync(host, script)
	sync.sync(_a())
	var p1 = _proxy(sync, 1)
	var p2 = _proxy(sync, 2)
	var detail := _expect_plan("same view", sync.plan(PackedInt64Array([1, 2])), [], [], [1, 2])
	if detail != "PASS": return _detail("", detail)
	var first := [_snapshot(p1), _snapshot(p2)]
	sync.sync(_a())
	var second := [_snapshot(p1), _snapshot(p2)]
	if second != first:
		return "the second sync changed %s != %s" % [str(second), str(first)]
	detail = _expect_same(sync, p1, 1, "proxy_1")
	if detail != "PASS": return _detail("", detail)
	detail = _expect_same(sync, p2, 2, "proxy_2")
	if detail != "PASS": return _detail("", detail)
	sync.sync(_view([1, 2], ["Hero", "Goblin"], ["Player", "Monster"], [1, 0]))
	var override = _mesh(p1).material_override
	if override == null:
		return "the control dead sync did not tint proxy_1"
	# B8/B7: a dead row is part of the idempotence check too, and its override
	# must stay the same object across equal syncs, not be reallocated.
	var dead_first := [_snapshot(p1), _snapshot(p2)]
	sync.sync(_view([1, 2], ["Hero", "Goblin"], ["Player", "Monster"], [1, 0]))
	var dead_second := [_snapshot(p1), _snapshot(p2)]
	if dead_second != dead_first:
		return "the second dead sync changed %s != %s" % [str(dead_second), str(dead_first)]
	if _mesh(p1).material_override != override:
		return "the dead override is not the same object"
	if (_mesh(p1).material_override as StandardMaterial3D).albedo_color != TINT:
		return "the dead albedo = %s" % str((_mesh(p1).material_override as StandardMaterial3D).albedo_color)
	if _label(p1).modulate != TINT:
		return "the dead modulate = %s" % str(_label(p1).modulate)
	kept.append(p1)
	kept.append(p2)
	return "PASS"

func _case_s13(_host: Node, _script: Script) -> String:
	var source = _read_stripped("res://scripts/SceneSync.gd")
	if source == null: return "SceneSync.gd did not read"
	var calls := _matches(source, r"view\.([a-z_]+)\(")
	for call_name in calls:
		if not (call_name in ["net_ids", "names", "sides", "dead"]):
			return "view.%s() is not an allowed getter" % call_name
	for wanted in ["net_ids", "names", "sides", "dead"]:
		if not calls.has(wanted):
			return "view.%s() is not called" % wanted
	var tokens := _matches(source, r"[A-Za-z_][A-Za-z0-9_]*")
	for banned in BANNED:
		if tokens.has(banned):
			return "banned identifier %s" % banned
	var control = _read_stripped("res://scripts/Main.gd")
	if control == null: return "Main.gd did not read"
	if not _matches(control, r"[A-Za-z_][A-Za-z0-9_]*").has("session"):
		return "the control scanner missed session"
	if _matches(control, r"\.revision\(").is_empty():
		return "the control scanner missed the revision call"
	return "PASS"

func _read_stripped(path: String):
	var file := FileAccess.open(path, FileAccess.READ)
	if file == null: return null
	var out := ""
	while not file.eof_reached():
		var line := file.get_line()
		var at := line.find("#")
		if at >= 0: line = line.substr(0, at)
		out += line + "\n"
	file.close()
	return out

func _matches(source: String, pattern: String) -> Array:
	var re := RegEx.new()
	if re.compile(pattern) != OK: return []
	var result: Array = []
	for entry in re.search_all(source):
		result.append(entry.get_string(1) if entry.get_group_count() > 0 else entry.get_string())
	return result
