<div align="center">

# agent-browser-cli

面向 Agent 的浏览器感知与控制 CLI，把真实 Chrome 会话变成可复用的标签页扫描、页面 JS、Cookie、CDP 和截图能力。

浏览器感知 · 页面控制 · Chrome 登录态复用 · CDP · 条件等待 · Agent Skill 集成

<p>
  <a href="https://github.com/sleepinginsummer/agent-browser-cli"><img src="https://img.shields.io/badge/CLI-agentbrowsercli-2ea44f" alt="CLI agentbrowsercli"></a>
  <a href="https://github.com/sleepinginsummer/agent-browser-cli/blob/main/LICENSE"><img src="https://img.shields.io/badge/License-MIT-green" alt="License MIT"></a>
  <a href="https://github.com/sleepinginsummer/agent-browser-cli"><img src="https://img.shields.io/badge/sys-win%2Fmac%2Flinux-0078D6?labelColor=0078D6&color=C0C0C0" alt="sys win/mac/linux"></a>
  <a href="https://github.com/sleepinginsummer/agent-browser-cli/releases"><img src="https://img.shields.io/badge/release-v0.3.4-orange" alt="release v0.3.4"></a>
  <a href="https://github.com/sleepinginsummer/agent-browser-cli/pulls"><img src="https://img.shields.io/badge/PRs-welcome-brightgreen" alt="PRs welcome"></a>
</p>

