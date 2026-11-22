extends Node
# T156c: click an enemy to attack it. A left press casts the camera ray, takes the
# proxy whose centre lies nearest the ray, and submits the first option the host
# offers against it (D78c). The client decides no rule: it checks no sides, and
# `main.choose` is the only way anything is sent. A click on nothing does nothing;
# a click on a row with no option writes a short note in the HUD. The press is not
# marked handled, so a scene that also moves on left click decides which wins.

const PICK_RADIUS := 0.7       # metres from the ray to a proxy's centre
const PICK_HEIGHT := 0.6       # a proxy's centre above its node position
const NOTHING := -1            # attack(): nothing was sent
const NO_OPTION := "No option targets %s"

var main: Node = null         # set in _ready to get_parent(); tests set it
var input_calls: int = 0      # left presses handled (read by tests)

func _ready() -> void:
	main = get_parent()

func _unhandled_input(event: InputEvent) -> void:
	if not (event is InputEventMouseButton):
		return
	if not event.pressed or event.button_index != MOUSE_BUTTON_LEFT:
		return
	if main == null:
		return
	input_calls += 1
	var camera = main.get_node_or_null("Camera3D")
	var proxies = main.get_node_or_null("Proxies")
	if camera == null or proxies == null:
		return
	var id: int = pick_proxy(camera, proxies, event.position)
	if id == 0:
		return
	attack(id)

# The id of the proxy nearest the click ray (within PICK_RADIUS, in front of the
# camera); the smaller id on a tie; 0 when none.
func pick_proxy(camera: Camera3D, proxies: Node, screen: Vector2) -> int:
	var origin: Vector3 = camera.project_ray_origin(screen)
	var dir: Vector3 = camera.project_ray_normal(screen)
	var best_id: int = 0
	var best_distance: float = INF
	for child in proxies.get_children():
		var child_name := String(child.name)
		if not child_name.begins_with("proxy_") or not (child is Node3D):
			continue
		var id: int = int(child_name.substr(6))
		var centre: Vector3 = child.global_position + Vector3(0.0, PICK_HEIGHT, 0.0)
		var along: float = (centre - origin).dot(dir)
		if along < 0.0:
			continue
		var distance: float = (origin + dir * along).distance_to(centre)
		if distance > PICK_RADIUS:
			continue
		if distance < best_distance or (distance == best_distance and id < best_id):
			best_distance = distance
			best_id = id
	return best_id

# The first option whose target is `target`, else -1. End turn (target 0) never
# matches.
func attack_index(view, target: int) -> int:
	if view == null or target == 0:
		return -1
	var targets = view.option_targets()
	for i in range(targets.size()):
		if int(targets[i]) == target:
			return i
	return -1

# Submits the first option against `target`: the chosen code, NOTHING when the
# host offers none, 1 when there is no session.
func attack(target: int) -> int:
	if main == null or main.session == null:
		return 1
	var view = main.view
	var index: int = attack_index(view, target)
	var hud = main.get_node("Hud")
	if index < 0:
		var who := "-"
		if view != null:
			var row: int = view.net_ids().find(target)
			if row >= 0:
				who = view.names()[row]
		hud.show_notice(NO_OPTION % who)
		return NOTHING
	var code: int = main.choose(index)
	if code == 0 or code == 4 or code == 5:
		return code
	if code == 30:
		hud.show_notice(main.session.last_error())
	else:
		hud.show_error(main.session.last_error())
	return code
