# R10-deep (T099) · Authoring interfaces for AI agents — deep pass

Deep pass over `PASS1.md`; R10-deep corrects its reasoning where the deeper literature disagrees.
Method: 45 pages fetched this run from 35 distinct sources (20 academic: 13 peer-reviewed conference/journal
papers, 1 open-access thesis-adjacent replication study, plus preprints; 15 vendor/standards/practitioner docs).
References followed: SWE-agent → ReAct + SWE-bench (+Reflexion); Anthropic *Writing tools* → MCP tools +
*SWE-bench Sonnet*; Anthropic *Building effective agents* → tool-use docs + *Writing tools*.
Every quote below is verbatim from a page fetched in this run.

## 1. Recommendations, ranked by expected impact on our agent authoring

**R1. Build a small intent-level layer above `EditCommand`, shaped as a few consolidated verbs (not one command per JSON field), shared by GUI and agents.** `new` (confirms the *direction* of PASS1 R1, corrects its *shape*).
Why: CONTEXT.md gives agents nine primitives; "add an NPC with this conversation to this area" is a hand-assembled batch. Consolidation is the vendor-documented answer, and it rides our atomic batch/undo so invariants still hold.
Evidence: https://www.anthropic.com/engineering/writing-tools-for-agents — "Tools can consolidate functionality, handling potentially *multiple* discrete operations (or API calls) under the hood." · https://platform.openai.com/docs/guides/function-calling — "Combine functions that are always called in sequence." · https://arxiv.org/html/2405.15793v3 — "10.7 percentage points *more* instances than the baseline agent, which uses only the default Linux shell."
Strength: measured study + vendor doc. Effort: medium.

**R2. Make every rejection carry a machine-readable pointer + code + suggested fix, at validate time and apply time.** `confirmed from pass 1` (R2).
Why: CONTEXT.md §4.2 promises "a suggested fix"; the crpg-edit doc says that "field is not built yet". Actionable errors are the best-measured interface win, and apply-time validation is the natural hook.
Evidence: https://arxiv.org/html/2405.15793v3 — "This intervention improves performance considerably (without linting, 15.0% ↓ 3.0)." and "Any attempt at editing has a 90.5% chance of eventually being successful. This probability drops off to 57.2% after a single failed edit." · https://modelcontextprotocol.io/docs/concepts/tools — tool errors should be "actionable feedback that language models can use to self-correct and retry with adjusted parameters".
Strength: measured study + vendor doc. Effort: small.

**R3. Add a two-phase plan/apply: `crpgc apply --plan` returns a reviewable diff + diagnostics, saveable and re-executable, and apply never rolls back silently.** `confirmed from pass 1` (extends R4+R7).
Why: §11.2 already has a pure `SavePlan`; making commit a second step gives agents and the GUI the same review loop, and documents atomicity/partial-failure honestly.
Evidence: https://developer.hashicorp.com/terraform/cli/commands/plan — "The `plan` command alone does not actually carry out the proposed changes" and "save the generated plan to a file on disk, which you can later execute by passing the file to `terraform apply`". · https://developer.hashicorp.com/terraform/cli/commands/apply — "Terraform does not automatically roll back a partially-completed apply."
Strength: vendor doc. Effort: medium.

**R4. Make identity the agent-visible address: stable ULIDs for objects, JSON Pointers only for low-level leaf edits; return semantic handles (slug + id) in receipts.** `confirmed from pass 1` (R5, corrected — see §3).
Why: §11.2 already offers "by document path or by object ULID". Keep both, but make intent commands take ULIDs/slugs and return them; paths rename, ULIDs do not. Cheap and removes a whole hallucination class.
Evidence: https://www.anthropic.com/engineering/writing-tools-for-agents — "merely resolving arbitrary alphanumeric UUIDs to more semantically meaningful and interpretable language ... significantly improves Claude's precision in retrieval tasks by reducing hallucinations". · https://platform.claude.com/docs/en/agents-and-tools/tool-use/define-tools — "Return semantic, stable identifiers (for example, slugs or UUIDs) rather than opaque internal references".
Strength: vendor doc. Effort: small.

