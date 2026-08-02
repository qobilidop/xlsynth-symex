# Project guidance

- Treat `docs/design.md` as the authoritative design document and keep current
  conclusions there. Use `docs/research.md` for prior art and `docs/notes/` for
  historical material.
- Keep this repository scoped to symbolic evaluation of pure XLS functions;
  whole-processor symbolic execution belongs downstream.
- An AI agent that materially co-authors a commit should add a `Co-Authored-By`
  trailer identifying itself and the model actually used. Each agent must use
  its own identity and valid attribution address; never attribute another
  agent's work or blindly reuse a model name. For example:

  ```text
  Co-Authored-By: Codex GPT-5.6 Sol <codex@openai.com>
  Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
  ```
