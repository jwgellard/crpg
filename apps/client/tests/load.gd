extends SceneTree
# E1 (T047b): proves the gdext library loads and CrpgSession works end to end.
# Run: Godot4 --headless --path apps/client -s res://tests/load.gd
# Exit 0 pass; 2 class missing; 3 first failed check.
# Control: CRPG_CAMPAIGN set to a missing directory -> open = 20 -> exit 3.

func _fail(message: String) -> void:
	printerr("load.gd FAIL: ", message)
	quit(3)

func _campaign_dir() -> String:
	var from_env := OS.get_environment("CRPG_CAMPAIGN")
	if from_env != "":
		return from_env
	return ProjectSettings.globalize_path("res://").path_join("../../campaigns/fixtures/combat_basic").simplify_path()

func _init() -> void:
	if not ClassDB.class_exists("CrpgSession"):
		printerr("load.gd: class CrpgSession is missing")
		quit(2)
		return
	var session = ClassDB.instantiate("CrpgSession")
	if session == null:
		_fail("instantiate returned null")
		return
	var code: int = session.open(_campaign_dir(), "", "", 1)
	if code != 0:
		_fail("open = %d (%s)" % [code, session.last_error()])
		return
	code = session.poll()
	if code != 0:
		_fail("poll = %d (%s)" % [code, session.last_error()])
		return
	var view = session.view()
	if view.revision() != 1:
		_fail("revision = %d" % view.revision())
		return
	if view.net_ids().size() != 2:
		_fail("net_ids size = %d" % view.net_ids().size())
		return
	if view.names() != PackedStringArray(["Hero", "Goblin"]):
		_fail("names = %s" % str(view.names()))
		return
	# The fixture opens with the goblin's turn, so the Hero (the default side)
	# has no options on the first poll; the NPC acts after its 600 ms delay.
	var waited_ms := 0
	while view.option_labels().size() == 0 and waited_ms < 5000:
		OS.delay_msec(50)
		waited_ms += 50
		code = session.poll()
		if code != 0:
			_fail("poll = %d (%s)" % [code, session.last_error()])
			return
		view = session.view()
	if view.option_labels().size() <= 0:
		_fail("no option labels after %d ms" % waited_ms)
		return
	if not (view.events() is Array):
		_fail("events is not an Array")
		return
	if session.submit_option(-1, 0) != 3:
		_fail("submit_option(-1, 0) != 3")
		return
	if session.submit_option(view.revision(), 99) != 6:
		_fail("submit_option(rev, 99) != 6")
		return
	session.close()
	if session.poll() != 1:
		_fail("poll after close != 1")
		return
	print("load.gd OK")
	quit(0)