**R5. Prefer uniquely-anchored local edits over whole-document rewrites; fail loudly on ambiguous anchors; keep the lossless text round-trip for dialogue/quests.** `new`.
Why: whole-file writes are simplest but costliest and, in SWE-agent's ablation, the *full-file* viewer hurt; a search/replace or line-range edit that errors when the anchor is not unique is the highest-reliability pattern the strongest coding agents converged on. This is exactly §4.8's dialogue text round-trip, generalized.
Evidence: https://aider.chat/docs/more/edit-formats.html — "it can be slow and costly because the LLM has to return the *entire file* even if just a few lines are edited"; likewise the "udiff" format "reduced their 'lazy coding' tendencies". · https://www.anthropic.com/engineering/swe-bench-sonnet — "The replacement will only occur if there is exactly one match of `old_str`. If there are more or fewer matches, the model is shown an appropriate error message for it to retry."
Strength: measured study (Aider leaderboard) + vendor doc. Effort: medium.

**R6. Bound and shape every response; steer the next step; receipts carry only changed paths + diagnostics.** `confirmed from pass 1` (R6).
Why: receipts grow with campaign size; context is the agent's scarcest resource. Same principle the GUI's Problems panel needs.
Evidence: https://www.anthropic.com/engineering/writing-tools-for-agents — "For Claude Code, we restrict tool responses to 25,000 tokens by default." and "we use ~⅓ of the tokens with `concise` tool responses". · https://arxiv.org/html/2405.15793v3 — "Either too little content (30 lines, 14.3% ↓ 3.7) or too much (entire file, 12.7% ↓ 5.3) lowers performance."
Strength: measured study + vendor doc. Effort: small.

**R7. Write 3–4 sentence, namespaced descriptions per verb; keep the surface under ~20; treat examples as eval-gated.** `confirmed from pass 1` (R3, corrected nuance).
Why: we are already at the ceiling (nine `EditCommand`s, five FFI types); the remaining lever is description quality. Namespacing (`edit.add_npc`) helps when the layer grows.
Evidence: https://platform.claude.com/docs/en/agents-and-tools/tool-use/define-tools — "Aim for at least 3–4 sentences for each tool description, more if the tool is complex." · https://platform.openai.com/docs/guides/function-calling — "Aim for fewer than 20 functions available at the start of a turn at any one time, though this is just a soft suggestion" and "Adding examples may hurt performance for reasoning models".
Strength: vendor doc. Effort: small.

**R8. If humans and agents share one campaign (the editor-is-a-client model), adopt field-ownership + conflict rejection rather than last-writer-wins.** `new`.
Why: §11.1 makes the editor a live client of a running session, so two writers can touch one object. Kubernetes solved exactly this with declarative intent plus tracked ownership.
Evidence: https://kubernetes.io/docs/reference/using-api/server-side-apply/ — "trying to change a field that is controlled by a different manager results in a rejected request unless the client forces an override"; clients submit "their *fully specified intent*".
Strength: vendor doc. Effort: large.

**R9. `crpgc schema <type>` should return the schema *plus* a validated minimal example and the fields agents most often get wrong.** `new` (extends §4.8 item 1).
Why: raw schemas are hard to instantiate; retrieval of docs+examples is what beat GPT-4 at API calls, and our strict schemas (`additionalProperties:false`) already define the target shape.
Evidence: https://arxiv.org/abs/2305.15334 — "When combined with a document retriever, Gorilla demonstrates a strong capability to adapt to test-time document changes". · https://arxiv.org/abs/2203.15491 — "we analyze its use by the Kaggle community, identifying unused and apparently useless parts of the API that can be eliminated without affecting client programs." · https://platform.openai.com/docs/guides/structured-outputs — "ensures the model will always generate responses that adhere to your supplied JSON Schema".
Strength: measured study + vendor doc. Effort: medium.

