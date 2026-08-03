# Project guidance

- Treat `docs/design.md` as the authoritative v1 specification. Keep changing
  implementation facts in `docs/status.md`, the current-to-v1 work sequence in
  `docs/roadmap.md`, and release evidence requirements in
  `docs/verification.md`. Use `docs/research.md` for prior art and
  `docs/notes/` for historical material.
- Keep this repository scoped to symbolic evaluation of pure XLS functions;
  whole-processor symbolic execution belongs downstream.
- An AI agent that materially co-authors a commit should add a `Co-Authored-By`
  trailer identifying itself and the model actually used. Each agent must use
  its own identity and valid attribution address; never attribute another
  agent's work or blindly reuse a model name. Before committing, resolve the
  exact current-session model from session/runtime metadata; a broad family
  description such as "based on GPT-5" is not an exact model identity. If the
  exact model cannot be determined, do not guess; ask the user. For example:

  ```text
  Co-Authored-By: Codex GPT-5.6 Sol <codex@openai.com>
  Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
  ```
