extends SceneTree
# T051: runs the HUD cases (H1-H19). Exit 0 only when run returns the String
# "PASS"; 6 when it returns anything else (a case failed, or crashed); 3 when
# hud_cases.gd fails to load. H20 (the real session) runs in smoke.gd.
# Run: Godot4 --headless --path apps/client -s res://tests/hud.gd

const CASES := "res://tests/hud_cases.gd"

func _initialize() -> void:
	var script: Script = load(CASES)
	if script == null or not script.can_instantiate():
		printerr("hud.gd FAIL: hud_cases.gd did not load")
		quit(3)
		return
	var cases = script.new()
	var result = cases.run(root)
	if not (result is String) or result != "PASS":
		printerr("hud.gd FAIL: %s" % str(result))
		quit(6)
		return
	print("hud.gd OK")
	quit(0)
