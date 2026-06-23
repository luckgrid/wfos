# Core workflow apps & tools

The essential native, local-first apps and tools WfOS recommends for low-level writing,
note-taking, and AI-assisted document workflows — and how they fit together. These are
**recommendations**, not dependencies: none are installed by `dust bootstrap`, and the
markdown-on-disk source of truth keeps every choice swappable.

The detailed product concept that builds on this stack (Mindflow) is a future, decoupled
product — not part of WfOS today.

Status legend: **recommended** (the WfOS starting point) · **optional** (swappable
alternative) · **reference** (noted, not endorsed for new work).

---

## The stack at a glance

| Layer | Tool | Role |
|-------|------|------|
| Quick capture | [Logseq](https://logseq.com/) | fast notes, ideas, research, daily journaling |
| Long-form docs | [Obsidian](https://obsidian.md/) | larger docs, specs, structured vaults |
| Typeset / publish | [Typst](https://typst.app/) | compile markdown/Typst into polished PDFs |
| Agent retrieval | [QMD](https://github.com/tobi/qmd) | index local markdown; hybrid search; fetch snippets/line ranges via CLI or MCP |
| AI engine (local) | [aichat](https://github.com/sigoden/aichat) + [Ollama](https://ollama.com/) | RAG over notes, sessions, local models, no Docker |
| AI engine (cloud) | [OpenRouter](https://openrouter.ai/) | high-tier cloud models when local isn't enough |

Everything reads and writes **plain markdown in a directory you own** (git-tracked, readable
by agents, devs, and sessions). The editor and AI engine are layers on top of that directory,
not the source of truth.

---

## Writing & notes

### Logseq — quick notes, ideas, research (recommended)

[Logseq](https://logseq.com/) (AGPL-3.0) is an outliner for fast, low-friction capture: daily
journals, fleeting ideas, research snippets, and linked thoughts. Its block model and backlinks
make it ideal for the early "catch the idea before it's gone" stage. Files are local markdown.

### Obsidian — larger docs and specs (recommended)

[Obsidian](https://obsidian.md/) (proprietary, free for personal use) is the home for larger,
more structured work: specs, briefs, foundational docs, and long-form writing. Its mature plugin
ecosystem (including local-AI plugins) and vault model suit documents that graduate out of quick
capture. Vaults are plain markdown folders.

**How they pair:** capture in Logseq → promote anything worth developing into an Obsidian vault
where it becomes a doc/spec. Both sit over markdown, so the same files stay readable to the AI
engine and to agents.

### SilverBullet — OSS/hackable alternative (optional)

[SilverBullet](https://silverbullet.md/) (MIT) is a single-process, self-hosted markdown
workspace that is highly scriptable. Consider it if you want a fully open-source, extensible base
to build custom writing workflows on rather than a packaged app.

---

## Typeset & publish — Typst

[Typst](https://typst.app/open-source/) (Apache-2.0) compiles markup into publish-grade PDFs much
faster than LaTeX, with a modern scripting language. Use it as the final step that turns markdown
notes/specs into polished documents and whitepapers. The [tinymist](https://github.com/Myriad-Dreamin/tinymist)
language server adds editor integration (preview, completion) in VS Code/Neovim.

---

## AI engine — aichat + Ollama, with OpenRouter for cloud

[aichat](https://github.com/sigoden/aichat) (MIT/Apache-2.0) is a single Rust binary that turns a
notes directory into an AI workspace:

- **RAG** over your markdown directory (`aichat --rag <name>`),
- **sessions** and **roles** for context-aware, repeatable interactions,
- **function calling / MCP / agents** for tool use,
- a built-in local API (`aichat --serve`) any editor plugin or future UI can point at.

[Ollama](https://ollama.com/) (MIT) runs open models locally with no Docker required, keeping
notes private by default. [OpenRouter](https://openrouter.ai/) is configured as an additional
provider so you can route to high-tier cloud models when a task needs more than a local model can
give — same `aichat` interface, different backend.

> Privacy posture: local-first by default (Ollama); cloud is opt-in per request via the OpenRouter
> provider. Nothing leaves the machine unless you choose a cloud model.

---

## Agent retrieval — QMD

[QMD](https://github.com/tobi/qmd) (Query Markup Documents, MIT) is a local hybrid search engine
for markdown collections. Agents search first (`query`, `search`), then retrieve only the sections
they need (`get`, `multi_get` with line ranges and docids). The index and full corpus stay
off-context; snippets lead, full text follows on demand. Cite docids and line numbers in answers.

Suggested Workstreams collections (configure yourself):

- `Plan/bin/` — fleeting capture / workbench docs
- `Plan/src/` — validated specs and foundation docs
- `wfos/docs/` — workspace reference
- Obsidian vault root, if separate from Plan

Use `qmd context add` per collection so search results carry human-readable corpus labels.

```bash
npm install -g @tobilu/qmd   # Node >= 22; macOS: brew install sqlite
qmd collection add ~/path/to/markdown --name mynotes
qmd context add qmd://mynotes "Description for ranking"
qmd update && qmd embed
qmd query "..." -n 5          # hybrid search
qmd get "#docid:120:40"       # line-range retrieval
```

Agent integration (install yourself; WfOS does not manage these):

- Official skill: `npx skills add tobi/qmd --skill qmd` or copy from the repo `skills/qmd/`
- MCP: `qmd mcp` (stdio) or `qmd mcp --http --daemon` for a shared server
- MCP setup for Cursor/OpenClaw: [mcp-setup.md](https://github.com/tobi/qmd/blob/main/skills/qmd/references/mcp-setup.md)

First run downloads ~2GB of local GGUF models into `~/.cache/qmd/`. Run `qmd doctor` if
model-backed commands fail. QMD indexes and retrieves; aichat chats and writes back — use both or
either.

---

## How they fit together

```mermaid
flowchart LR
  Notes[Markdown notes dir<br/>Logseq + Obsidian]
  QMD[QMD<br/>index and search]
  Agents[Cursor / OpenClaw agents]
  AIChat[aichat<br/>RAG + sessions + serve]
  Ollama[Ollama<br/>local models]
  OR[OpenRouter<br/>cloud models]
  Typst[Typst<br/>compile to PDF]

  Notes --> QMD
  QMD -->|"snippets then line ranges"| Agents
  Notes --> AIChat
  AIChat --> Ollama
  AIChat --> OR
  AIChat --> Notes
  Notes --> Typst
```

1. **Capture** ideas/research in Logseq; develop docs and specs in Obsidian — all plain markdown
   in one directory.
2. **Index** with QMD: `collection add`, `update`, `embed`; agents search before reading whole
   files.
3. **Augment** with aichat: RAG and sessions read that directory; responses and structured
   outputs are written back as markdown.
4. **Route** to Ollama locally by default, or OpenRouter for high-tier cloud models when needed.
5. **Publish** finished specs/docs through Typst.

---

## Workstreams placement

Recommended namespace mapping for the writing stack:

```mermaid
flowchart LR
  subgraph PlanNs [Plan — Decisions]
    Capture[Logseq quick capture]
    Specs[Obsidian specs and foundation docs]
  end
  subgraph BrandNs [Brand — Expressions]
    Publish[Typst PDF output]
  end
  Capture --> Specs
  Specs --> Publish
  QMD[QMD retrieval] --> PlanNs
  AIChat[aichat RAG] --> PlanNs
```

- **Plan/bin/** — fleeting capture, scratch research (Logseq-oriented flow)
- **Plan/src/** — validated specs, foundation docs (Obsidian vault canon)
- **Brand/lib/** — published PDFs and export copies (Typst output)

---

## Quick start (not part of `dust bootstrap`)

These are documented installs you run manually — WfOS does not install or manage them.

```bash
# CLI tools (Homebrew)
brew install aichat ollama typst

# Agent retrieval (npm; Node >= 22)
npm install -g @tobilu/qmd

# Apps (Homebrew casks)
brew install --cask obsidian logseq

# Pull a local model and start the daemon
ollama pull llama3.1          # or any model from https://ollama.com/library
ollama serve                  # background local model server

# Point aichat at a local model + index your notes for RAG
#   ~/.config/aichat/config.yaml — set the Ollama client and an OpenRouter client:
#     clients:
#       - type: openai-compatible
#         name: ollama
#         api_base: http://localhost:11434/v1
#       - type: openai-compatible
#         name: openrouter
#         api_base: https://openrouter.ai/api/v1
#         api_key: <OPENROUTER_API_KEY>
aichat --rag notes            # build/query a RAG over your notes directory
aichat --serve                # expose a local OpenAI-compatible API + playground

# QMD — index markdown for agent search/retrieval (see Agent retrieval section above)
qmd collection add ~/path/to/markdown --name mynotes
qmd update && qmd embed
```

License notes: aichat (MIT/Apache-2.0), Ollama (MIT), QMD (MIT), Typst (Apache-2.0), Logseq (AGPL-3.0),
Obsidian (proprietary, free tier), SilverBullet (MIT), OpenRouter (hosted service).

---

## Sessions & workspace restoration

Restore where you left off — windows, tabs, apps, and terminal context — across a workflow.

| Tool | Idea | Status |
|------|------|--------|
| [FlashSpace](https://github.com/wojciech-kulik/FlashSpace) | fast virtual workspace/space switching (macOS) | optional |
| [Freeter](https://freeter.io/) | organize tools and resources per workflow | optional |
| Decks | bring scattered knowledge back together | reference |
| [Spaces](https://spacesformac.xyz/) | per-context window and app layouts | reference |
| [tmux](https://github.com/tmux/tmux) / [zellij](https://github.com/zellij-org/zellij) | persistent terminal sessions (Dust `session` module) | core |
| [Kilo Code](https://github.com/Kilo-Org/kilocode) | reference for context/session storage and knowledge transfer between sessions/agents | reference |

Terminal sessions are already part of the Dust [`session` module](native-substrate.md#modules); GUI
window/space restoration is handled by the apps above today and is a candidate for future Kraken
session management.

---

## Relationship to WfOS

- These apps are **documented recommendations**, not Dust-managed tools — install them yourself.
- The markdown directory is the contract; editors and AI engines are swappable layers over it.
- The deeper idea-capture → spec product concept (Mindflow) is intentionally decoupled and lives
  in the Workstreams Plan workstream. WfOS may install it eventually, but is not coupled to it now.
- See [tool-catalog.md](tool-catalog.md) for the full grouped catalog and [native-substrate.md](native-substrate.md) for
  the native CLI substrate.
