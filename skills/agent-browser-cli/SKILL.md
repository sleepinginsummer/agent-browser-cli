---
name: agent-browser-cli
description: 使用 agent-browser-cli 进行浏览器感知与控制、页面交互、截图/PDF、Cookie/CDP 和排障。
---

# agent-browser-cli

使用 `agent-browser-cli` 控制用户真实 Chrome。底层是 Rust daemon + Chrome 扩展桥，保留登录态和 Cookie；不是 Selenium/Playwright。

## 正常流程直接执行命令

每次开始浏览器任务，不要先做健康检查，直接执行最贴近目标的命令。`tabs` / `open` / `exec` / `scan` 会按需自动启动 daemon；daemon 未常驻是正常状态，不是故障。

```bash
agent-browser-cli open https://example.com
agent-browser-cli scan --text-only
agent-browser-cli snapshot --limit 200
agent-browser-cli exec --tab <tabId> 'return document.title'
```

只有目标命令失败、输出明确提示连接异常、扩展未连接、端口不一致、无可用标签页，或用户明确要求排障时，才进入状态检查：

```bash
agent-browser-cli status
agent-browser-cli doctor
agent-browser-cli logs --tail 100
```

`status` 中 `daemon_not_running` / `running=false` 不能单独作为停止操作的理由；优先继续执行目标命令让 CLI 自动拉起 daemon。`doctor` 只检查状态，不自动启动 daemon、不改配置、不安装 skill。

补充一个容易误判的点：`status` / `doctor` 里看到 `daemon_not_running`，如果此时还没有执行 `tabs` / `open` / `exec` / `scan`，通常只是 daemon 按需常驻而未启动，不代表故障。只有目标命令已经失败，或者日志/输出明确提示端口、扩展、标签页异常时，才进入排障。

## 打开本地文件

`open` 支持 HTTP(S) URL、绝对 `file://` URL 和本机文件路径。相对路径以执行 CLI 时的当前目录为基准；支持 `~`、中文、空格和符号链接，符号链接会解析到真实文件。所有普通文件类型均可打开，目录会被拒绝。

```bash
agent-browser-cli open ./demo.html
agent-browser-cli open ~/Documents/report.pdf
agent-browser-cli open 'file:///Users/me/My%20Files/demo.html#intro'
agent-browser-cli open ./demo.html --timeout 15
```

本地文件要求 Chrome 扩展版本至少为 2.1，并在 `chrome://extensions` → Agent Browser CLI Bridge → 详情中开启“允许访问文件网址”。权限关闭时 `open` 会在创建标签前失败；`status` / `doctor` 会按 Profile 报告 `file_scheme_access`，但该可选能力不影响普通 HTTP(S) 健康状态。

输入分类规则：

- `./`、`../`、`~/`、绝对路径和显式 `file://` 表示本地文件；文件必须存在且是普通文件。
- 裸输入若对应当前目录中已存在的文件，文件优先；否则按 Web 地址处理。要强制打开网站，显式写 `https://`。
- 普通路径中的 `?` 和 `#` 是文件名字符；仅显式 `file://` URL 使用 query/fragment 语义。
- `localhost`、回环/私网 IP、单标签主机和 `.local` 的无协议地址默认使用 HTTP；其他域名默认 HTTPS；端口 80/443 分别强制 HTTP/HTTPS。
- 只支持 `http:`、`https:` 和 `file:`；其他显式 scheme 会返回 `unsupported_scheme`。
- 异平台路径会明确报错，例如 macOS/Linux 不会把 `C:\\Users\\...` 错当作网址。

本地文件 `open` 默认最多等待 10 秒，直到新标签成为可控制会话；可用 `--timeout` 调整。超时后标签会保留，错误结果包含稳定的 `error_code` 和 `opened_tab_id`。活动 `open` 成功后新标签成为默认会话，`--background` 不切换默认会话。

## 常用命令优先级

先区分三个入口：

```text
scan：内容感知，适合看正文、列表、页面文本。
snapshot：操作定位，适合找按钮、链接、输入框并生成 @e 引用。
exec / JSON CDP：逃生口，封装命令失效或特殊页面时回退。
```

基础感知和按需排障：

```bash
agent-browser-cli tabs
agent-browser-cli tabtree
agent-browser-cli tabtree --full
agent-browser-cli tabtree --profile work
agent-browser-cli tabtree --tab <tabId>
agent-browser-cli lookup tab <tabId>
agent-browser-cli lookup browser <browser_id>
agent-browser-cli lookup profile work
agent-browser-cli tabs --profile work
agent-browser-cli profile-label set work --profile <profile_id>
agent-browser-cli scan --tabs-only
agent-browser-cli scan --profile work --tab <tabId> --text-only
agent-browser-cli open --profile work https://example.com
agent-browser-cli open --window https://example.com
agent-browser-cli open --window --focus https://example.com
# open 返回后继续操作新页面时，优先使用 result.opened_tab_id / result.opened_session_key
agent-browser-cli open https://example.com
agent-browser-cli close --tab <tabId>
agent-browser-cli exec --tab <tabId> 'return document.title'
agent-browser-cli status
agent-browser-cli doctor
agent-browser-cli logs --tail 100
```

