<div align="center">

# agent-browser-cli

A browser perception and control CLI for agents, turning a real Chrome session into reusable tab scanning, page JavaScript, Cookie, CDP, and screenshot capabilities.

Browser perception · Page control · Chrome session reuse · CDP · Conditional wait · Agent Skill integration

<p>
  <a href="https://github.com/sleepinginsummer/agent-browser-cli"><img src="https://img.shields.io/badge/CLI-agentbrowsercli-2ea44f" alt="CLI agentbrowsercli"></a>
  <a href="https://github.com/sleepinginsummer/agent-browser-cli/blob/main/LICENSE"><img src="https://img.shields.io/badge/License-MIT-green" alt="License MIT"></a>
  <a href="https://github.com/sleepinginsummer/agent-browser-cli"><img src="https://img.shields.io/badge/sys-win%2Fmac%2Flinux-0078D6?labelColor=0078D6&color=C0C0C0" alt="sys win/mac/linux"></a>
  <a href="https://github.com/sleepinginsummer/agent-browser-cli/releases"><img src="https://img.shields.io/badge/release-v0.3.1--beta.1-orange" alt="release v0.3.1-beta.1"></a>
  <a href="https://github.com/sleepinginsummer/agent-browser-cli/pulls"><img src="https://img.shields.io/badge/PRs-welcome-brightgreen" alt="PRs welcome"></a>
</p>

