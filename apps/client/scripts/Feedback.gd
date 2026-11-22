extends Node3D
# T156c: combat feedback. A health bar floats over each combatant, a red "-N" or
# grey "miss" rises from whoever was hit, and the target flashes white briefly.
# Numbers and flashes come from `action_resolved` events and bars from the health
# columns (spec 9.2 rule 2: render events, do not diff health). It reads only the
# view, sends nothing and never touches a proxy node, its children or its
# materials; bars and numbers are 2D controls placed each frame over the proxy's
# current position, so they follow whoever draws the proxy. All timing here is
# presentation: nothing waits on or changes the session.

const BAR_SIZE := Vector2(64, 8)
const BAR_LIFT := 2.0          # metres above the proxy origin
const NUMBER_LIFT := 2.3
const NUMBER_MS := 1000.0
const NUMBER_RISE := 48.0      # pixels over a number's life
const FLASH_MS := 250.0
const FLASH_LIFT := 0.6
const FLASH_RADIUS := 0.65
const HIT_COLOR := Color(1.0, 0.3, 0.3)
const MISS_COLOR := Color(0.8, 0.8, 0.8)
const FLASH_COLOR := Color(1.0, 1.0, 1.0, 0.5)

var main: Node = null          # set in _ready to get_parent(); tests set it
var numbers_spawned: int = 0   # every number ever spawned (read by tests)

var _overlay: CanvasLayer
var _revision: int = -1
var _bars: Dictionary = {}     # int net id -> ProgressBar
var _alive: Dictionary = {}    # int net id -> bool
var _numbers: Array = []       # of {"node": Label, "target": int, "age": float}
var _flashes: Dictionary = {}  # int net id -> {"node": MeshInstance3D, "age": float}

func _init() -> void:
	_overlay = CanvasLayer.new()
	_overlay.name = "Overlay"
	_overlay.layer = 0
	add_child(_overlay)

func _ready() -> void:
	main = get_parent()

func _process(delta: float) -> void:
	if main == null or not ("view" in main):
		return
	sync(main.view)
	advance(delta * 1000.0)
	var camera = main.get_node_or_null("Camera3D")
	var proxies = main.get_node_or_null("Proxies")
	if camera != null and proxies != null:
		place(camera, proxies)

func sync(view) -> void:
	if view == null:
		return
	var revision: int = view.revision()
	if revision == _revision:
		return
	_revision = revision
	show_view(view)

func show_view(view) -> void:
	_show_bars(view)
	for event in view.events():
		if event.get("kind", "") != "action_resolved":
			continue
		var damage: int = int(event.get("damage", 0))
		var outcome: String = event.get("outcome", "")
		var target: int = int(event.get("target", 0))
		_spawn_number(target, damage, outcome)
		if damage > 0:
			_flash(target)

func advance(ms: float) -> void:
	for entry in _numbers.duplicate():
		entry["age"] += ms
		var label: Label = entry["node"]
		if entry["age"] >= NUMBER_MS:
			_numbers.erase(entry)
			_release(label)
		else:
			label.modulate.a = 1.0 - entry["age"] / NUMBER_MS
	for target in _flashes.keys():
		var entry: Dictionary = _flashes[target]
		entry["age"] += ms
		var flash: MeshInstance3D = entry["node"]
		if entry["age"] >= FLASH_MS:
			_flashes.erase(target)
			_release(flash)
		else:
			var material := flash.material_override as StandardMaterial3D
			material.albedo_color.a = FLASH_COLOR.a * (1.0 - entry["age"] / FLASH_MS)

