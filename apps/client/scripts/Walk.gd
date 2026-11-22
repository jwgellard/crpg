extends Node3D
# T050b: the click-to-move walk scene. It owns one real-time session, polls it
# once per frame, feeds SceneSync and CombatHud, and draws every proxy at the
# position the session says (SceneSync may call only view.net_ids/names/sides/
# dead, so positions are drawn here). Left click on the ground selects a
# controlled proxy or sends a move; right click halts. No rule is decided here
# (ADR-0033 §14); the host decides every move.

const OPEN_SEED := 1
const RAW := 65536.0
const PICK_RADIUS := 0.6
const SELECTED := -1  # click_ground's "selected, nothing sent"
const CAMERA_AT := Vector3(8, 12, 16)
const LOOK_AT := Vector3(8, 0, 6)

var session = null  # CombatHud reads main.session
var view = null
var seen: int = 0
var open_code: int = 0
var stop_code: int = 0
var controlled: PackedInt64Array = PackedInt64Array()
var selected: int = 0
var input_calls: int = 0  # B-W8: calls made from input (read by tests)

func _ready() -> void:
	$Camera3D.look_at_from_position(CAMERA_AT, LOOK_AT, Vector3.UP)
	session = ClassDB.instantiate("CrpgSession")
	if session == null:
		open_code = 2
		$Hud.show_error("CrpgSession is not available")
		return
	var code: int = session.open_v4_rt(_campaign_dir(), "", "", OPEN_SEED)
	if code != 0:
		open_code = code
		$Hud.show_error(session.last_error())
		return
	controlled = session.controlled()
	selected = int(controlled[0]) if controlled.size() > 0 else 0
	view = session.view()

func _process(_delta: float) -> void:
	if session == null or open_code != 0 or stop_code != 0:
		return
	var code: int = session.poll()
	if code == 30:
		# A refused intent is a notice, not a fence: show it and keep going.
		$Hud.show_notice(session.last_error())
	elif code != 0:
		stop_code = code
		$Hud.show_error(session.last_error())
		return
	view = session.view()
	if view.revision() != seen:
		seen = view.revision()
		$Proxies.sync(view)
		$Hud.sync(view)
	draw_rows(view.net_ids(), view.position_known(), session.render_positions())
	place_marker()

func _unhandled_input(event: InputEvent) -> void:
	if not (event is InputEventMouseButton):
		return
	if not event.pressed:
		return
	if event.button_index != MOUSE_BUTTON_LEFT and event.button_index != MOUSE_BUTTON_RIGHT:
		return
	var point = ground_point(event.position)
	if point == null:
		return
	input_calls += 1
	if event.button_index == MOUSE_BUTTON_LEFT:
		click_ground(point)
	else:
		halt_selected()

func _exit_tree() -> void:
	if session != null:
		session.close()
		session = null

func choose(index: int) -> int:
	if session == null:
		return 1
	return session.submit_option(seen, index)

func click_ground(point: Vector3) -> int:
	if session == null or open_code != 0:
		return 1
	var best_id: int = 0
	var best_distance: float = INF
	for raw_id in controlled:
		var id := int(raw_id)
		var node: Node3D = $Proxies.get_node_or_null("proxy_%d" % id)
		if node == null:
			continue
		var distance := Vector2(node.position.x - point.x, node.position.z - point.z).length()
		if distance < best_distance:
			best_distance = distance
			best_id = id
	if best_id != 0 and best_distance <= PICK_RADIUS:
		selected = best_id
		return SELECTED
	var code: int = session.move_to(selected, to_raw(point.x), 0, to_raw(point.z))
	if code != 0:
		$Hud.show_notice(session.last_error())
	return code

func halt_selected() -> int:
	if session == null or open_code != 0:
		return 1
	var code: int = session.halt(selected)
	if code != 0:
		$Hud.show_notice(session.last_error())
	return code

func ground_point(screen: Vector2):
	var origin: Vector3 = $Camera3D.project_ray_origin(screen)
	var dir: Vector3 = $Camera3D.project_ray_normal(screen)
	if absf(dir.y) < 1e-6:
		return null
	var t := -origin.y / dir.y
	if t < 0.0:
		return null
	var hit := origin + dir * t
	hit.y = 0.0
	return hit

func draw_rows(ids: PackedInt64Array, known: PackedByteArray, drawn: PackedInt64Array) -> void:
	if known.size() != ids.size() or drawn.size() != 3 * ids.size():
		return
	for i in range(ids.size()):
		if known[i] != 1:
			continue
		var node: Node3D = $Proxies.get_node_or_null("proxy_%d" % int(ids[i]))
		if node == null:
			continue
		node.position = Vector3(drawn[3 * i] / RAW, drawn[3 * i + 1] / RAW, drawn[3 * i + 2] / RAW)

func place_marker() -> void:
	var node: Node3D = null
	if selected != 0:
		node = $Proxies.get_node_or_null("proxy_%d" % selected)
	if node == null:
		$Marker.visible = false
		return
	$Marker.visible = true
	$Marker.position = Vector3(node.position.x, 0.02, node.position.z)

func to_raw(metres: float) -> int:
	return int(round(metres * RAW))

func _campaign_dir() -> String:
	var from_env := OS.get_environment("CRPG_CAMPAIGN")
	if from_env != "":
		return from_env
	return ProjectSettings.globalize_path("res://").path_join("../../campaigns/demo/walk").simplify_path()
