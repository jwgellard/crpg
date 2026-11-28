extends "res://scripts/Walk.gd"
# T157g: walk, engage, move, strike, readout, walk again. The party walks in real
# time (Walk.gd's click to move); the host switches the encounter to turn-based
# when a hero comes near the Goblin, or when the player clicks it; in the fight a
# click on the floor declares a move for the active hero, a click on the Goblin
# submits the first option against it, and a readout shows the movement points
# left. When one side is left the party walks again. The client decides no rule:
# it checks no side, reach, distance or allowance (ADR-0033 section 14); the host
# refuses what is not allowed and the refusal shows as a receipt in the HUD log.
# Pick's own handler is off: this scene routes the click itself, because a child's
# _unhandled_input runs before its parent's.

const MODE_TURN_BASED := 0
const ENGAGE := "You engage %s: the fight starts at the next round."
const MOVE_TEXT := "Movement: %d / %d"

func _ready() -> void:
	$Pick.set_process_unhandled_input(false)
	$Camera3D.look_at_from_position(CAMERA_AT, LOOK_AT, Vector3.UP)
	session = ClassDB.instantiate("CrpgSession")
	if session == null:
		open_code = 2
		$Hud.show_error("CrpgSession is not available")
		return
	var code: int = session.open_v4_walk(_campaign_dir(), "", "", OPEN_SEED)
	if code != 0:
		open_code = code
		$Hud.show_error(session.last_error())
		return
	controlled = session.controlled()
	selected = int(controlled[0]) if controlled.size() > 0 else 0
	view = session.view()

func _process(delta: float) -> void:
	super._process(delta)
	if session == null or open_code != 0 or stop_code != 0 or view == null:
		return
	var active: int = int(view.active())
	if active != 0 and controlled.has(active):
		selected = active
		place_marker()
	var text: String = move_text()
	$MoveHud/MoveLabel.text = text
	$MoveHud/MoveLabel.visible = text != ""

func in_fight() -> bool:
	return view != null and int(view.active()) != 0

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
	if in_fight():
		if event.button_index == MOUSE_BUTTON_LEFT:
			fight_click(event.position, point)
	elif event.button_index == MOUSE_BUTTON_LEFT:
		walk_click(event.position, point)
	else:
		halt_selected()

# Walking: a click on your own hero selects it, a click on anyone else asks for the
# fight, a click on the floor sends a move.
func walk_click(screen: Vector2, point: Vector3) -> int:
	if session == null or open_code != 0:
		return 1
	var id: int = $Pick.pick_proxy($Camera3D, $Proxies, screen)
	if controlled.has(id):
		selected = id
		return SELECTED
	if id != 0 and view != null and not view.ended():
		return engage(id)
	return click_ground(point)

# Asks the host to switch to turn-based; the host decides and applies it at the
# next round boundary.
func engage(id: int) -> int:
	if session == null or open_code != 0:
		return 1
	var code: int = session.request_mode(MODE_TURN_BASED)
	if code == 0:
		var who := "-"
		if view != null:
			var row: int = view.net_ids().find(id)
			if row >= 0:
				who = view.names()[row]
		$Hud.show_notice(ENGAGE % who)
	elif code == 5:
		pass
	elif code == 30:
		$Hud.show_notice(session.last_error())
	else:
		$Hud.show_error(session.last_error())
	return code

# In the fight: your own hero is never attacked by a click, anyone else is
# attacked through the first option the host offers against it, the floor is a
# move.
func fight_click(screen: Vector2, point: Vector3) -> int:
	if session == null or open_code != 0:
		return 1
	var id: int = $Pick.pick_proxy($Camera3D, $Proxies, screen)
	if controlled.has(id):
		selected = id
		return SELECTED
	if id != 0:
		return $Pick.attack(id)
	return step_to(point)

# Declares a turn-based move of the active hero to the point.
func step_to(point: Vector3) -> int:
	if session == null or open_code != 0:
		return 1
	var code: int = session.declare_move(seen, to_raw(point.x), 0, to_raw(point.z))
	if code == 0 or code == 4 or code == 5:
		pass
	elif code == 6 or code == 30:
		$Hud.show_notice(session.last_error())
	else:
		$Hud.show_error(session.last_error())
	return code

# The movement readout: the active controlled hero's points, "" when not shown.
func move_text() -> String:
	if not in_fight():
		return ""
	var active: int = int(view.active())
	if not controlled.has(active):
		return ""
	var row: int = view.net_ids().find(active)
	if row < 0:
		return ""
	var now = view.movement()
	var most = view.movement_max()
	if row >= now.size() or row >= most.size() or int(now[row]) < 0:
		return ""
	return MOVE_TEXT % [int(now[row]), int(most[row])]

func _campaign_dir() -> String:
	var from_env := OS.get_environment("CRPG_CAMPAIGN")
	if from_env != "":
		return from_env
	return ProjectSettings.globalize_path("res://").path_join("../../campaigns/demo/adventure").simplify_path()