第二阶段后的推荐流程：

```text
看页面内容：scan / scan --text-only
selector 明确时：直接 click/fill selector
selector 不明确或页面复杂时：snapshot 生成 @e，再 click/fill @e
页面结构或内容变化后：重新 snapshot
封装命令失效或覆盖不到特殊页面时：回退 exec / JSON CDP / 自定义 JS
```

示例：

```bash
agent-browser-cli scan --text-only
agent-browser-cli click 'button[type=submit]'
agent-browser-cli snapshot --limit 200
agent-browser-cli snapshot --offset 200 --limit 200
agent-browser-cli snapshot --details
agent-browser-cli click '@e1'
agent-browser-cli fill '@e2' 'hello'
agent-browser-cli fill '@e2' --clear
agent-browser-cli fill '@e2' ' world' --append
agent-browser-cli send-keys --target '@e2' 'Enter'
agent-browser-cli mouse-click '@e3'
```

所有高层操作都支持 `--tab <tabId>`，多 Chrome Profile / 多浏览器实例时还支持 `--profile <profile_id-or-label>` 和 `--browser <browser_id>`。`tabs` 输出会包含 `browser_id`、`profile_id`、`profile_label`、`tab_id`、`session_key`。`tabtree` 用树形结构列出 browser → profile → tabs，支持 `--tab <tabId>`、`--profile <profile_id-or-label>`、`--browser <browser_id>` 过滤，过滤结果仍保留父子节点；默认 compact 输出会截断 URL 并省略 `session_key`，需要完整字段时用 `tabtree --full`；`lookup tab <tabId>` 可由 tab 反查 `browser_id` / `profile_id` / `profile_label`，`lookup browser <browser_id>` 可反查所属 profile。`profile-label set <label> --profile <profile_id>` 可设置当前 Chrome Profile 的 label 并校验当前 daemon 内唯一性，`profile-label clear --profile <profile_id>` 可清空；popup 设置是本地便捷入口，不保证跨 Profile 唯一；label 只允许英文、数字、`-`、`_`、`.`，当前 daemon 内匹配到多个 profile 时会报歧义，不能猜。只传 `--tab` 且存在歧义时，必须补 `--profile` 或 `--browser`。`@e` 只在当前 daemon、当前 session_key、最近一次 `snapshot` 内有效。`@e` 只接受 `@e1` 这种带 `@` 的格式。

慢页面要把等待和监控分开：

```bash
agent-browser-cli click '@e1' --wait-js 'return document.body.innerText.includes("完成")' --wait-timeout 10 --monitor
```

`--wait-js` 负责等慢加载；`--monitor` 只负责操作前后页面 diff，默认关闭。

Chrome 右上角“正在控制你的浏览器”是 `chrome.debugger.attach` 的正常提示，不是 daemon 常驻状态。普通 CDP 命令空闲后会延迟约 30 秒自动 detach；30 秒内同一 tab 继续执行 CDP 会复用连接并重新计时。network/console 持续监听期间不会自动 detach，停止监听或清理时才释放。

弹窗处理：扩展默认不改写业务页面的 `alert` / `confirm` / `prompt`。只有 CLI 页面执行命令期间临时抑制弹窗，结束后恢复；如果怀疑页面原生弹窗行为异常，先让用户重载扩展和页面。

## 端口和扩展

固定 API 端口：

```text
127.0.0.1:18767
```

默认扩展 WebSocket 端口：

```text
127.0.0.1:18765
```

配置文件：

```text
~/.agent-browser-cli/config.json
```

修改扩展端口会影响 Chrome 扩展和 daemon 连接，执行前必须说明影响并取得用户确认：

```bash
agent-browser-cli set-extension-port <port>
```



## 网络和控制台调试

`network` / `console` 需要扩展侧持续监听 CDP 事件。修改或升级扩展后，必须先让用户重载 Chrome 插件。

```bash
agent-browser-cli network start --tab <tabId>
agent-browser-cli network list --tab <tabId> --filter api
agent-browser-cli network detail <requestId> --tab <tabId>
agent-browser-cli network clear --tab <tabId>
agent-browser-cli network stop --tab <tabId>

agent-browser-cli console start --tab <tabId>
agent-browser-cli console list --tab <tabId>
agent-browser-cli console list --tab <tabId> --level error
agent-browser-cli console clear --tab <tabId>
agent-browser-cli console stop --tab <tabId>
```

