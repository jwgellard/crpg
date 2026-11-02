extends Node3D
# T047d: spawns one primitive proxy per view row, once, from the first
# non-empty view. The layout is client-only (ADR-0033 §14); it decides nothing
# and never despawns (that is T048).

var _spawned: bool = false

func sync(view) -> void:
	if _spawned or view == null:
		return
	var ids = view.net_ids()
	if ids.size() == 0:
		return
	_spawned = true
	var names = view.names()
	var sides = view.sides()
	var side_order: Array[String] = []
	var side_counts := {}
	for i in range(ids.size()):
		var side: String = sides[i]
		if not side_order.has(side):
			side_order.append(side)
		var side_index: int = side_order.find(side)
		var within: int = int(side_counts.get(side, 0))
		side_counts[side] = within + 1
		_spawn_proxy(ids[i], names[i], side_index, within)

func proxy_count() -> int:
	var count: int = 0
	for child in get_children():
		if String(child.name).begins_with("proxy_"):
			count += 1
	return count

func _spawn_proxy(net_id: int, display_name: String, side_index: int, within: int) -> void:
	var proxy := Node3D.new()
	proxy.name = "proxy_%d" % net_id
	proxy.position = Vector3(3.0 * float(side_index), 0.0, 1.5 * float(within))

	var mesh_instance := MeshInstance3D.new()
	if side_index == 0:
		var capsule := CapsuleMesh.new()
		capsule.radius = 0.4
		capsule.height = 1.2
		mesh_instance.mesh = capsule
	else:
		var box := BoxMesh.new()
		box.size = Vector3(0.8, 1.0, 0.8)
		mesh_instance.mesh = box
	proxy.add_child(mesh_instance)

	var label := Label3D.new()
	label.text = display_name
	label.position = Vector3(0.0, 1.2, 0.0)
	label.billboard = BaseMaterial3D.BILLBOARD_ENABLED
	proxy.add_child(label)

	add_child(proxy)