func place(camera: Camera3D, proxies: Node) -> void:
	for id in _bars.keys():
		var bar: ProgressBar = _bars[id]
		var proxy = proxies.get_node_or_null("proxy_%d" % int(id))
		if proxy == null:
			bar.visible = false
			continue
		var at: Vector3 = proxy.global_position + Vector3(0.0, BAR_LIFT, 0.0)
		bar.visible = bool(_alive.get(id, false)) and not camera.is_position_behind(at)
		bar.position = camera.unproject_position(at) - BAR_SIZE / 2.0
	for entry in _numbers:
		var label: Label = entry["node"]
		var proxy = proxies.get_node_or_null("proxy_%d" % int(entry["target"]))
		if proxy == null:
			label.visible = false
			continue
		var at: Vector3 = proxy.global_position + Vector3(0.0, NUMBER_LIFT, 0.0)
		label.visible = not camera.is_position_behind(at)
		label.position = camera.unproject_position(at) + Vector2(0.0, -NUMBER_RISE * entry["age"] / NUMBER_MS)
	for target in _flashes.keys():
		var flash: MeshInstance3D = _flashes[target]["node"]
		var proxy = proxies.get_node_or_null("proxy_%d" % int(target))
		if proxy == null:
			flash.visible = false
			continue
		flash.visible = true
		flash.global_position = proxy.global_position + Vector3(0.0, FLASH_LIFT, 0.0)

# ---- bars ----------------------------------------------------------------

func _show_bars(view) -> void:
	var ids = view.net_ids()
	var health = view.health()
	var max_health = view.max_health()
	var known = view.health_known()
	var dead = view.dead()
	var wanted: Dictionary = {}
	for i in range(ids.size()):
		var id: int = int(ids[i])
		if int(known[i]) != 1 or int(max_health[i]) <= 0:
			continue
		wanted[id] = true
		var bar: ProgressBar = _bars.get(id)
		if bar == null:
			bar = _make_bar(id)
			_bars[id] = bar
		bar.max_value = int(max_health[i])
		bar.value = clampi(int(health[i]), 0, int(max_health[i]))
		_alive[id] = int(dead[i]) == 0
		bar.visible = bool(_alive[id])
	for id in _bars.keys():
		if not wanted.has(id):
			_release(_bars[id])
			_bars.erase(id)
			_alive.erase(id)

func _make_bar(id: int) -> ProgressBar:
	var bar := ProgressBar.new()
	bar.name = "bar_%d" % id
	bar.show_percentage = false
	bar.min_value = 0
	bar.size = BAR_SIZE
	bar.custom_minimum_size = BAR_SIZE
	bar.mouse_filter = Control.MOUSE_FILTER_IGNORE
	var fill := StyleBoxFlat.new()
	fill.bg_color = HIT_COLOR
	bar.add_theme_stylebox_override("fill", fill)
	_overlay.add_child(bar)
	return bar

# ---- numbers and flashes -------------------------------------------------

func _spawn_number(target: int, damage: int, outcome: String) -> void:
	var text := "0"
	if damage > 0:
		text = "-%d" % damage
		if outcome == "critical_success":
			text += "!"
	elif outcome == "failure" or outcome == "critical_failure":
		text = "miss"
	numbers_spawned += 1
	var label := Label.new()
	label.name = "num_%d" % numbers_spawned
	label.text = text
	label.mouse_filter = Control.MOUSE_FILTER_IGNORE
	label.add_theme_color_override("font_color", HIT_COLOR if damage > 0 else MISS_COLOR)
	label.add_theme_font_size_override("font_size", 28)
	label.add_theme_constant_override("outline_size", 6)
	label.add_theme_color_override("font_outline_color", Color(0.0, 0.0, 0.0))
	_overlay.add_child(label)
	_numbers.append({"node": label, "target": target, "age": 0.0})

func _flash(target: int) -> void:
	if _flashes.has(target):
		_flashes[target]["age"] = 0.0
		return
	var mesh := SphereMesh.new()
	mesh.radius = FLASH_RADIUS
	mesh.height = FLASH_RADIUS * 2.0
	var material := StandardMaterial3D.new()
	material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	material.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	material.albedo_color = FLASH_COLOR
	var flash := MeshInstance3D.new()
	flash.name = "flash_%d" % target
	flash.mesh = mesh
	flash.material_override = material
	add_child(flash)
	_flashes[target] = {"node": flash, "age": 0.0}

# Frees a node the way a proxy is freed: remove first, so its name is free at once.
func _release(node: Node) -> void:
	var parent := node.get_parent()
	if parent != null:
		parent.remove_child(node)
	node.queue_free()
