# Project guidance

- Treat `docs/user/guide.md` as the source of truth for public behavior and
  `docs/developer/design.md` as the source of truth for internal architecture.
  Keep validation claims in `docs/developer/verification.md` and upstream
  integration considerations in `docs/developer/upstreaming.md`. Historical
  reasoning belongs in Git and pull requests, not permanent working-note files.
- Keep this repository scoped to symbolic evaluation of pure XLS functions;
  whole-processor symbolic execution belongs downstream.
- Run repository development and verification commands through `./dev.sh`.
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