`network detail` 会截断大响应体并标记 `base64Encoded`，不要把巨大 body 粘到对话里。`network clear` 清请求缓存；`network stop` 会停止监听并清请求缓存。`console clear` 清日志缓存；`console stop` 会停止监听并清日志缓存。


`agent-browser-cli stop` / daemon idle 退出时会额外清理 daemon 内的 snapshot/@e 缓存，并通知扩展清理 network/console 调试缓存。

## 标签分组

多任务开新标签时可以用 session 或 group-title 把标签放入 Chrome 原生标签组。需要独立窗口时用 `open --window`；默认不聚焦，需要抢焦点时显式加 `--focus`；`--window --group-title` / `--window --session` 会把新窗口里的首个 tab 加入对应 tab group。分组只是整理浏览器标签，失败不影响开 tab 主流程。

```bash
agent-browser-cli open https://example.com --session research
agent-browser-cli open https://example.com --group-title "任务A"
agent-browser-cli open --window https://example.com
agent-browser-cli open --window --focus https://example.com
agent-browser-cli open --profile work https://example.com
```

`--session` 和 `--group-title` 都会作为标签组标题；两者同时传时优先使用 `--group-title`。

## 截图和 PDF

截图/PDF 必须让 CLI 写文件，不要把 base64 大段塞进上下文。命令只返回路径、字节数和少量元信息。

```bash
agent-browser-cli screenshot --out /tmp/page.png
agent-browser-cli screenshot --full-page --out /tmp/full.png
agent-browser-cli screenshot --target '@e1' --out /tmp/button.png
agent-browser-cli screenshot --selector 'button[type=submit]' --format jpeg --quality 70 --out /tmp/button.jpg
agent-browser-cli save-pdf --out /tmp/page.pdf
agent-browser-cli save-pdf --paper a4 --landscape --scale 0.9 --out /tmp/page.pdf
```

`screenshot` 默认截当前视口；`--full-page` 截全页；`--target` 和兼容别名 `--selector` 二选一，目标既可以是 `@e` 也可以是 CSS selector。没有 `--out` 时，截图写到 `/tmp/agent-browser-cli-screenshots/`。

`save-pdf` 默认 `paper=a4`、`scale=1.0`、`print-background=true`；需要关闭背景时用 `--no-print-background`。没有 `--out` 时，PDF 写到 `/tmp/agent-browser-cli-pdfs/`，默认文件名来自清理后的页面标题。

如果封装命令失效，用 `exec` 调 CDP 后由脚本落盘，仍然避免把 base64 粘到对话里。

## exec 使用规则

执行复杂 JS 时写入临时文件：

```bash
agent-browser-cli exec --tab <tabId> --file /tmp/script.js
```

需要等待页面变化时使用 `--wait-js`，不要在脚本里固定 `setTimeout`：

```bash
agent-browser-cli exec --tab <tabId> 'document.querySelector("button").click()' --wait-js 'return document.body.innerText.includes("完成")' --wait-timeout 3
```

`exec` 中使用 `await` 必须显式 `return`，否则结果可能是 `null`。

## JSON/CDP 逃生口

跨标签页、Cookie、CDP、扩展管理、浏览器内容权限时，用 JSON 指令：

```bash
agent-browser-cli exec '{"cmd":"tabs"}'
agent-browser-cli exec '{"cmd":"cookies"}'
agent-browser-cli exec '{"cmd":"cdp","tabId":303987837,"method":"Page.captureScreenshot","params":{"format":"png"}}'
```

CDP 点击优先三事件序列：`mouseMoved -> mousePressed -> mouseReleased`。首次 attach 可能出现 Chrome infobar，先发无害 `mouseMoved(0,0)` 预热。

## 文件上传

文件上传优先用 DataTransfer API，不优先使用 CDP `DOM.setFileInputFiles`：

```js
const input = document.querySelector('input[type=file]');
const file = new File(['content'], 'demo.txt', { type: 'text/plain' });
const dt = new DataTransfer();
dt.items.add(file);
input.files = dt.files;
input.dispatchEvent(new Event('input', { bubbles: true }));
input.dispatchEvent(new Event('change', { bubbles: true }));
return input.files.length;
```

## 运维入口

- 目标命令失败、扩展未连接、端口不一致、无可用标签页：看 `references/operations.md`。
- daemon 未运行但尚未执行目标命令：不要排障，直接继续执行 `tabs/open/exec/scan`。
- skill 安装：先 `agent-browser-cli install-skill --dry-run` 展示计划，用户确认后再 `agent-browser-cli install-skill`。
- 可自动执行的排障命令：`status`、`doctor`、`logs --tail`、`restart`、`stop`、`tabs`。