[AI One-Line Install](#ai-one-line-install) · [Manual Installation](#manual-installation) · [Chrome Extension](#chrome-extension) · [Update](#update) · [Changelog](CHANGELOG.md) · [Uninstall](#uninstall) · [Friendly Links](#friendly-links)

[中文](README.md) | English

</div>

`agent-browser-cli` is a browser perception and control tool for agents. It connects to the user's real Chrome browser through a Chrome extension, preserving login state and cookies while providing tab scanning, page JavaScript execution, cookie reading, CDP control, screenshots, file uploads, dropdown clicks, and related capabilities.

This project is not Selenium or Playwright. It is better suited for helping agents read pages accurately and perform actions inside an existing browser session.

## Project Info

- Current version: `0.3.1-beta.1`
- Supported platforms: Windows (including WSL) / Mac / Linux
- Browser: Chrome, with the `assets/tmwd_cdp_bridge` extension loaded
- Linux prerequisite: the local Chrome / Chromium build must support loading extensions
- WSL prerequisite: use `WSL 2.0.0+`, and preferably enable `networkingMode=mirrored` on Windows `11 22H2+` so WSL can reach the host Chrome bridge service on `localhost`

## Acknowledgements

The browser control capability in this project was extracted and adapted from the Web toolchain in [GenericAgent](https://github.com/lsdefine/GenericAgent), including ideas and implementation around `TMWebDriver`, `simphtml`, and the `tmwd_cdp_bridge` extension.

Thanks to the GenericAgent project for the browser bridge, page simplification, CDP control, and practical SOPs. This repository reorganizes and enhances that work for standalone usage and CLI invocation.

## AI One-Line Install

```text
Please read https://github.com/sleepinginsummer/agent-browser-cli/blob/main/AI_INSTALL.md, follow the instructions to install the CLI, load the Chrome extension, and add `skills/agent-browser-cli/SKILL.md`.
```

## Improvements

- Extracted browser control capability from GenericAgent and exposed it as a CLI for Codex, Claude Code, and OpenCode. The GenericAgent browser extension can be reused; if this extension and the GenericAgent extension are loaded at the same time, they should use different extension ports so they do not both connect to the same `TMWebDriver` service.
- Avoids reinitializing the browser connection for every command.
- Adds a startup lock to avoid repeated low-level port binding when multiple CLI commands start concurrently.
- Adds the skill `skills/agent-browser-cli/SKILL.md` for AI usage reference.
- Includes several optimizations to reduce command execution time.
- Rust implementation for the CLI side.

## What It Can Do

1. Automated testing
   It can reuse a real browser environment for page-flow validation, form submission, button clicks, navigation checks, and login-state page testing.
2. Frontend page debugging
   It can read the DOM, execute JS, inspect page state, and capture screenshots to help locate frontend interaction, rendering, and data issues, including backend API integration problems.
3. Page style debugging
   It can execute JS on real pages to modify DOM / CSS and temporarily validate styles, layout, and interaction effects. It is more of a debugging assistant than a complete design tool.
4. Web data extraction
   It can read page content, tables, lists, cookies, and API-related state, making it suitable for extracting data from pages that require login state.
5. Browser operation scripting
   It can chain operations such as opening pages, switching tabs, executing JS, taking screenshots, and uploading files into scripts for repetitive web tasks.
6. Agent-assisted web admin operation
   It is suitable for letting AI Agents operate existing web tools such as admin consoles, configuration pages, low-code platforms, and form systems.
7. Page structure analysis
   It can simplify HTML and identify main content areas and list structures, helping Agents understand complex pages faster.
8. Security research and reverse-engineering assistance
   It can observe page behavior in a real browser session, execute debugging scripts, and read frontend state to assist analysis of frontend logic and API calls.

## Its Capabilities

1. Scan current Chrome tabs and get page titles, URLs, and tab IDs.
2. Switch to a specified tab and reuse existing pages and login state.
3. Open new tabs and directly visit target URLs.
4. Execute JavaScript in pages and read DOM, forms, state, and page data.
5. Read cookies from the current page for login-state related tasks.
6. Call Chrome CDP capabilities for lower-level page control.
7. Capture page screenshots for visual inspection and page confirmation.
8. Upload local files to web file picker inputs.
9. Operate common page elements such as dropdowns, buttons, and forms.

## Layout

```text
.
├── Cargo.toml                    # Rust crate config
├── src/                          # Rust CLI / daemon / bridge
├── assets/tmwd_cdp_bridge/       # Chrome MV3 extension
├── assets/simphtml_opt.js        # Page simplification script
├── assets/simphtml_find_list.js  # List detection script
├── npm/                          # npm launcher scripts
└── skills/agent-browser-cli/     # skill
```

## Manual Installation

### npm

```bash
npm install -g @sleepinsummer/agent-browser-cli
agent-browser-cli tabs
```

### Build From Source

```bash
cargo build --release
./target/release/agent-browser-cli tabs
```


## Chrome Extension

1. Recommended: download `chrome-extensions.zip` from the [latest Release](https://github.com/sleepinginsummer/agent-browser-cli/releases/latest), extract it, open `chrome://extensions` in Chrome, enable `Developer mode`, click `Load unpacked`, and select the extracted `tmwd_cdp_bridge` directory.

2. When building from local source, you can also load this extension directory directly:

```text
assets/tmwd_cdp_bridge
```

3. Chrome needs at least one normal web page tab open. Do not leave it only on `about:blank` or `chrome://` pages.
4. After the extension is connected, a Chrome extension tip badge appears on the right side of the page. The badge position is draggable and expands on hover. It auto-hides after 10 seconds without commands, and you can also click `Hide for this session` to hide it manually. Manual hiding lasts for the current service connection cycle and resets after the service disconnects after about 300 seconds and reconnects.

### Custom Chrome Extension WebSocket Port

- `18768`: default extension WebSocket port, used by the Chrome extension. It can be changed with `agent-browser-cli set-extension-port <port>`.
- `18765`: common GenericAgent `TMWebDriver` port. If this extension and the GenericAgent extension are both enabled in Chrome, do not point both extensions at the same port. Otherwise both extensions may try to control the same tab's `chrome.debugger` session and fail with `Another debugger is already attached`.
- `18767`: CLI HTTP API port, used by the CLI to reuse the session. It cannot be used as the extension port.

Change the extension port from CLI:

```bash
agent-browser-cli set-extension-port 18768
```

This command writes the config file. If the daemon is running, it restarts the daemon so the new port takes effect immediately.

You can also edit the config file manually. The config file is `~/.agent-browser-cli/config.json`. It is created automatically when missing:

```json
{
  "extension_port": 18768
}
```

Manual edit example:

```json
{
  "extension_port": 18768
}
```

After manually editing the config file, run `agent-browser-cli restart` so the daemon listens on the new port.

The Chrome extension popup can also update the extension port and reconnect immediately. The popup port must match the CLI `extension_port` config.

If the same Chrome Profile already has GenericAgent's `TMWD CDP Bridge` extension installed, keep GenericAgent on `18765` and let this project use its default `18768`. Both extensions can exist in the same profile, but avoid running CDP fallback operations against the same tab from both tools at the same time.

### Dialog Suppression

The extension no longer rewrites page `alert` / `confirm` / `prompt` globally. Native dialogs are suppressed only while a CLI page script command is running, then restored after the command finishes.

## Quick Check

```bash
agent-browser-cli tabs
agent-browser-cli open https://www.baidu.com
```

On success, it returns:

```json
{
  "ok": true,
  "result": {
    "status": "success",
    "metadata": {
      "tabs_count": 1
    }
  }
}
```

### Profile Label

When multiple Chrome Profiles or browser instances are connected, `profile_id` and `browser_id` are long. You can set a short label for each Chrome Profile and then use `--profile <label>`.

```bash
agent-browser-cli lookup tab <tabId>
agent-browser-cli lookup browser <browser_id>
agent-browser-cli profile-label set work --profile <profile_id>
agent-browser-cli tabs --profile work
```

You can also set Profile Label from the extension popup in the corresponding Chrome Profile. The label is only an alias; internal routing still uses `browser_id:profile_id:tab_id`. If a label matches multiple profiles in the current daemon, the CLI reports ambiguity. Prefer setting labels via CLI because it checks uniqueness across currently connected profiles; the popup is a local convenience entry and cannot guarantee cross-profile uniqueness. `tabtree` uses compact output by default; pass `--full` for full URLs and session keys.

## Common Commands

The README only keeps the quick entry point. For the full command list and browser operation SOP, see [skills/agent-browser-cli/SKILL.md](./skills/agent-browser-cli/SKILL.md).

```bash
agent-browser-cli tabs
agent-browser-cli tabtree
agent-browser-cli tabtree --full
agent-browser-cli tabtree --profile <profile_label>
agent-browser-cli tabtree --tab <tabId>
agent-browser-cli lookup tab <tabId>
agent-browser-cli lookup browser <browser_id>
agent-browser-cli profile-label set work --profile <profile_id>
agent-browser-cli profile-label clear --profile <profile_id>
```

## Update

For the full version history, see [CHANGELOG.md](./CHANGELOG.md).

AI one-line update:

```text
Please read https://github.com/sleepinginsummer/agent-browser-cli/blob/main/AI_INSTALL.md, follow the instructions to update the CLI, download the extension zip again to the user-specified location, ask the user to manually load the Chrome extension, and update the related SKILL.md.
```

If the Chrome extension has updates, download the zip again, overwrite the existing files, and reload the `assets/tmwd_cdp_bridge` extension in `chrome://extensions`.

## Uninstall

Stop the long-lived service first:

```bash
agent-browser-cli stop
```

Then clean up as needed:

```bash
rm -f .agent-browser-cli.log .agent-browser-cli.lock
rm -rf ~/.agents/skills/agent-browser-cli
```

Finally, remove the `TMWD CDP Bridge` extension from Chrome's extension management page, or remove the loaded `assets/tmwd_cdp_bridge` extension configuration.

## Friendly Links

- [LINUX DO - A New Ideal Community](https://linux.do/)
- [GenericAgent--复旦团队研发|仅仅~3K行代码 Self-Evolving Agent](https://github.com/lsdefine/GenericAgent/tree/main)

## License

MIT License. See [LICENSE](./LICENSE).