**R10. Build the eval harness now, scored end-state + pass^k, ~20 tasks, held-out set.** `confirmed from pass 1` (R8, strengthened).
Why: Phase 6's definition of done ("created entirely in the editor by someone who has never seen the JSON") is the scenario; end-state scoring survives nondeterminism, and pass^k exposes inconsistency.
Evidence: https://arxiv.org/abs/2406.12045 — "even state-of-the-art function calling agents (like gpt-4o) succeed on <50% of the tasks, and are quite inconsistent (pass^8 <25% in retail)." · https://www.anthropic.com/engineering/multi-agent-research-system — "We started with a set of about 20 queries representing real usage patterns." and "end-state evaluation rather than turn-by-turn analysis". · https://www.anthropic.com/engineering/writing-tools-for-agents — "We relied on held-out test sets to ensure we did not overfit".
Strength: measured study + vendor doc. Effort: medium.

## 2. What the research literature says

- **SWE-agent (NeurIPS 2024; arXiv 2405.15793).** The agent-computer interface, not the model, is the lever: +10.7pp over shell-only on SWE-bench Lite (18.0 vs 11.0); no-edit command 10.3 vs 18.0; edit-without-linting 15.0 vs 18.0; 30-line or full-file viewer 14.3/12.7 vs 18.0. Failed edits compound: "51.7%" of trajectories have ≥1 failed edit; recovery 90.5%→57.2%. Successful runs are cheap: median "$1.21 and 12 steps" vs "$2.52 and 21 steps".
- **ReAct (ICLR 2023; arXiv 2210.03629).** Interleaving reasoning traces with actions beats act-only baselines: "an absolute success rate of 34% and 10%" (ALFWorld, WebShop). Justifies receipts that carry a thought-then-action structure.
- **Reflexion (NeurIPS 2023; arXiv 2303.11366).** Verbal self-feedback: "91% pass@1 accuracy on the HumanEval coding benchmark, surpassing ... GPT-4 that achieves 80%". Errors should be usable as learning signal within a run.
- **SWE-bench (ICLR 2024; arXiv 2310.06770).** 2,294 real GitHub issues; baseline "Claude 2 ... solve a mere 1.96% of the issues" — the difficulty we should expect.
- **Gorilla / APIBench (NeurIPS 2024; arXiv 2305.15334).** Retrieval over documentation "substantially mitigates the issue of hallucination"; a finetuned model "surpasses the performance of GPT-4 on writing API calls".
- **Toolformer (NeurIPS 2023; arXiv 2302.04761).** Self-supervised tool selection; shows the model learns "which APIs to call, when to call them, what arguments to pass".
- **CodeAct (ICML 2024; arXiv 2402.01030).** Unifying actions as executable Python "outperforms widely used alternatives (up to 20% higher success rate)" — an argument for a composable command file rather than fixed tool slots.
- **Lost in the Middle (TACL 2023; arXiv 2307.03172).** "performance is often highest when relevant information occurs at the beginning or end of the input context, and significantly degrades when models must access relevant information in the middle" — put the fix/next-step at the edges of responses.
- **API-Bank (EMNLP 2023; arXiv 2304.08244).** 73 API tools, 314 annotated dialogues, 753 calls; "Lynx surpasses Alpaca's tool utilization performance by more than 26 pts".
- **τ-bench (arXiv 2406.12045).** gpt-4o "<50% of the tasks", pass^8 "<25% in retail"; rules and consistency are the frontier, not single calls.
- **Chain-of-Thought (NeurIPS 2022; arXiv 2201.11903).** Eight exemplars reached SOTA on GSM8K — examples help; R7 keeps them eval-gated for reasoning models.
- **Semi-automated API wrapping (ICSE-NIER 2022; arXiv 2203.15491).** Usability work found "unused and apparently useless parts of the API that can be eliminated without affecting client programs" — the case for a small surface.
- **Berkeley Function-Calling Leaderboard (NeurIPS 2024).** 2,000 pairs across Python/Java/JS/REST/SQL; failure modes include parameters needing implicit conversion and hallucinated function calls, motivating R2/R9.

Reference-following evidence: SWE-agent cites ReAct [62] and SWE-bench [20]; I fetched both (and Reflexion [39]). Anthropic *Writing tools* cites MCP and *SWE-bench Sonnet*; both fetched. Anthropic *Building effective agents* cites its tool-use docs and *Writing tools*; both fetched.

