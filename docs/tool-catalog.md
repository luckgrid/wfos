# Tool catalog

The open-source tools, libraries, skills, and crates that WfOS builds on or plans to include,
grouped by role. Each entry notes its status and where it fits.

Status legend: **core** (installed by the native-toolchain (Panoply) today) · **recommended-default** (a native-toolchain default,
still swappable) · **optional** (detected/swappable) · **planned** (intended, not yet wired) ·
**inspiration** (reference, not a dependency).

---

## Core dependencies — Unix substrate (native-toolchain / Panoply)

The low-level CLI layer. Defaults are installed by `panoply bootstrap`; alternatives are
detected if present. See [native-toolchain.md](native-toolchain.md).

| Tool | Status | Role | License |
|------|--------|------|---------|
| [git](https://git-scm.com/) | core | version control | GPL-2.0 |
| [gh](https://cli.github.com/) | core | GitHub CLI | MIT |
| [OpenSSH](https://www.openssh.com/) | core | secure remote access and keys | BSD |
| [fzf](https://github.com/junegunn/fzf) | core | fuzzy finder and selection | MIT |
| [tmux](https://github.com/tmux/tmux) | core | persistent terminal sessions | ISC |
| [starship](https://github.com/starship/starship) | core | cross-shell prompt context | ISC |
| [zoxide](https://github.com/ajeetdsouza/zoxide) | core | smarter directory jumping | MIT |
| [eza](https://github.com/eza-community/eza) | core | modern `ls` | MIT |
| [bat](https://github.com/sharkdp/bat) | core | `cat` with syntax highlighting | MIT/Apache-2.0 |
| [ripgrep](https://github.com/BurntSushi/ripgrep) | core | fast recursive search | MIT/Unlicense |
| [fd](https://github.com/sharkdp/fd) | core | fast file find | MIT/Apache-2.0 |
| [jq](https://github.com/jqlang/jq) | core | JSON processor | MIT |
| [tldr (tealdeer)](https://github.com/tldr-pages/tealdeer) | core | practical command cheatsheets | MIT/Apache-2.0 |
| [btop](https://github.com/aristocratos/btop) | core | resource/process monitor (top/htop replacement) | Apache-2.0 |
| [dua](https://github.com/Byron/dua-cli) | core | disk-usage visualizer (`du` replacement; chosen over the external `dust` CLI, which clashed with the former native-toolchain brand) | MIT |
| [direnv](https://direnv.net/) | core | per-directory environment activation | MIT |
| [shellcheck](https://www.shellcheck.net/) | core | shell script linting | GPL-3.0 |
| [zsh-autosuggestions](https://github.com/zsh-users/zsh-autosuggestions) | core | async command suggestions from history (sourced plugin) | MIT |
| [zsh-syntax-highlighting](https://github.com/zsh-users/zsh-syntax-highlighting) | core | real-time command-line syntax highlighting (sourced plugin) | BSD-3-Clause |
| [pass](https://www.passwordstore.org/) | core | Unix password store (agent-blocked) | GPL-2.0 |
| [age](https://github.com/FiloSottile/age) / [sops](https://github.com/getsops/sops) | optional | file/secret encryption (config files: sops+age; interactive: pass) | BSD-3 / MPL-2.0 |
| [gitleaks](https://github.com/gitleaks/gitleaks) | optional | scan staged/committed files for leaked secrets (pre-commit gate candidate) | MIT |
| [chezmoi](https://www.chezmoi.io/) | optional | cross-machine dotfile manager (complements the native-toolchain symlink bootstrap) | MIT |
| [RTK (Rust Token Killer)](https://github.com/rtk-ai/rtk) | recommended-default | LLM output compressor — proxies dev commands, cuts tokens 60-90% (native-toolchain `agent` module; swappable via `PANOPLY_RTK`) | Apache-2.0 |
| [choose](https://github.com/theryangeary/choose) | optional | field selector — human-friendly `cut`/`awk` alternative | MIT |
| [zsh-autocomplete](https://github.com/marlonrichert/zsh-autocomplete) | optional | real-time menu completion (sourced plugin; can conflict) | MIT |
| [jj](https://github.com/jj-vcs/jj) | optional | Git-compatible VCS alternative | MIT/Apache-2.0 |
| [skim](https://github.com/skim-rs/skim) | optional | Rust fuzzy finder (fzf alternative) | MIT |
| [zellij](https://github.com/zellij-org/zellij) | optional | terminal workspace (tmux alternative) | MIT |
| [Fabric](https://github.com/danielmiessler/fabric) | planned | AI-augmentation patterns runnable from the shell | MIT |

Secrets are tiered by concern: **pass** for interactive CLI logins/keys, **sops + age** for
configuration files checked into git (encrypts values, keeps keys and diffs readable). See
[`packages/panoply/dotfiles/SECRETS.md`](../packages/panoply/dotfiles/SECRETS.md) and
[native-toolchain.md](native-toolchain.md#modules).

Dotfiles practice and bootstrap inspiration: [dotfiles.github.io](https://dotfiles.github.io/)
(utilities, frameworks, bootstrap, tips). WfOS integrates low-level tooling in this spirit —
small, composable, dotfile-friendly.

## Monorepo & toolchains

| Tool | Status | Role |
|------|--------|------|
| [proto](https://moonrepo.dev/proto) | core | pins and installs workspace toolchains (`.prototools`) |
| [moon](https://moonrepo.dev/moon) | core | project graph, task running, caching |
| [starbase](https://github.com/moonrepo/starbase) | planned | Rust framework for the runtime-controller (Cthulhu) CLI |
| [mise](https://mise.jdx.dev/) | core | runtime/version manager (native-toolchain default) |

See [monorepo.md](monorepo.md).

## Runtime engine — Rust crates

The engine under the [runtime-controller (Cthulhu)](runtime-controller.md); see [runtime-architecture.md](runtime-architecture.md).

| Crate / spec | Status | Role |
|--------------|--------|------|
| [Tokio](https://crates.io/crates/tokio) | planned | async runtime + subprocess proxying |
| [clap](https://crates.io/crates/clap) | planned | CLI argument/command parsing |
| [Ratatui](https://crates.io/crates/ratatui) | planned | terminal UI (TUI phase) |
| [Serde](https://crates.io/crates/serde) | planned | config/profile parsing (TOML-first) |
| [Orka](https://crates.io/crates/orka) | planned | pluggable async DAG workflow engine (candidate) |
| [Zenoh](https://crates.io/crates/zenoh) | planned | pub/sub data fabric (federation/multi-process) |
| [rmcp](https://crates.io/crates/rmcp) / [MCP](https://modelcontextprotocol.io) | planned | expose native commands as LLM tools |

## Web & docs publishing

| Tool | Status | Role |
|------|--------|------|
| [Zola](https://www.getzola.org/) | planned | static-site generator for `apps/docs` and `apps/web` |
| [Typst](https://typst.app/) | planned | production/publish-grade docs and whitepapers |

See [apps.md](apps.md).

## AI enhancements (selectable in the setup flow)

These are **opt-in**: the planned setup flow ([setup.md](setup.md)) presents them as choices,
each with a description, so a developer or agent installs only what they want. None are
required for WfOS to be useful.

| Tool | Status | What it does |
|------|--------|--------------|
| [RTK (Rust Token Killer)](https://github.com/rtk-ai/rtk) | recommended-default | CLI proxy that compresses command output to cut LLM token use 60–90%; single Rust binary. Now a native-toolchain `agent`-module default (see the Unix-substrate table above); swappable via `PANOPLY_RTK`. |
| [QMD](https://github.com/tobi/qmd) | optional | Local hybrid search (BM25 + vectors + rerank) over markdown collections; CLI/MCP `query` → `get` retrieval for agents without loading full vaults |
| [ponytail](https://github.com/DietrichGebert/ponytail) | optional | forces the simplest, most minimal solution that works (anti-over-engineering) |
| [drawio-skill](https://github.com/Agents365-ai/drawio-skill) | optional | generate diagrams/flowcharts as draw.io files and export images |
| [SkillSpector](https://github.com/nvidia/skillspector) | optional | security scanner for AI agent skills — detect vulnerabilities and malicious patterns |
| [Handy](https://github.com/cjpais/Handy) | optional | free, offline, extensible speech-to-text |
| [improve](https://github.com/shadcn/improve) | optional | survey a codebase and produce prioritized, self-contained improvement plans |
| [OpenRouter](https://openrouter.ai/) | optional | low-level model-adapter / AI-routing layer for building tools (not a high-level agent UI) |
| [Fabric](https://github.com/danielmiessler/fabric) | optional | crowdsourced AI prompt "patterns" usable anywhere |

OpenRouter is the intended substrate for model adapters and routing inside WfOS-built tools —
a primitive to build on, not a replacement for an agent CLI.

**RTK** compresses shell output; **QMD** compresses document retrieval — both cut agent token use,
but at different layers. See [workflow-apps.md](workflow-apps.md#agent-retrieval--qmd).

## AI engine / writing (docs-only — see workflow-apps.md)

Recommended for local-first, native writing and AI-assisted document workflows. These are
**documented recommendations**, not native-toolchain-managed tools — install them yourself. Full guide and
quick-start in [workflow-apps.md](workflow-apps.md).

| Tool | Status | Role | License |
|------|--------|------|---------|
| [aichat](https://github.com/sigoden/aichat) | docs-only | Rust LLM CLI — RAG over notes, sessions, `--serve` local API | MIT/Apache-2.0 |
| [QMD](https://github.com/tobi/qmd) | docs-only | Agent retrieval index over local markdown (collections, context, line-range `get`) | MIT |
| [Ollama](https://ollama.com/) | docs-only | run open models locally, no Docker | MIT |
| [OpenRouter](https://openrouter.ai/) | docs-only | cloud-model routing for high-tier models (aichat provider) | hosted |
| [Typst](https://typst.app/) | docs-only | compile markdown/Typst into publish-grade PDFs | Apache-2.0 |
| [tinymist](https://github.com/Myriad-Dreamin/tinymist) | docs-only | Typst language server (editor integration) | Apache-2.0 |
| [Fabric](https://github.com/danielmiessler/fabric) | optional | crowdsourced AI prompt "patterns" usable from the shell | MIT |

## Native / local apps (docs-only — see workflow-apps.md)

Core writing, note-taking, and session-restoration apps. Documented recommendations, not native-toolchain
dependencies; the full guide is in [workflow-apps.md](workflow-apps.md).

| App | Status | Role | License |
|-----|--------|------|---------|
| [Logseq](https://logseq.com/) | recommended | quick notes, ideas, research, journaling | AGPL-3.0 |
| [Obsidian](https://obsidian.md/) | recommended | larger docs, specs, structured vaults | proprietary (free tier) |
| [SilverBullet](https://silverbullet.md/) | optional | OSS, hackable single-process markdown workspace | MIT |
| [Kilo Code](https://github.com/Kilo-Org/kilocode) | reference | context/session storage and knowledge transfer | Apache-2.0 |
| [Reor](https://github.com/reorproject/reor) | reference | local AI notes app (archived Mar 2026; reference only) | AGPL-3.0 |

## WASM / WASI runtimes

The portable execution target; see [portable-component-runtime.md](portable-component-runtime.md).

| Project | Status | Role |
|---------|--------|------|
| [WASI](https://github.com/WebAssembly/WASI) | planned | system interface spec for portable components |
| [Wasmtime](https://wasmtime.dev/) | core | WASM/WASI runtime (native-toolchain `wisp` module — not the portable-component-runtime product Wisp) |
| [Spin](https://github.com/spinframework/spin) | inspiration | event-driven WASM apps without a container layer |
| [wasmCloud](https://github.com/wasmcloud/wasmcloud) | inspiration | distributed platform for WASM components |
| [Hyperlight Wasm](https://opensource.microsoft.com/blog/2025/03/26/hyperlight-wasm-fast-secure-and-os-free/) | inspiration | micro-VM isolation for WASM |

## Network & security

Local-first today; relevant when work spans machines (federation).

| Tool | Status | Role |
|------|--------|------|
| [WireGuard](https://www.wireguard.com/) | planned | fast, modern VPN tunnel |
| [Tailscale](https://tailscale.com/) | planned | managed WireGuard mesh |
| [Headscale](https://github.com/juanfont/headscale) | planned | self-hosted Tailscale control server |
| [SoftEther](https://www.softether.org/) | inspiration | multi-protocol VPN |

## Workflow inspirations

Local-first workflow apps that prove ideas WfOS borrows from (window/session/space layouts,
scattered-knowledge capture). Reference, not dependencies. See the sessions & workspace
restoration section of [workflow-apps.md](workflow-apps.md#sessions--workspace-restoration).

| App | Idea |
|-----|------|
| Decks | bringing scattered knowledge back together |
| [Freeter](https://freeter.io/) | organizing tools and resources per workflow |
| [FlashSpace](https://github.com/wojciech-kulik/FlashSpace) | fast virtual workspace/space switching |
| [Spaces](https://spacesformac.xyz/) | per-context window and app layouts |
