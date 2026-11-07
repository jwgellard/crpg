extends CanvasLayer
# T051: the demo HUD. It renders the view (turn, health, options, log, end block)
# and calls `choose` on its parent; it decides no rule (ADR-0033 section 14,
# spec 9.2). The nodes are built in _init so a test can drive the HUD before the
# scene tree runs _ready.

const LOG_MAX_LINES := 200
const NOT_OPEN := "NOT_OPEN"
const ERROR_COLOR := Color(1.0, 0.35, 0.35)

var main: Node = null

var _lines: PackedStringArray = PackedStringArray()
var _logged_revision: int = -1
var _round_label: Label
var _turn_label: Label
var _health_box: VBoxContainer
var _options_box: VBoxContainer
var _waiting_label: Label
var _log: RichTextLabel
var _status_label: Label
var _end_label: Label

func _init() -> void:
	var panel := PanelContainer.new()
	panel.name = "Panel"
	panel.set_anchors_and_offsets_preset(Control.PRESET_TOP_LEFT)
	panel.position = Vector2(12.0, 12.0)
	panel.custom_minimum_size = Vector2(420.0, 0.0)
	add_child(panel)

	var margin := MarginContainer.new()
	margin.name = "Margin"
	for side in ["left", "right", "top", "bottom"]:
		margin.add_theme_constant_override("margin_" + side, 12)
	panel.add_child(margin)

	var column := VBoxContainer.new()
	column.name = "Column"
	column.add_theme_constant_override("separation", 6)
	margin.add_child(column)

	_round_label = _add_label(column, "Round", true)
	_turn_label = _add_label(column, "Turn", true)
	_health_box = _add_box(column, "Health")
	_options_box = _add_box(column, "Options")
	_waiting_label = _add_label(column, "Waiting", false)

	_log = RichTextLabel.new()
	_log.name = "Log"
	_log.custom_minimum_size = Vector2(0.0, 120.0)
	_log.scroll_following = true
	_log.bbcode_enabled = false
	_log.text = ""
	column.add_child(_log)

	_status_label = _add_label(column, "Status", false)
	_end_label = _add_label(column, "End", false)

func _ready() -> void:
	main = get_parent()

func sync(view) -> void:
	if view == null:
		return
	_round_label.text = "Round %d" % (view.round() + 1)
	var active := _name_for(view, view.active())
	_turn_label.text = "Turn: " + active
	_sync_health(view)
	_sync_options(view)
	_waiting_label.text = "Waiting..." if active == "-" else "Waiting for %s..." % active
	_waiting_label.visible = not view.ended() and view.option_labels().is_empty()
	_sync_log(view)
	_sync_end(view)

func show_error(text: String) -> void:
	_status_label.text = text
	_status_label.visible = true
	_status_label.add_theme_color_override("font_color", ERROR_COLOR)
	_append("ERROR: " + text)

func show_notice(text: String) -> void:
	_append(text)

func clear() -> void:
	_clear_children(_health_box)
	_clear_children(_options_box)
	for label in [_round_label, _turn_label, _waiting_label, _status_label, _end_label]:
		label.text = ""
	for label in [_waiting_label, _status_label, _end_label]:
		label.visible = false
	_status_label.remove_theme_color_override("font_color")
	_lines = PackedStringArray()
	_log.text = ""
	_logged_revision = -1

func _add_label(column: VBoxContainer, label_name: String, shown: bool) -> Label:
	var label := Label.new()
	label.name = label_name
	label.text = ""
	label.visible = shown
	column.add_child(label)
	return label

func _add_box(column: VBoxContainer, box_name: String) -> VBoxContainer:
	var box := VBoxContainer.new()
	box.name = box_name
	column.add_child(box)
	return box

func _sync_health(view) -> void:
	_clear_children(_health_box)
	var names = view.names()
	var health = view.health()
	var max_health = view.max_health()
	var known = view.health_known()
	var dead = view.dead()
	for i in range(view.net_ids().size()):
		var text: String = "%s: " % names[i]
		if known[i] == 0:
			text += "?"
		else:
			text += "%d/%d" % [health[i], max_health[i]]
		if dead[i] != 0:
			text += " (dead)"
		var line := Label.new()
		line.text = text
		_health_box.add_child(line)

func _sync_options(view) -> void:
	_clear_children(_options_box)
	var labels = view.option_labels()
	for i in range(labels.size()):
		var button := Button.new()
		button.text = labels[i]
		button.disabled = false
		button.pressed.connect(_on_option.bind(i))
		_options_box.add_child(button)

func _sync_log(view) -> void:
	var revision: int = view.revision()
	if revision == _logged_revision:
		return
	_logged_revision = revision
	for event in view.events():
		_append(_event_text(view, event))
	for receipt in view.receipts():
		if not receipt.get("applied", false):
			_append(_receipt_text(receipt.get("code", 0)))

func _sync_end(view) -> void:
	if view.ended():
		_end_label.text = "Encounter ended\nRounds played: %d\nWinner: %s" % [view.round() + 1, _winner(view)]
		_end_label.visible = true
	else:
		_end_label.text = ""
		_end_label.visible = false

func _winner(view) -> String:
	var sides = view.sides()
	var dead = view.dead()
	var alive: Array = []
	for i in range(sides.size()):
		if dead[i] == 0 and not alive.has(sides[i]):
			alive.append(sides[i])
	if alive.size() == 1:
		return alive[0]
	return "draw"

func _event_text(view, event: Dictionary) -> String:
	var kind: String = event.get("kind", "")
	match kind:
		"action_resolved":
			return "%s | %s -> %s: %s, %d damage" % [
				_name_for(view, event.get("actor", 0)),
				event.get("ability", ""),
				_name_for(view, event.get("target", 0)),
				_verdict(event.get("outcome", "")),
				event.get("damage", 0),
			]
		"turn_started":
			return "Turn: %s (round %d)" % [_name_for(view, event.get("actor", 0)), event.get("round", 0) + 1]
		"died":
			return "%s dies." % _name_for(view, event.get("entity", 0))
		"encounter_ended":
			return "Encounter ended (round %d)" % (event.get("round", 0) + 1)
		_:
			return kind

func _verdict(outcome: String) -> String:
	match outcome:
		"success":
			return "hit (success)"
		"critical_success":
			return "hit (critical success)"
		"failure":
			return "miss (failure)"
		"critical_failure":
			return "miss (critical failure)"
		_:
			return "outcome (%s)" % outcome.replace("_", " ")

func _receipt_text(code: int) -> String:
	var reason := "refused by the server"
	if code == 17:
		reason = "not allowed"
	elif code == 18:
		reason = "no longer legal"
	return "Rejected: %s (code %d)" % [reason, code]

func _name_for(view, net_id: int) -> String:
	var index: int = view.net_ids().find(net_id)
	if index < 0:
		return "-"
	return view.names()[index]

func _append(line: String) -> void:
	_lines.append(line)
	if _lines.size() > LOG_MAX_LINES:
		_lines = _lines.slice(_lines.size() - LOG_MAX_LINES)
	_log.text = "\n".join(_lines)

func _on_option(index: int) -> void:
	if main == null:
		return
	var code: int = main.choose(index)
	if code == 0 or code == 4 or code == 5:
		return
	var message: String = NOT_OPEN if main.session == null else main.session.last_error()
	if code == 30:
		show_notice(message)
	else:
		show_error(message)

func _clear_children(box: VBoxContainer) -> void:
	for child in box.get_children():
		box.remove_child(child)
		child.queue_free()
