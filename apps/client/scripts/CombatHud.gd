extends CanvasLayer
# T047d: round, active name, health lines, one button per option, the event
# log and the end label. It renders the view and calls `choose`; it decides no
# rule (ADR-0033 §14).

var main: Node = null

var _round_label: Label
var _active_label: Label
var _health_box: VBoxContainer
var _options_box: VBoxContainer
var _log: RichTextLabel
var _end_label: Label
var _status_label: Label
var _logged_revision: int = -1

func _ready() -> void:
	main = get_parent()
	_build()

func _build() -> void:
	var panel := PanelContainer.new()
	panel.set_anchors_and_offsets_preset(Control.PRESET_TOP_LEFT)
	panel.position = Vector2(12.0, 12.0)
	panel.custom_minimum_size = Vector2(420.0, 0.0)
	add_child(panel)

	var margin := MarginContainer.new()
	margin.add_theme_constant_override("margin_left", 12)
	margin.add_theme_constant_override("margin_right", 12)
	margin.add_theme_constant_override("margin_top", 12)
	margin.add_theme_constant_override("margin_bottom", 12)
	panel.add_child(margin)

	var column := VBoxContainer.new()
	column.add_theme_constant_override("separation", 6)
	margin.add_child(column)

	_round_label = Label.new()
	column.add_child(_round_label)
	_active_label = Label.new()
	column.add_child(_active_label)

	_health_box = VBoxContainer.new()
	column.add_child(_health_box)

	_options_box = VBoxContainer.new()
	column.add_child(_options_box)

	_log = RichTextLabel.new()
	_log.custom_minimum_size = Vector2(0.0, 120.0)
	_log.scroll_following = true
	column.add_child(_log)

	_status_label = Label.new()
	_status_label.visible = false
	column.add_child(_status_label)

	_end_label = Label.new()
	_end_label.visible = false
	column.add_child(_end_label)

func sync(view) -> void:
	if view == null:
		return
	_round_label.text = "Round %d" % (view.round() + 1)
	_active_label.text = "Active: %s" % _name_for(view, view.active())
	_sync_health(view)
	_sync_options(view)
	_sync_events(view)
	_end_label.text = "Encounter ended"
	_end_label.visible = view.ended()

func show_error(text: String) -> void:
	_status_label.text = text
	_status_label.visible = true
	_status_label.add_theme_color_override("font_color", Color(1.0, 0.35, 0.35))
	_log.append_text("ERROR: " + text + "\n")

func show_notice(text: String) -> void:
	_log.append_text(text + "\n")

func _sync_health(view) -> void:
	_clear(_health_box)
	var ids = view.net_ids()
	var names = view.names()
	var health = view.health()
	var max_health = view.max_health()
	var known = view.health_known()
	var dead = view.dead()
	for i in range(ids.size()):
		var line := Label.new()
		var text := "%s: " % names[i]
		if known[i] == 0:
			text += "?"
		else:
			text += "%d/%d" % [health[i], max_health[i]]
		if dead[i] != 0:
			text += " (dead)"
		line.text = text
		_health_box.add_child(line)

func _sync_options(view) -> void:
	_clear(_options_box)
	var labels = view.option_labels()
	for i in range(labels.size()):
		var button := Button.new()
		button.text = labels[i]
		button.pressed.connect(_on_option.bind(i))
		_options_box.add_child(button)

func _sync_events(view) -> void:
	var revision: int = view.revision()
	if revision == _logged_revision:
		return
	_logged_revision = revision
	for event in view.events():
		_log.append_text(_event_text(view, event) + "\n")

func _event_text(view, event: Dictionary) -> String:
	var kind: String = event.get("kind", "")
	match kind:
		"action_resolved":
			return "%s -> %s: %s (%s, %d)" % [
				_name_for(view, event.get("actor", 0)),
				_name_for(view, event.get("target", 0)),
				event.get("ability", ""),
				event.get("outcome", ""),
				event.get("damage", 0),
			]
		"turn_started":
			return "Turn: %s (round %d)" % [
				_name_for(view, event.get("actor", 0)),
				event.get("round", 0),
			]
		"died":
			return "Died: %s" % _name_for(view, event.get("entity", 0))
		"encounter_ended":
			return "Encounter ended (round %d)" % event.get("round", 0)
		_:
			return kind

func _name_for(view, net_id: int) -> String:
	var index: int = view.net_ids().find(net_id)
	if index < 0:
		return "-"
	return view.names()[index]

func _on_option(index: int) -> void:
	if main == null:
		return
	var code: int = main.choose(index)
	if code != 0 and code != 4 and code != 5:
		show_error(main.session.last_error())

func _clear(box: VBoxContainer) -> void:
	for child in box.get_children():
		box.remove_child(child)
		child.queue_free()
