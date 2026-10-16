"""Build a bake-off scratch directory from the saved fixtures.

Usage: python -I setup_fixtures.py <repo-root> <scratch-dir>

Creates <scratch>/tpl/{t1,t2,t3} and <scratch>/ref/hidden.rs, the layout bake.py
and score.py expect. Keep <scratch> outside the repository: a worker that can
see ref/ will copy the answer (docs/agents/MODELS.md section 1, item 3).
"""
import pathlib
import shutil
import sys

repo, scratch = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
fix = pathlib.Path(__file__).resolve().parent.parent / "fixtures"
tpl = scratch / "tpl"
if tpl.exists():
    shutil.rmtree(tpl)
# t1, scout: the repository's own rules file and dependency lint.
(tpl / "t1" / "tools" / "lint").mkdir(parents=True)
shutil.copy(repo / "AGENTS.md", tpl / "t1" / "AGENTS.md")
shutil.copy(repo / "tools" / "lint" / "deps.py", tpl / "t1" / "tools" / "lint" / "deps.py")
# t2, implement: the todo!() crate; the hidden tests go to ref/, never into tpl/.
t2 = tpl / "t2"
shutil.copytree(fix / "implement", t2, ignore=shutil.ignore_patterns("*.rs.txt"))
(t2 / "AGENTS.txt").rename(t2 / "AGENTS.md")
(scratch / "ref").mkdir(parents=True, exist_ok=True)
shutil.copy(fix / "implement" / "hidden.rs.txt", scratch / "ref" / "hidden.rs")
# t3, review: the planted-defect module and its spec.
shutil.copytree(fix / "review", tpl / "t3")
(tpl / "t3" / "SPEC.txt").rename(tpl / "t3" / "SPEC.md")
print(f"fixtures ready in {tpl}")