## 3. Pass-1 claims that did not hold

- **PASS1 R1's evidence.** It justified the intent layer with SWE-agent's +10.7pp, but that ablation is about the *editing/search/viewer* interface, not intent-vs-primitive granularity. The direct evidence for consolidation is Anthropic ("Tools can consolidate functionality…") and OpenAI ("Combine functions that are always called in sequence"). Direction holds; reasoning corrected.
- **PASS1 R5 ("Address everything by ULID, never by path").** Too absolute and self-contradictory: a document is still addressed by path, and JSON Pointer (RFC 6901) is the standard for leaf edits. What the sources support is *object identity by stable id* plus *uniquely-anchored edits*; Claude's error-proofing was making paths *absolute*, not removing them ("we simply made the tool always require an absolute path").
- **PASS1 R6's framing of "either too little or too much".** Holds verbatim for SWE-agent's viewer, but it is a *file-viewer window* result, not a general claim about response verbosity; R6 states it as a viewer/truncation effect.
- **PASS1 R7 omitted a warning.** Terraform's `-out` two-step is right, but "Terraform does not automatically roll back a partially-completed apply" — we must not imply atomicity we do not have.
- **PASS1's single-number framing of SWE-bench Lite.** Real: 18.00% (54/300); quoting only the full-set 12.47% obscures the ablation baseline (11.00 shell-only), so R1/R6 state both.

## 4. Contradictions (both sides cited)

- **Tailored ACI vs minimal tools.** SWE-agent (GPT-4 Turbo, 2024): purpose-built ACI gives "64% relative increase" over shell-only. Anthropic (Claude 3.5 Sonnet, 2025): "49% on SWE-bench Verified ... with a simple prompt and two general purpose tools." Investment pays most for weaker models; keep primitives simple and consolidate only where evals show gains.
- **Spend tokens vs save tokens.** Multi-agent post: "token usage by itself explains 80% of the variance" (more tokens = better). Same team: "restrict tool responses to 25,000 tokens". Resolution: spend on reasoning and high-signal content, never raw dumps (R6).
- **Consolidate vs examples.** Anthropic/OpenAI both say consolidate ("Consolidate related operations into fewer tools"); OpenAI also says examples help, then warns "Adding examples may hurt performance for reasoning models". Treat `input_examples` as eval-gated (R7).
- **Whole-file vs diff vs search/replace.** Aider: whole is simplest but "slow and costly"; diff is efficient but some models need `udiff` to avoid "lazy coding". Claude: string-replacement with a uniqueness check is "highest reliability". No universal winner — tie the format to the model and to anchor uniqueness (R5).
- **Client-key idempotency vs server-tracked ownership.** Stripe dedupes retries with a client UUID (v1 keys pruned after 24h); Kubernetes/Terraform track field ownership server-side. Both are idempotence; choose per layer (R3 client retries, R8 multi-writer).
- **Scalability of the tool surface.** OpenAI: keep it under ~20 and use tool search; Anthropic: fewer capable tools; MCP: paginate `tools/list` and namespace to disambiguate collisions. Agreement on smallness; different mechanisms.

## 5. Not applicable (one line each)

- MCP `x-mcp-header` parameter→HTTP-header mirroring: transport concern; our CLI/FFI is local (CONTEXT §11.3).
- Terraform state locking, workspaces, `-parallelism`: single-user, single-campaign document model.
- Multi-agent parallel subagents: one agent against one campaign document; concurrent writers are the R8 case, not fan-out.
- MCP elicitation / multi-round-trip input and Stripe v1-vs-v2 idempotency differences: no remote consent flow or hosted API.
- Godot `SceneTree` runtime semantics (pre/post-order, `_process`): we author, we do not run the scene tree.
- Kubernetes RBAC, field-manager HTTP transport, `application/apply-patch+yaml`: only the ownership concept transfers.

## Agent log

- 2026-10-16 (UTC) · claude-code/bookkeeper + T099 fold-in · Keeps the deep pass's agent-authoring-interface findings as the T099 record; the V9 check confirmed every claim, so no markers were needed.
