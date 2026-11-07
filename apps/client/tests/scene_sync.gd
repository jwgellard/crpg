extends SceneTree
# T048: runs the SceneSync cases (S1-S13) and checks S14 two frames later: no
# freed proxy is still valid and every kept proxy is. Exit 0 only when run
# returns the String "PASS"; 6 when it returns anything else (a case failed, or
# a crash made a -> String function return "" instead of "PASS"); 7 a
# deferred-free check failed; 3 a script failed to load.
# Run: Godot4 --headless --path apps/client -s res://tests/scene_sync.gd

const CASES := "res://tests/scene_sync_cases.gd"

var _cases = null
var _done: bool = false
var _frames: int = 0

func _initialize() -> void:
	var script: Script = load(CASES)
	if script == null or not script.can_instantiate():
		printerr("scene_sync.gd FAIL: scene_sync_cases.gd did not load")
		quit(3)
		return
	_cases = script.new()
	if _cases == null or not _cases.has_method("run"):
		printerr("scene_sync.gd FAIL: scene_sync_cases.gd did not instantiate")
		quit(3)
		return
	var result = _cases.run(root)
	if not (result is String) or result != "PASS":
		printerr("scene_sync.gd FAIL: %s" % str(result))
		quit(6)
		return
	_done = true

func _process(_delta: float) -> bool:
	if not _done:
		return false
	_frames += 1
	if _frames < 2:
		return false
	for node in _cases.freed:
		if is_instance_valid(node):
			printerr("scene_sync.gd FAIL: a freed proxy is still valid")
			quit(7)
			return false
	for node in _cases.kept:
		if not is_instance_valid(node):
			printerr("scene_sync.gd FAIL: a kept proxy was freed")
			quit(7)
			return false
	print("scene_sync.gd OK")
	quit(0)
	return false
