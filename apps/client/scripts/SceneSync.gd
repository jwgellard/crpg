extends Node3D
# T048: the proxy scene is a function of every view it is given. sync()
# reconciles live proxies against the rows: appears -> spawn, leaves -> free,
# kept -> rewrite in place. Layout and dead tint are client-only (ADR-0033 §14).

const DEAD_TINT := Color(0.35, 0.35, 0.35)
const SIDE_COLORS := [
	Color(0.20, 0.60, 1.00, 1.00),
	Color(1.00, 0.35, 0.20, 1.00),
	Color(0.25, 0.85, 0.45, 1.00),
	Color(0.75, 0.45, 0.95, 1.00),
]

var _proxies: Dictionary = {}        # int NetId -> Node3D; live proxies only
var _side_order: Array[String] = []  # first-seen order since the last clear()

func plan(ids: PackedInt64Array) -> Dictionary:
	var wanted: Dictionary = {}
	for net_id in ids:
		wanted[int(net_id)] = true
	var despawn: Array = []
	for net_id in _proxies.keys():
		if not wanted.has(int(net_id)):
			despawn.append(int(net_id))
	var spawn: Array = []
	var refresh: Array = []
	for net_id in ids:
		var value := int(net_id)
		if _proxies.has(value):
			if not refresh.has(value):
				refresh.append(value)
		elif not spawn.has(value):
			spawn.append(value)
	despawn.sort()
	spawn.sort()
	refresh.sort()
	return {"despawn": despawn, "spawn": spawn, "refresh": refresh}

func sync(view) -> void:
	if view == null:
		return
	var ids: PackedInt64Array = view.net_ids()
	var names: PackedStringArray = view.names()
	var sides: PackedStringArray = view.sides()
	var dead: PackedByteArray = view.dead()
	var moves := plan(ids)
	for net_id in moves["despawn"]:
		_free_proxy(int(net_id))
	for net_id in moves["spawn"]:
		_proxies[int(net_id)] = _make_proxy(int(net_id))
	var ranks: Dictionary = {}
	for i in range(ids.size()):
		var net_id := int(ids[i])
		var side: String = sides[i]
		if not _side_order.has(side):
			_side_order.append(side)
		var side_index: int = _side_order.find(side)
		var rank: int = int(ranks.get(side, 0))
		ranks[side] = rank + 1
		var proxy: Node3D = _proxies[net_id]
		proxy.position = Vector3(3.0 * float(side_index), 0.0, 1.5 * float(rank))
		_set_mesh(proxy.get_node("Mesh") as MeshInstance3D, side_index)
		var label := proxy.get_node("Label") as Label3D
		label.text = names[i]
		_set_dead(proxy, int(dead[i]) != 0)
		move_child(proxy, i)

func clear() -> void:
	for net_id in _proxies.keys():
		_free_proxy(int(net_id))
	_side_order.clear()

func proxy_count() -> int:
	var count: int = 0
	for child in get_children():
		if String(child.name).begins_with("proxy_"):
			count += 1
	return count

func _make_proxy(net_id: int) -> Node3D:
	var proxy := Node3D.new()
	proxy.name = "proxy_%d" % net_id
	var mesh_instance := MeshInstance3D.new()
	mesh_instance.name = "Mesh"
	proxy.add_child(mesh_instance)
	var label := Label3D.new()
	label.name = "Label"
	label.text = ""
	label.position = Vector3(0.0, 1.2, 0.0)
	label.billboard = BaseMaterial3D.BILLBOARD_ENABLED
	label.font_size = 64
	label.pixel_size = 0.0075
	label.outline_size = 16
	proxy.add_child(label)
	add_child(proxy)
	return proxy

func _free_proxy(net_id: int) -> void:
	var proxy: Node3D = _proxies.get(net_id)
	if proxy == null:
		return
	remove_child(proxy)
	proxy.queue_free()
	_proxies.erase(net_id)

func _set_mesh(mesh_instance: MeshInstance3D, side_index: int) -> void:
	var mesh_height: float
	if side_index == 0:
		var capsule := CapsuleMesh.new()
		capsule.radius = 0.4
		capsule.height = 1.2
		mesh_instance.mesh = capsule
		mesh_height = capsule.height
	else:
		var box := BoxMesh.new()
		box.size = Vector3(0.8, 1.0, 0.8)
		mesh_instance.mesh = box
		mesh_height = box.size.y
	# B13: lift only the Mesh child by half its height, so the mesh's lower
	# extent rests on the ground (proxy-local y == 0). The proxy keeps y == 0.
	mesh_instance.position.y = mesh_height / 2.0
	# B14: per-side surface colour (not material_override, which the dead tint
	# owns). Reapplied on every sync so a kept row changing sides recolours.
	var surface := StandardMaterial3D.new()
	surface.albedo_color = SIDE_COLORS[side_index % SIDE_COLORS.size()]
	mesh_instance.set_surface_override_material(0, surface)

func _set_dead(proxy: Node3D, dead: bool) -> void:
	var mesh_instance := proxy.get_node("Mesh") as MeshInstance3D
	var label := proxy.get_node("Label") as Label3D
	if dead:
		var override = mesh_instance.material_override
		if not (override is StandardMaterial3D):
			override = StandardMaterial3D.new()
			mesh_instance.material_override = override
		(override as StandardMaterial3D).albedo_color = DEAD_TINT
		label.modulate = DEAD_TINT
	else:
		mesh_instance.material_override = null
		label.modulate = Color(1.0, 1.0, 1.0)