[AI 一句话安装](#ai-一句话安装) · [手动安装](#手动安装) · [Chrome 扩展](#chrome-扩展) · [更新](#更新) · [更新日志](CHANGELOG.md) · [卸载](#卸载) · [友情链接](#友情链接)

中文 | [English](README_EN.md)

</div>

`agent-browser-cli` 是一个面向 Agent 的浏览器感知与控制工具。它通过 Chrome 扩展连接用户真实浏览器，保留登录态和 Cookie，提供标签页扫描、页面 JS 执行、Cookie 读取、CDP 控制、截图、文件上传、下拉框点击等能力。

本项目不是 Selenium / Playwright。它更适合在已有浏览器会话中辅助 Agent 精确读取页面和执行操作。

## 项目信息

- 当前版本：`0.3.1-beta.1`
- 支持平台：Windows（包括 WSL）/ Mac / Linux
- 浏览器：Chrome，需加载拓展 `assets/tmwd_cdp_bridge`
- Linux 支持前提：本机 Chrome / Chromium 需要支持安装扩展
- WSL 支持前提：需使用 `WSL 2.0.0+`，并建议在 Windows `11 22H2+` 下启用 `networkingMode=mirrored`，以便 WSL 连接宿主机 `localhost` 上的 Chrome 桥接服务

## 致谢

本项目的浏览器控制能力提取并改造自 [GenericAgent](https://github.com/lsdefine/GenericAgent) 项目中的 Web 工具链，包括 `TMWebDriver`、`simphtml` 和 `tmwd_cdp_bridge` 扩展相关思路与实现。

感谢 GenericAgent 项目提供的浏览器桥接、页面简化、CDP 控制和实践 SOP。本仓库在此基础上做了面向独立使用和 CLI 调用的整理与增强。

## AI 一句话安装

```text
请阅读 https://github.com/sleepinginsummer/agent-browser-cli/blob/main/AI_INSTALL.md，按说明安装 CLI、加载 Chrome 扩展，并添加 `skills/agent-browser-cli/SKILL.md`。
```

## 改进内容

- 从 GenericAgent 中拆出浏览器控制能力，使用cli 提供给codex、claude code、opencode使用。可以复用 GenericAgent 浏览器插件；如果同时加载本项目扩展和 GenericAgent 扩展，需要使用不同插件端口，避免两个扩展同时连接同一个 `TMWebDriver` 服务。
- 避免每次命令都重新初始化浏览器连接。
- 新增启动锁，避免多个 CLI 并发启动时重复绑定底层端口。
- 增加skill：`skills/agent-browser-cli/SKILL.md`，提供ai参考使用。
- 若干优化，缩短命令执行时间
- rust实现cli端

## 他能做的事情

1. 自动化测试
   可以复用真实浏览器环境做页面流程验证、表单提交、按钮点击、跳转检查、登录态页面测试。
2. 前端页面 Debug
   可以读取 DOM、执行 JS、查看页面状态、截图确认效果，辅助定位前端交互、渲染和数据问题，对接后端接口。
3. 页面样式调试
   可以在真实页面里执行 JS 修改 DOM / CSS，临时验证样式、布局和交互效果，但更偏辅助调试，不是完整设计工具。
4. 网页数据采集
   可以读取页面内容、表格、列表、Cookie 和接口相关状态，适合处理需要登录态的页面数据提取。
5. 浏览器操作脚本化
   可以把打开页面、切换标签页、执行 JS、截图、上传文件等操作串成脚本，做重复性网页任务。
6. Agent 辅助操作网页后台
   适合让 AI Agent 操作管理后台、配置页面、低代码平台、表单系统等已有网页工具。
7. 页面结构分析
   可以简化 HTML、识别主要内容区和列表结构，帮助 Agent 更快理解复杂页面。
8. 安全研究和逆向辅助
   可以在真实浏览器会话里观察页面行为、执行调试脚本、读取前端状态，辅助分析前端逻辑和接口调用。

## 他的能力

1. 扫描当前 Chrome 标签页，获取页面标题、URL 和标签页 ID。
2. 切换到指定标签页，复用已有页面和登录态。
3. 打开新标签页，支持直接访问目标 URL。
4. 在页面中执行 JavaScript，读取 DOM、表单、状态和页面数据。
5. 读取当前页面 Cookie，方便处理登录态相关任务。
6. 调用 Chrome CDP 能力，执行更底层的页面控制。
7. 截取页面截图，用于视觉检查和页面确认。
8. 上传本地文件到网页文件选择框。
9. 操作下拉框、按钮、表单等常见页面元素。

## 目录结构

```text
.
├── Cargo.toml                    # Rust 工程配置
├── src/                          # Rust CLI / 常驻服务 / bridge
├── assets/tmwd_cdp_bridge/       # Chrome MV3 扩展
├── assets/simphtml_opt.js        # 页面简化脚本
├── assets/simphtml_find_list.js  # 列表识别脚本
├── npm/                          # npm 启动脚本
└── skills/agent-browser-cli/     # skill
```

## 手动安装

### npm 安装

```bash
npm install -g @sleepinsummer/agent-browser-cli
agent-browser-cli tabs
```

### 本地源码构建

```bash
cargo build --release
./target/release/agent-browser-cli tabs
```


## Chrome 扩展

1. 推荐从 [最新 Release](https://github.com/sleepinginsummer/agent-browser-cli/releases/latest) 下载 `chrome-extensions.zip`，下载后解压，Chrome 打开 `chrome://extensions`，开启“开发者模式”，点击“加载已解压的扩展程序”，选择解压后的 `tmwd_cdp_bridge` 目录。

2. 本地源码构建时，也可以直接加载扩展目录：

```text
assets/tmwd_cdp_bridge
```

3. Chrome 需要至少打开一个正常网页标签页，不要只停留在 `about:blank` 或 `chrome://` 页面。
4. 扩展连接后会在页面右侧显示 Chrome 插件提示角标。角标支持拖动位置，鼠标悬浮时展开；10 秒无命令后自动隐藏，也可以点击 `本次隐藏` 手动隐藏，本次服务连接周期内不再显示，约 300 秒服务断开并下次重连后恢复。

###  自定义Chrome插件的ws监听端口

- `18768`：默认插件 WebSocket 端口，Chrome 扩展连接使用，可通过 `agent-browser-cli set-extension-port <port>` 修改。
- `18765`：GenericAgent 的常用 `TMWebDriver` 端口。若 Chrome 中同时启用本项目扩展和 GenericAgent 扩展，请不要让两者连接同一个端口，否则两个扩展都可能尝试控制同一个标签页的 `chrome.debugger`，导致 `Another debugger is already attached`。
- `18767`：CLI HTTP API 端口，供 CLI 复用会话，不能作为插件端口使用。

CLI 修改插件端口：

```bash
agent-browser-cli set-extension-port 18768
```

该命令会写入配置文件；如果 daemon 正在运行，会自动重启 daemon，让新端口立即生效。

也可以手动修改配置文件。配置文件位于 `~/.agent-browser-cli/config.json`，不存在时会自动生成：

```json
{
  "extension_port": 18768
}
```

手动修改示例：

```json
{
  "extension_port": 18768
}
```

手动改配置后需要执行 `agent-browser-cli restart`，daemon 才会按新端口重新监听。

Chrome 插件 popup 中也可以修改插件端口并立即重连。插件端口必须和 CLI 配置中的 `extension_port` 一致。

如果已经在同一个 Chrome Profile 中安装了 GenericAgent 的 `TMWD CDP Bridge` 扩展，建议保留 GenericAgent 使用 `18765`，本项目使用默认 `18768`。两个扩展可以同时存在，但不要同时对同一个标签页执行需要 CDP fallback 的操作。

### Profile Label

多 Chrome Profile / 多浏览器实例下，`profile_id` 和 `browser_id` 较长。可以给每个 Chrome Profile 设置短 label，之后用 `--profile <label>` 操作。

```bash
agent-browser-cli lookup tab <tabId>
agent-browser-cli lookup browser <browser_id>
agent-browser-cli profile-label set work --profile <profile_id>
agent-browser-cli tabs --profile work
```

也可以在对应 Chrome Profile 的扩展 popup 中设置 Profile Label。label 只作为别名，内部路由仍使用 `browser_id:profile_id:tab_id`；如果当前 daemon 内 label 匹配到多个 profile，CLI 会报歧义。推荐用 CLI 设置 label，因为 CLI 会校验当前 daemon 内跨 Profile 唯一性；popup 是本地便捷入口，不保证跨 Profile 唯一。`tabtree` 默认截断 URL 并省略 `session_key` 以减少 token，需完整字段时加 `--full`。




### 弹窗抑制

扩展不再默认重写页面的 `alert` / `confirm` / `prompt`。只有 CLI 执行页面脚本命令期间会临时抑制原生弹窗，命令结束后恢复，避免长期污染业务页面全局函数。

## 快速自检

```bash
agent-browser-cli tabs
agent-browser-cli open https://www.baidu.com
```

成功时会返回：

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

## 常用命令

README 只保留快速入口；完整命令和浏览器操作 SOP 见 [skills/agent-browser-cli/SKILL.md](./skills/agent-browser-cli/SKILL.md)。

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

## 更新

完整版本记录见 [CHANGELOG.md](./CHANGELOG.md)。

ai一句话更新
```text
请阅读 https://github.com/sleepinginsummer/agent-browser-cli/blob/main/AI_INSTALL.md，按说明更新 CLI、重新下载插件zip让用户指定位置，用户手动加载 Chrome 扩展，并更新相关 SKILL.md`。
```

如果 Chrome 扩展有更新，在 `chrome://extensions` 中重新下载zpi覆盖，然后重新加载 `assets/tmwd_cdp_bridge` 扩展。


## 卸载

先停止常驻服务：

```bash
agent-browser-cli stop
```

然后按需清理：

```bash
npm uninstall -g @sleepinsummer/agent-browser-cli
rm -f .agent-browser-cli.log .agent-browser-cli.lock
rm -rf ~/.agents/skills/agent-browser-cli
```

最后在 Chrome 扩展管理页中移除 `TMWD CDP Bridge` 扩展，或删除已加载的 `assets/tmwd_cdp_bridge` 扩展配置。



## 友情链接

- [LINUX DO - 新的理想型社区](https://linux.do/)
- [GenericAgent--复旦团队研发|仅仅~3K行代码 Self-Evolving Agent](https://github.com/lsdefine/GenericAgent/tree/main)

## 许可证

MIT License. See [LICENSE](./LICENSE).
