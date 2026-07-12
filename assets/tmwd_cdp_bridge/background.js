// background.js - Cookie + CDP Bridge
chrome.runtime.onInstalled.addListener(() => {
  console.log('CDP Bridge installed');
  // Strip CSP headers to allow eval/inline scripts
  chrome.declarativeNetRequest.updateDynamicRules({
    removeRuleIds: [9999],
    addRules: [{
      id: 9999, priority: 1,
      action: { type: 'modifyHeaders', responseHeaders: [
        { header: 'content-security-policy', operation: 'remove' },
        { header: 'content-security-policy-report-only', operation: 'remove' }
      ]},
      condition: { urlFilter: '*', resourceTypes: ['main_frame', 'sub_frame'] }
    }]
  });
});

let lastCommandAt = 0;
const DEFAULT_WS_PORT = 18765;
const CLI_API_PORT = 18767;
let wsPort = DEFAULT_WS_PORT;
const browserId = `browser-${crypto.randomUUID()}`;
const extensionVersion = chrome.runtime.getManifest().version;
let profileId = null;
let profileLabel = null;

async function loadClientIdentity() {
  const data = await chrome.storage.local.get({ profileId: null, profileLabel: null });
  profileId = data.profileId || `profile-${crypto.randomUUID()}`;
  profileLabel = data.profileLabel || null;
  if (!data.profileId) await chrome.storage.local.set({ profileId });
  return { browserId, profileId, profileLabel };
}

function withClientIdentity(payload) {
  return Object.assign({
    browser_id: browserId,
    profile_id: profileId || 'profile-pending',
    profile_label: profileLabel || undefined
  }, payload);
}

async function handleExtMessage(msg, sender) {
  if (msg.cmd === 'status') return await handleStatus();
  if (msg.cmd === 'setPort') return await handleSetPort(msg);
  if (msg.cmd === 'setProfileLabel') return await handleSetProfileLabel(msg);
  lastCommandAt = Date.now();
  if (msg.cmd === 'cookies') return await handleCookies(msg, sender);
  if (msg.cmd === 'cdp') return await handleCDP(msg, sender);
  if (msg.cmd === 'batch') return await handleBatch(msg, sender);
  if (msg.cmd === 'openTab') return await handleOpenTab(msg);
  if (msg.cmd === 'closeTab') return await handleCloseTab(msg, sender);
  if (msg.cmd === 'networkStart') return await handleNetworkStart(msg, sender);
  if (msg.cmd === 'networkList') return await handleNetworkList(msg, sender);
  if (msg.cmd === 'networkDetail') return await handleNetworkDetail(msg, sender);
  if (msg.cmd === 'networkClear') return await handleNetworkClear(msg, sender);
  if (msg.cmd === 'networkStop') return await handleNetworkStop(msg, sender);
  if (msg.cmd === 'consoleStart') return await handleConsoleStart(msg, sender);
  if (msg.cmd === 'consoleList') return await handleConsoleList(msg, sender);
  if (msg.cmd === 'consoleClear') return await handleConsoleClear(msg, sender);
  if (msg.cmd === 'consoleStop') return await handleConsoleStop(msg, sender);
  if (msg.cmd === 'debugClearAll') return await handleDebugClearAll();
  if (msg.cmd === 'tabs') {
    try {
      if (msg.method === 'switch') {
        const tab = await chrome.tabs.update(msg.tabId, { active: true });
        // 默认只切换 Chrome 内部 active tab，不抢占系统前台窗口。
        if (msg.allowFocus === true && tab.windowId) await chrome.windows.update(tab.windowId, { focused: true });
        return { ok: true };
      } else {
        const tabs = await queryScriptableTabs();
        const data = tabs.map(t => ({ id: t.id, url: t.url, title: t.title, active: t.active, windowId: t.windowId }));
        return { ok: true, data };
      }
    } catch (e) { return { ok: false, error: e.message }; }
  }
  if (msg.cmd === 'management') {
    try {
      if (msg.method === 'list') {
        const all = await chrome.management.getAll();
        return { ok: true, data: all.map(e => ({ id: e.id, name: e.name, enabled: e.enabled, type: e.type, version: e.version })) };
      }
      if (msg.method === 'reload') {
        chrome.alarms.create('tmwd-self-reload', { when: Date.now() + 200 });
        return { ok: true };
      }
      if (msg.method === 'disable') {
        await chrome.management.setEnabled(msg.extId, false);
        return { ok: true };
      }
      if (msg.method === 'enable') {
        await chrome.management.setEnabled(msg.extId, true);
        return { ok: true };
      }
      return { ok: false, error: 'Unknown method: ' + msg.method };
    } catch (e) { return { ok: false, error: e.message }; }
  }
  if (msg.cmd === 'contentSettings') {
    try {
      const type = msg.type || 'automaticDownloads';
      const setting = msg.setting || 'allow';
      const pattern = msg.pattern || '<all_urls>';
      await chrome.contentSettings[type].set({
        primaryPattern: pattern,
        setting: setting
      });
      return { ok: true };
    } catch (e) { return { ok: false, error: e.message }; }
  }
  return { ok: false, error: 'Unknown cmd: ' + msg.cmd };
}

async function handleStatus() {
  return {
    ok: true,
    data: {
      wsConnected: !!ws && ws.readyState === WebSocket.OPEN,
      wsUrl: getWsUrl(),
      wsPort,
      lastCommandAt,
      browserId,
      profileId,
      profileLabel,
      extensionId: chrome.runtime.id,
      extensionVersion,
      fileSchemeAccess: await chrome.extension.isAllowedFileSchemeAccess()
    }
  };
}

async function handleSetPort(msg) {
  const port = normalizePort(msg.port);
  await saveWsPort(port);
  reconnectWS();
  return {
    ok: true,
    data: {
      wsPort,
      wsUrl: getWsUrl(),
      wsConnected: !!ws && ws.readyState === WebSocket.OPEN
    }
  };
}

function normalizeProfileLabel(label) {
  const raw = String(label || '').trim();
  if (!raw) return null;
  if (raw.length > 40) throw new Error('Profile Label 长度不能超过 40 个字符');
  if (!/^[A-Za-z0-9_.-]+$/.test(raw)) throw new Error('Profile Label 只能包含英文、数字、-、_、.');
  return raw;
}

async function handleSetProfileLabel(msg) {
  const label = normalizeProfileLabel(msg.label);
  profileLabel = label;
  await chrome.storage.local.set({ profileLabel });
  await sendTabsUpdate();
  return {
    ok: true,
    data: {
      browserId,
      profileId,
      profileLabel
    }
  };
}

chrome.runtime.onMessage.addListener((msg, sender, sendResponse) => {
  handleExtMessage(msg, sender).then(sendResponse);
  return true;
});

async function handleCookies(msg, sender) {
  try {
    let url = msg.url || sender.tab?.url;
    if (!url && msg.tabId) {
      const tab = await chrome.tabs.get(msg.tabId);
      url = tab.url;
    }
    const origin = url.match(/^https?:\/\/[^\/]+/)[0];
    const all = await chrome.cookies.getAll({ url });
    const part = await chrome.cookies.getAll({ url, partitionKey: { topLevelSite: origin } }).catch(() => []);
    const merged = [...all];
    for (const c of part) {
      if (!merged.some(x => x.name === c.name && x.domain === c.domain)) merged.push(c);
    }
    return { ok: true, data: merged };
  } catch (e) {
    return { ok: false, error: e.message };
  }
}


const debugSessions = new Map();
const DEBUG_DETACH_DELAY_MS = 30000;
const DEBUG_DETACH_ALARM_PREFIX = 'tmwd-debug-detach-';

function debugDetachAlarmName(tabId) {
  return `${DEBUG_DETACH_ALARM_PREFIX}${tabId}`;
}

function scheduleDebugDetach(session, reason = 'idle') {
  if (!session || !session.attached || session.network || session.console) return;
  session.detachReason = reason;
  chrome.alarms.create(debugDetachAlarmName(session.tabId), { delayInMinutes: DEBUG_DETACH_DELAY_MS / 60000 });
}

function resolveTabId(msg, sender) {
  const tabId = Number(msg.tabId || sender.tab?.id);
  if (!Number.isInteger(tabId) || tabId <= 0) throw new Error('no tabId');
  return tabId;
}

function getDebugSession(tabId) {
  let session = debugSessions.get(tabId);
  if (!session) {
    session = {
      tabId,
      attached: false,
      network: false,
      console: false,
      requests: new Map(),
      requestOrder: [],
      logs: [],
      detachReason: null
    };
    debugSessions.set(tabId, session);
  }
  return session;
}

function isDebuggerAlreadyAttachedError(error) {
  return /Another debugger is already attached/i.test(String(error?.message || error || ''));
}

async function isDebuggerTargetAttached(tabId) {
  try {
    const targets = await chrome.debugger.getTargets();
    const target = targets.find(t => t.tabId === tabId);
    return !!target?.attached;
  } catch (e) {
    console.log('[TMWD-DEBUG] getTargets failed:', e.message);
    return false;
  }
}

async function attachDebuggerWithRecovery(tabId) {
  try {
    await chrome.debugger.attach({ tabId }, '1.3');
    return;
  } catch (e) {
    if (!isDebuggerAlreadyAttachedError(e)) throw e;

    // daemon 非 graceful 退出时，Chrome 可能还保留本扩展的旧 attach。
    // 这里只在 Chrome 明确报告该 tab 已 attached 后做一次 detach+retry，避免正常路径反复抖动。
    const attached = await isDebuggerTargetAttached(tabId);
    if (!attached) throw e;
    try {
      await chrome.debugger.detach({ tabId });
    } catch (detachError) {
      console.log('[TMWD-DEBUG] stale detach failed:', tabId, detachError.message);
      throw e;
    }
    await chrome.debugger.attach({ tabId }, '1.3');
  }
}

async function ensureDebugAttached(session) {
  await chrome.alarms.clear(debugDetachAlarmName(session.tabId)).catch(() => null);
  session.detachReason = null;
  if (session.attached) return;
  await attachDebuggerWithRecovery(session.tabId);
  session.attached = true;
}

async function detachDebugSession(session, reason = 'manual') {
  if (!session) return;
  await chrome.alarms.clear(debugDetachAlarmName(session.tabId)).catch(() => null);
  if (session.attached) {
    try { await chrome.debugger.detach({ tabId: session.tabId }); } catch (e) {
      console.log('[TMWD-DEBUG] detach failed:', session.tabId, reason, e.message);
    }
  }
  session.attached = false;
  session.network = false;
  session.console = false;
  session.detachReason = null;
}

async function detachDebugIfIdle(session) {
  if (!session.attached || session.network || session.console) return;
  scheduleDebugDetach(session, 'idle');
}

async function detachAllDebugSessions(reason = 'cleanup') {
  const sessions = [...debugSessions.values()];
  for (const session of sessions) {
    session.requests.clear();
    session.requestOrder = [];
    session.logs = [];
    await detachDebugSession(session, reason);
  }
  debugSessions.clear();
}

async function handleNetworkStart(msg, sender) {
  try {
    const tabId = resolveTabId(msg, sender);
    const session = getDebugSession(tabId);
    await ensureDebugAttached(session);
    await chrome.debugger.sendCommand({ tabId }, 'Network.enable', {});
    session.network = true;
    return { ok: true, status: 'started', tabId, count: session.requestOrder.length };
  } catch (e) {
    return { ok: false, error: e.message };
  }
}

async function handleNetworkList(msg, sender) {
  try {
    const tabId = resolveTabId(msg, sender);
    const session = getDebugSession(tabId);
    const filter = String(msg.filter || '').toLowerCase();
    const limit = Math.max(1, Math.min(Number(msg.limit || 100), 1000));
    let items = session.requestOrder.map(id => session.requests.get(id)).filter(Boolean);
    if (filter) {
      items = items.filter(item => [item.url, item.method, item.status, item.mimeType, item.resourceType].some(v => String(v || '').toLowerCase().includes(filter)));
    }
    items = items.slice(-limit).map(summarizeRequest);
    return { ok: true, status: session.network ? 'started' : 'stopped', tabId, count: items.length, requests: items };
  } catch (e) {
    return { ok: false, error: e.message };
  }
}

async function handleNetworkDetail(msg, sender) {
  try {
    const tabId = resolveTabId(msg, sender);
    const requestId = String(msg.requestId || '');
    const session = getDebugSession(tabId);
    const item = session.requests.get(requestId);
    if (!item) return { ok: false, error: 'unknown requestId: ' + requestId };
    const detail = Object.assign({}, item);
    if (item.completed && !item.failed && session.attached) {
      try {
        const body = await chrome.debugger.sendCommand({ tabId }, 'Network.getResponseBody', { requestId });
        detail.body = truncateText(body.body || '', 20000);
        detail.base64Encoded = !!body.base64Encoded;
        detail.bodyTruncated = String(body.body || '').length > 20000;
      } catch (e) {
        detail.bodyError = e.message;
      }
    }
    return { ok: true, tabId, request: detail };
  } catch (e) {
    return { ok: false, error: e.message };
  }
}

async function handleNetworkClear(msg, sender) {
  try {
    const tabId = resolveTabId(msg, sender);
    const session = getDebugSession(tabId);
    session.requests.clear();
    session.requestOrder = [];
    return { ok: true, status: 'cleared', tabId };
  } catch (e) {
    return { ok: false, error: e.message };
  }
}

async function handleNetworkStop(msg, sender) {
  try {
    const tabId = resolveTabId(msg, sender);
    const session = getDebugSession(tabId);
    if (session.attached) {
      try { await chrome.debugger.sendCommand({ tabId }, 'Network.disable', {}); } catch (_) {}
    }
    session.network = false;
    session.requests.clear();
    session.requestOrder = [];
    await detachDebugIfIdle(session);
    return { ok: true, status: 'stopped', cleared: true, tabId };
  } catch (e) {
    return { ok: false, error: e.message };
  }
}

async function handleConsoleStart(msg, sender) {
  try {
    const tabId = resolveTabId(msg, sender);
    const session = getDebugSession(tabId);
    await ensureDebugAttached(session);
    await chrome.debugger.sendCommand({ tabId }, 'Runtime.enable', {});
    await chrome.debugger.sendCommand({ tabId }, 'Log.enable', {}).catch(() => null);
    session.console = true;
    return { ok: true, status: 'started', tabId, count: session.logs.length };
  } catch (e) {
    return { ok: false, error: e.message };
  }
}

async function handleConsoleList(msg, sender) {
  try {
    const tabId = resolveTabId(msg, sender);
    const session = getDebugSession(tabId);
    const level = String(msg.level || '').toLowerCase();
    const limit = Math.max(1, Math.min(Number(msg.limit || 100), 1000));
    let logs = session.logs;
    if (level) logs = logs.filter(item => String(item.level || '').toLowerCase() === level);
    return { ok: true, status: session.console ? 'started' : 'stopped', tabId, count: logs.length, logs: logs.slice(-limit) };
  } catch (e) {
    return { ok: false, error: e.message };
  }
}

async function handleConsoleClear(msg, sender) {
  try {
    const tabId = resolveTabId(msg, sender);
    const session = getDebugSession(tabId);
    session.logs = [];
    return { ok: true, status: 'cleared', tabId };
  } catch (e) {
    return { ok: false, error: e.message };
  }
}

async function handleConsoleStop(msg, sender) {
  try {
    const tabId = resolveTabId(msg, sender);
    const session = getDebugSession(tabId);
    if (session.attached) {
      try { await chrome.debugger.sendCommand({ tabId }, 'Runtime.disable', {}); } catch (_) {}
      try { await chrome.debugger.sendCommand({ tabId }, 'Log.disable', {}); } catch (_) {}
    }
    session.console = false;
    session.logs = [];
    await detachDebugIfIdle(session);
    return { ok: true, status: 'stopped', cleared: true, tabId };
  } catch (e) {
    return { ok: false, error: e.message };
  }
}

function summarizeRequest(item) {
  return {
    requestId: item.requestId,
    url: item.url,
    method: item.method,
    status: item.status,
    mimeType: item.mimeType,
    resourceType: item.resourceType,
    completed: !!item.completed,
    failed: !!item.failed,
    errorText: item.errorText,
    timestamp: item.timestamp
  };
}

function rememberRequest(session, requestId) {
  if (!session.requests.has(requestId)) session.requestOrder.push(requestId);
  while (session.requestOrder.length > 1000) {
    const old = session.requestOrder.shift();
    session.requests.delete(old);
  }
  let item = session.requests.get(requestId);
  if (!item) {
    item = { requestId };
    session.requests.set(requestId, item);
  }
  return item;
}

function pushLog(session, item) {
  session.logs.push(item);
  if (session.logs.length > 1000) session.logs.splice(0, session.logs.length - 1000);
}

function remoteObjectText(arg) {
  if (!arg) return '';
  if ('value' in arg) {
    if (typeof arg.value === 'string') return truncateText(arg.value, 2000);
    try { return truncateText(JSON.stringify(arg.value), 2000); } catch (_) { return String(arg.value); }
  }
  return truncateText(arg.description || arg.type || '', 2000);
}

function truncateText(text, max) {
  const value = String(text || '');
  return value.length > max ? value.slice(0, max) : value;
}

function onDebuggerEvent(source, method, params) {
  const tabId = source.tabId;
  if (!tabId) return;
  const session = debugSessions.get(tabId);
  if (!session) return;
  if (method === 'Network.requestWillBeSent') {
    const item = rememberRequest(session, params.requestId);
    item.url = params.request?.url || item.url;
    item.method = params.request?.method || item.method;
    item.resourceType = params.type || item.resourceType;
    item.timestamp = params.wallTime || params.timestamp || item.timestamp;
    item.requestHeaders = params.request?.headers;
  } else if (method === 'Network.responseReceived') {
    const item = rememberRequest(session, params.requestId);
    item.status = params.response?.status;
    item.statusText = params.response?.statusText;
    item.mimeType = params.response?.mimeType;
    item.responseHeaders = params.response?.headers;
    item.url = params.response?.url || item.url;
    item.resourceType = params.type || item.resourceType;
  } else if (method === 'Network.loadingFinished') {
    const item = rememberRequest(session, params.requestId);
    item.completed = true;
    item.encodedDataLength = params.encodedDataLength;
  } else if (method === 'Network.loadingFailed') {
    const item = rememberRequest(session, params.requestId);
    item.completed = true;
    item.failed = true;
    item.errorText = params.errorText;
  } else if (method === 'Runtime.consoleAPICalled') {
    pushLog(session, {
      level: params.type || 'log',
      text: (params.args || []).map(remoteObjectText).join(' '),
      timestamp: params.timestamp || Date.now(),
      url: params.stackTrace?.callFrames?.[0]?.url || '',
      line: params.stackTrace?.callFrames?.[0]?.lineNumber,
      column: params.stackTrace?.callFrames?.[0]?.columnNumber
    });
  } else if (method === 'Runtime.exceptionThrown') {
    pushLog(session, {
      level: 'error',
      text: params.exceptionDetails?.text || params.exceptionDetails?.exception?.description || 'exception thrown',
      timestamp: params.timestamp || Date.now(),
      url: params.exceptionDetails?.url || '',
      line: params.exceptionDetails?.lineNumber,
      column: params.exceptionDetails?.columnNumber
    });
  } else if (method === 'Log.entryAdded') {
    const e = params.entry || {};
    pushLog(session, {
      level: e.level || 'log',
      text: e.text || '',
      timestamp: e.timestamp || Date.now(),
      url: e.url || '',
      line: e.lineNumber,
      column: undefined
    });
  }
}

function onDebuggerDetach(source) {
  if (!source.tabId) return;
  chrome.alarms.clear(debugDetachAlarmName(source.tabId));
  const session = debugSessions.get(source.tabId);
  if (session) {
    session.attached = false;
    session.network = false;
    session.console = false;
    session.detachReason = null;
  }
}

chrome.debugger.onEvent.addListener(onDebuggerEvent);
chrome.debugger.onDetach.addListener(onDebuggerDetach);



async function handleCloseTab(msg, sender) {
  try {
    const tabId = Number(msg.tabId || sender.tab?.id);
    if (!Number.isInteger(tabId) || tabId <= 0) throw new Error('tabId is required');
    const session = debugSessions.get(tabId);
    if (session?.attached) await detachDebugSession(session, 'close-tab');
    debugSessions.delete(tabId);
    await chrome.tabs.remove(tabId);
    return { ok: true, data: { status: 'success', tabId } };
  } catch (e) {
    return { ok: false, error: e.message };
  }
}

async function handleDebugClearAll() {
  const tabIds = [...debugSessions.keys()];
  await detachAllDebugSessions('debugClearAll');
  return { ok: true, status: 'cleared', tabs: tabIds.length };
}

async function handleOpenTab(msg) {
  try {
    const url = normalizeOpenUrl(msg.url);
    const active = msg.active !== false;
    if (msg.window === true) {
      // window 模式默认不聚焦，避免新窗口打断用户当前工作区。
      const win = await chrome.windows.create({ url, focused: msg.allowFocus === true });
      const tab = Array.isArray(win.tabs) && win.tabs.length ? win.tabs[0] : null;
      const group = tab?.id ? await groupTabIfRequested(tab.id, msg.groupTitle) : { ok: false, skipped: true, reason: 'window created without tab info' };
      return { ok: true, data: { id: tab?.id, url: tab?.url || url, title: tab?.title || '', active: !!tab?.active, windowId: win.id, window: true, group } };
    }
    const tab = await chrome.tabs.create({ url, active });
    const group = await groupTabIfRequested(tab.id, msg.groupTitle);
    // 默认创建/激活标签页但不聚焦浏览器窗口，避免打断当前工作区。
    if (active && msg.allowFocus === true && tab.windowId) await chrome.windows.update(tab.windowId, { focused: true });
    return { ok: true, data: { id: tab.id, url: tab.url || url, title: tab.title || '', active: tab.active, windowId: tab.windowId, window: false, group } };
  } catch (e) {
    return { ok: false, error: e.message };
  }
}

async function groupTabIfRequested(tabId, title) {
  const cleanTitle = String(title || '').trim();
  if (!cleanTitle) return null;
  if (!chrome.tabs?.group || !chrome.tabGroups?.update) {
    return { ok: false, skipped: true, reason: 'tabGroups API unavailable' };
  }
  try {
    const existing = await findTabGroupByTitle(cleanTitle);
    const groupId = await chrome.tabs.group(existing ? { tabIds: tabId, groupId: existing.id } : { tabIds: tabId });
    await chrome.tabGroups.update(groupId, { title: cleanTitle });
    return { ok: true, id: groupId, title: cleanTitle };
  } catch (e) {
    // 分组只是整理标签，不影响打开页面的主流程。
    return { ok: false, skipped: true, reason: e.message };
  }
}

async function findTabGroupByTitle(title) {
  if (!chrome.tabGroups?.query) return null;
  const groups = await chrome.tabGroups.query({});
  return groups.find(group => group.title === title) || null;
}

function normalizeOpenUrl(url) {
  const raw = String(url || '').trim();
  if (!raw) throw new Error('url is required');
  if (/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(raw)) return raw;
  return 'https://' + raw;
}

async function handleBatch(msg, sender) {
  const R = [];
  const touchedSessions = new Set();
  const resolve$N = (params) => JSON.parse(JSON.stringify(params || {}).replace(/"\$(\d+)\.([^"]+)"/g,
    (_, i, path) => { let v = R[+i]; for (const k of path.split('.')) v = v[k]; return JSON.stringify(v); }));
  try {
    for (const c of msg.commands) {
      if (c.tabId === undefined && msg.tabId !== undefined) c.tabId = msg.tabId;
      if (c.cmd === 'cookies') {
        R.push(await handleCookies(c, sender));
      } else if (c.cmd === 'tabs') {
        const tabs = await queryScriptableTabs();
        R.push({ ok: true, data: tabs.map(t => ({ id: t.id, url: t.url, title: t.title, active: t.active, windowId: t.windowId })) });
      } else if (c.cmd === 'cdp') {
        const tabId = c.tabId || msg.tabId || sender.tab?.id;
        if (!tabId) throw new Error('no tabId');
        if (c.method === 'Page.bringToFront' && c.allowFocus !== true && msg.allowFocus !== true) {
          R.push({ skipped: true, reason: 'Page.bringToFront requires allowFocus=true' });
          continue;
        }
        const session = getDebugSession(tabId);
        await ensureDebugAttached(session);
        touchedSessions.add(session);
        R.push(await chrome.debugger.sendCommand({ tabId }, c.method, resolve$N(c.params)));
      } else {
        R.push({ ok: false, error: 'unknown cmd: ' + c.cmd });
      }
    }
    for (const session of touchedSessions) scheduleDebugDetach(session, 'batch-idle');
    return { ok: true, results: R };
  } catch (e) {
    for (const session of touchedSessions) scheduleDebugDetach(session, 'batch-error');
    return { ok: false, error: e.message, results: R };
  }
}

async function handleCDP(msg, sender) {
  const tabId = msg.tabId || sender.tab?.id;
  if (!tabId) return { ok: false, error: 'no tabId' };
  if (msg.method === 'Page.bringToFront' && msg.allowFocus !== true) {
    return { ok: true, data: { skipped: true, reason: 'Page.bringToFront requires allowFocus=true' } };
  }
  const session = getDebugSession(tabId);
  try {
    await ensureDebugAttached(session);
    const result = await chrome.debugger.sendCommand({ tabId }, msg.method, msg.params || {});
    scheduleDebugDetach(session, 'cdp-idle');
    return { ok: true, data: result };
  } catch (e) {
    scheduleDebugDetach(session, 'cdp-error');
    return { ok: false, error: e.message };
  }
}
// Filter out chrome:// and other internal tabs that can't be scripted.
const isScriptable = (url, fileSchemeAccess = false) => !!url && (/^https?:/i.test(url) || (fileSchemeAccess && /^file:/i.test(url)));

async function queryScriptableTabs() {
  const fileSchemeAccess = await chrome.extension.isAllowedFileSchemeAccess();
  return (await chrome.tabs.query({})).filter(tab => isScriptable(tab.url, fileSchemeAccess));
}

async function injectContentScriptsIntoExistingTabs() {
  const tabs = await queryScriptableTabs();
  for (const tab of tabs) {
    try {
      await chrome.scripting.executeScript({
        target: { tabId: tab.id, allFrames: true },
        files: ['config.js', 'content.js']
      });
    } catch (e) {
      console.log('[TMWD-WS] Inject content script failed:', tab.id, e.message);
    }
  }
}

function buildDialogSuppressionScript(enabled) {
  if (!enabled) {
    return `(() => {
      const state = window.__TMWD_DIALOG_SUPPRESSION__;
      if (!state) return;
      state.count = Math.max(0, Number(state.count || 1) - 1);
      if (state.count > 0) return;
      try {
        window.alert = state.alert;
        window.confirm = state.confirm;
        window.prompt = state.prompt;
      } finally {
        delete window.__TMWD_DIALOG_SUPPRESSION__;
      }
    })()`;
  }
  return `(() => {
    const existing = window.__TMWD_DIALOG_SUPPRESSION__;
    if (existing) {
      existing.count = Number(existing.count || 1) + 1;
      return;
    }
    const state = {
      count: 1,
      alert: window.alert,
      confirm: window.confirm,
      prompt: window.prompt
    };
    window.__TMWD_DIALOG_SUPPRESSION__ = state;
    const toast = (type, msg) => {
      try { console.log('[TMWD] ' + type + ' suppressed during CLI command:', msg); } catch (_) {}
      try {
        const d = document.createElement('div');
        d.textContent = '[' + type + '] ' + msg;
        Object.assign(d.style, {
          position:'fixed', top:'12px', right:'12px', zIndex:'2147483647',
          background:'#222', color:'#fff', padding:'10px 18px', borderRadius:'8px',
          fontSize:'14px', maxWidth:'420px', wordBreak:'break-all',
          boxShadow:'0 4px 16px rgba(0,0,0,.3)', opacity:'1',
          transition:'opacity .5s', pointerEvents:'none'
        });
        (document.body || document.documentElement).appendChild(d);
        setTimeout(() => { d.style.opacity = '0'; }, 3000);
        setTimeout(() => { d.remove(); }, 3600);
      } catch (_) {}
    };
    window.alert = function(msg) { toast('alert', msg); };
    window.confirm = function(msg) { toast('confirm', msg); return true; };
    window.prompt = function(msg, def) { toast('prompt', msg); return def || null; };
  })()`;
}

async function setDialogSuppressionByScripting(tabId, enabled) {
  try {
    await chrome.scripting.executeScript({
      target: { tabId },
      world: 'MAIN',
      func: async (script) => await eval(script),
      args: [buildDialogSuppressionScript(enabled)]
    });
    return true;
  } catch (e) {
    console.log('[TMWD-WS] dialog suppression scripting failed:', e.message);
    return false;
  }
}

async function setDialogSuppressionByCdp(tabId, enabled) {
  try {
    await chrome.debugger.sendCommand({ tabId }, 'Runtime.evaluate', {
      expression: buildDialogSuppressionScript(enabled),
      awaitPromise: true,
      returnByValue: true
    });
    return true;
  } catch (e) {
    console.log('[TMWD-WS] dialog suppression CDP failed:', e.message);
    return false;
  }
}

// --- Shared page/CDP script builder core ---
function buildExecScript(code, errorHandler) {
  return `(async () => {
    function smartProcessResult(result) {
      if (result === null || result === undefined || typeof result !== 'object') return result;
      try { if (result.window === result && result.document) return '[Window: ' + (result.location?.href || 'about:blank') + ']'; } catch(_){}
      if (typeof jQuery !== 'undefined' && result instanceof jQuery) {
        const elements = []; for (let i = 0; i < result.length; i++) { if (result[i] && result[i].nodeType === 1) elements.push(result[i].outerHTML); } return elements;
      }
      if (result instanceof NodeList || result instanceof HTMLCollection) {
        const elements = []; for (let i = 0; i < result.length; i++) { if (result[i] && result[i].nodeType === 1) elements.push(result[i].outerHTML); } return elements;
      }
      if (result.nodeType === 1) return result.outerHTML;
      if (!Array.isArray(result) && typeof result === 'object' && 'length' in result && typeof result.length === 'number') {
        const firstElement = result[0];
        if (firstElement && firstElement.nodeType === 1) {
          const elements = []; const length = Math.min(result.length, 100);
          for (let i = 0; i < length; i++) { const elem = result[i]; if (elem && elem.nodeType === 1) elements.push(elem.outerHTML); } return elements;
        }
      }
      try { return JSON.parse(JSON.stringify(result, function(key, value) { if (typeof value === 'object' && value !== null) { if (value.nodeType === 1) return value.outerHTML; if (value === window || value === document) return '[Object]'; try { if (value.window === value && value.document) return '[Window]'; } catch(_){} } return value; })); } catch (e) { return '[无法序列化: ' + e.message + ']'; }
    }
    try {
      const jsCode = ${JSON.stringify(code)}.trim();
      const lines = jsCode.split(/\\r?\\n/).filter(l => l.trim());
      const lastLine = lines.length > 0 ? lines[lines.length - 1].trim() : '';
      const AsyncFunction = Object.getPrototypeOf(async function(){}).constructor;
      let r;
      function _air(c) { const ls = c.split(/\\r?\\n/); let i = ls.length - 1; while (i >= 0 && !ls[i].trim()) i--; if (i < 0) return c; const t = ls[i].trim(); if (/^(return |return;|return$|let |const |var |if |if\\(|for |for\\(|while |while\\(|switch|try |throw |class |function |async |import |export |\\/\\/|})/.test(t)) return c; ls[i] = ls[i].match(/^(\\s*)/)[1] + 'return ' + t; return ls.join('\\n'); }
      if (lastLine.startsWith('return')) {
        r = await (new AsyncFunction(jsCode))();
      } else {
        try { r = eval(jsCode); if (r instanceof Promise) r = await r; } catch (e) {
          if (e instanceof SyntaxError && (/return/i.test(e.message) || /await/i.test(e.message))) { r = await (new AsyncFunction(_air(jsCode)))(); } else throw e;
        }
      }
      return { ok: true, data: smartProcessResult(r) };
    } catch (e) {
      ${errorHandler}
    }
  })()`;
}

function buildPageScript(code) {
  return buildExecScript(code, `
      const errMsg = e.message || String(e);
      return { ok: false, error: { name: e.name || 'Error', message: errMsg, stack: e.stack || '' },
        csp: errMsg.includes('Refused to evaluate') || errMsg.includes('unsafe-eval') || errMsg.includes('Content Security Policy') };
  `);
}

function buildCdpScript(code) {
  return buildExecScript(code, `
      return { ok: false, error: { name: e.name || 'Error', message: e.message || String(e), stack: e.stack || '' } };
  `);
}

// --- WebSocket Client for TMWebDriver ---
let ws = null;

function normalizePort(port) {
  const value = Number(port);
  if (!Number.isInteger(value) || value < 1 || value > 65535) {
    throw new Error('端口必须是 1-65535');
  }
  if (value === CLI_API_PORT) {
    throw new Error('18767 是 agent-browser-cli API 端口，请换一个插件端口');
  }
  return value;
}

async function loadWsPort() {
  const data = await chrome.storage.local.get({ wsPort: DEFAULT_WS_PORT });
  wsPort = normalizePort(data.wsPort);
  return wsPort;
}

async function saveWsPort(port) {
  wsPort = normalizePort(port);
  await chrome.storage.local.set({ wsPort });
}

function getWsUrl() {
  return `ws://127.0.0.1:${wsPort}`;
}

function scheduleProbe() {
  // Use chrome.alarms to survive MV3 service worker suspension
  chrome.alarms.create('tmwd-ws-probe', { delayInMinutes: 0.017 }); // ~1s
}

function scheduleKeepalive() {
  // Keep SW alive while WS is connected (~25s, under 30s SW timeout)
  chrome.alarms.create('tmwd-ws-keepalive', { delayInMinutes: 0.4 }); // ~24s
}

async function isServerAlive() {
  try {
    const ctrl = new AbortController();
    setTimeout(() => ctrl.abort(), 2000);
    await fetch(`http://127.0.0.1:${wsPort}`, { signal: ctrl.signal });
    return true; // Got HTTP response → port is listening
  } catch (e) {
    return false; // Network error (connection refused) or timeout → server not alive
  }
}

chrome.alarms.onAlarm.addListener(async (alarm) => {
  if (alarm.name === 'tmwd-self-reload') {
    chrome.runtime.reload();
    return;
  }
  if (alarm.name.startsWith(DEBUG_DETACH_ALARM_PREFIX)) {
    const tabId = Number(alarm.name.slice(DEBUG_DETACH_ALARM_PREFIX.length));
    const session = debugSessions.get(tabId);
    if (session?.attached && !session.network && !session.console) {
      await detachDebugSession(session, session.detachReason || 'delayed-idle');
    }
    return;
  }
  if (alarm.name === 'tmwd-ws-keepalive') {
    // Keepalive: ping to keep SW alive + detect dead connections
    if (ws && ws.readyState === WebSocket.OPEN) {
      try { ws.send('{"type":"ping"}'); } catch (_) {}
      scheduleKeepalive();
    } else {
      // Connection lost, switch to probe mode
      ws = null;
      detachAllDebugSessions('keepalive-lost').finally(() => scheduleProbe());
    }
  }
  if (alarm.name === 'tmwd-ws-probe') {
    if (ws && ws.readyState <= 1) return; // Already connected/connecting
    if (await isServerAlive()) {
      console.log('[TMWD-WS] Server detected, connecting...');
      connectWS();
    } else {
      scheduleProbe(); // Server not up, keep probing
    }
  }
});

async function handleWsExec(data) {
  const tabId = data.tabId;
  console.log('[TMWD-WS] Exec request', data.id, 'on tab', tabId);
  ws.send(JSON.stringify({ type: 'ack', id: data.id }));
  if (!tabId) {
    ws.send(JSON.stringify({ type: 'error', id: data.id, error: 'No tabId provided' }));
    return;
  }
  // Use onCreated listener to reliably capture new tabs (avoids race condition with query-diff)
  const newTabIds = new Set();
  const onCreated = (tab) => { newTabIds.add(tab.id); };
  chrome.tabs.onCreated.addListener(onCreated);
  await setDialogSuppressionByScripting(tabId, true);
  try {
    let res;
    try {
      const result = await chrome.scripting.executeScript({
        target: { tabId },
        world: 'MAIN',
        func: async (s) => await eval(s),
        args: [buildPageScript(data.code)]
      });
      res = result[0]?.result;
      if (res === null || res === undefined) {
        console.log('[TMWD-WS] executeScript returned null/undefined, treating as CSP issue');
        res = { ok: false, error: { name: 'Error', message: 'executeScript returned null (possible CSP or context issue)', stack: '' }, csp: true };
      }
    } catch (e) {
      console.log('[TMWD-WS] scripting.executeScript failed:', e.message);
      res = { ok: false, error: { name: e.name || 'Error', message: e.message || String(e), stack: e.stack || '' }, csp: true };
    }
    // CDP fallback for CSP-restricted pages
    if (res && !res.ok && res.csp) {
      console.log('[TMWD-WS] CDP fallback for tab', tabId);
      const wrappedCode = buildCdpScript(data.code);
      try {
        const session = getDebugSession(tabId);
        await ensureDebugAttached(session);
        await setDialogSuppressionByCdp(tabId, true);
        const cdpRes = await chrome.debugger.sendCommand({ tabId }, 'Runtime.evaluate', {
          expression: wrappedCode, awaitPromise: true, returnByValue: true
        });
        await setDialogSuppressionByCdp(tabId, false);
        scheduleDebugDetach(session, 'ws-exec-idle');
        if (cdpRes.exceptionDetails) {
          const desc = cdpRes.exceptionDetails.exception?.description || 'CDP Error';
          res = { ok: false, error: { name: 'Error', message: desc, stack: desc } };
        } else {
          res = cdpRes.result.value;
        }
      } catch (cdpErr) {
        try { await setDialogSuppressionByCdp(tabId, false); } catch (_) {}
        scheduleDebugDetach(getDebugSession(tabId), 'ws-exec-error');
        res = { ok: false, error: { name: 'Error', message: 'CDP fallback failed: ' + cdpErr.message, stack: '' } };
      }
    }
    // Grace period for async tab creation (e.g. link click with target=_blank)
    if (newTabIds.size === 0) await new Promise(r => setTimeout(r, 200));
    chrome.tabs.onCreated.removeListener(onCreated);
    // Get full info for captured new tabs
    const newTabs = [];
    for (const id of newTabIds) {
      try { const t = await chrome.tabs.get(id); newTabs.push({id: t.id, url: t.url, title: t.title}); } catch (_) {}
    }
    if (res?.ok) {
      ws.send(JSON.stringify({ type: 'result', id: data.id, result: res.data ?? null, newTabs }));
    } else {
      console.log(res);
      ws.send(JSON.stringify({ type: 'error', id: data.id, error: res?.error || 'Unknown error', newTabs }));
    }
  } catch (e) {
    ws.send(JSON.stringify({ type: 'error', id: data.id, error: { name: e.name || 'Error', message: e.message || String(e), stack: e.stack || '' } }));
  } finally {
    await setDialogSuppressionByScripting(tabId, false);
    chrome.tabs.onCreated.removeListener(onCreated);
  }
}

async function connectWS() {
  await loadWsPort();
  if (ws && ws.readyState <= 1) return; // CONNECTING or OPEN
  if (!(await isServerAlive())) {
    console.warn('[TMWD-WS] Server not ready, retrying later:', getWsUrl());
    ws = null;
    scheduleProbe();
    return;
  }
  ws = null;
  const wsUrl = getWsUrl();
  console.log('[TMWD-WS] Connecting to', wsUrl);
  try {
    ws = new WebSocket(wsUrl);
  } catch (e) {
    console.error('[TMWD-WS] Constructor error:', e);
    ws = null;
    scheduleProbe();
    return;
  }
  ws.onopen = async () => {
    console.log('[TMWD-WS] Connected!');
    scheduleKeepalive(); // Keep SW alive while connected
    await loadClientIdentity();
    const tabs = await queryScriptableTabs();
    ws.send(JSON.stringify(withClientIdentity({
      type: 'ext_ready',
      extension_version: extensionVersion,
      file_scheme_access: await chrome.extension.isAllowedFileSchemeAccess(),
      tabs: tabs.map(t => ({ id: t.id, url: t.url, title: t.title }))
    })));
    console.log('[TMWD-WS] Sent ext_ready with', tabs.length, 'tabs');
  };
  ws.onmessage = async (event) => {
    try {
      const data = JSON.parse(event.data);
      if (data.id && data.code) {
        lastCommandAt = Date.now();
        let code = data.code;
        // If code is a JSON string representing an object, parse it
        if (typeof code === 'string') {
          try { const p = JSON.parse(code); if (p && typeof p === 'object') code = p; } catch (_) {}
        }
        if (typeof code === 'object' && code !== null && code.cmd) {
          // Custom protocol message → route to handleExtMessage
          if (code.tabId === undefined && data.tabId !== undefined) code.tabId = data.tabId;
          const res = await handleExtMessage(code, {});
          ws.send(JSON.stringify({ type: res.ok ? 'result' : 'error', id: data.id, result: res.data ?? res.results ?? res, error: res.error }));
        } else if (typeof code === 'string') {
          // Plain JS code
          await handleWsExec(data);
        } else if (typeof code === 'object' && code !== null) {
          // Object without cmd → legacy extension message
          const msg = code.tabId === undefined && data.tabId !== undefined ? { ...code, tabId: data.tabId } : code;
          const res = await handleExtMessage(msg, {});
          ws.send(JSON.stringify({ type: res.ok ? 'result' : 'error', id: data.id, result: res.data ?? res.results ?? res, error: res.error }));
        }
      }
    } catch (e) {
      console.error('[TMWD-WS] message parse error', e);
    }
  };
  ws.onclose = async () => {
    console.log('[TMWD-WS] Disconnected');
    ws = null;
    // daemon crash/kill 时 server.rs 无法发送 debugClearAll，扩展必须自行释放 CDP attach。
    await detachAllDebugSessions('ws-close');
    scheduleProbe();
  };
  ws.onerror = (e) => {
    console.warn('[TMWD-WS] Connection warning:', e);
    // onclose will fire after this, which triggers reconnect
  };
}

function reconnectWS() {
  if (ws) {
    try { ws.onclose = null; ws.close(); } catch (_) {}
  }
  ws = null;
  chrome.alarms.clear('tmwd-ws-probe');
  chrome.alarms.clear('tmwd-ws-keepalive');
  connectWS();
}

// Initial connect + wake-up hooks
connectWS();
injectContentScriptsIntoExistingTabs();
chrome.runtime.onStartup.addListener(() => {
  connectWS();
  injectContentScriptsIntoExistingTabs();
});
chrome.runtime.onInstalled.addListener(() => {
  connectWS();
  injectContentScriptsIntoExistingTabs();
});

// Sync tab list on changes
async function sendTabsUpdate() {
  if (!ws || ws.readyState !== WebSocket.OPEN) return;
  const tabs = (await queryScriptableTabs()).filter(t => !/streamlit/i.test(t.title));
  await loadClientIdentity();
  ws.send(JSON.stringify(withClientIdentity({
    type: 'tabs_update',
    extension_version: extensionVersion,
    file_scheme_access: await chrome.extension.isAllowedFileSchemeAccess(),
    tabs: tabs.map(t => ({ id: t.id, url: t.url, title: t.title }))
  })));
}
chrome.tabs.onUpdated.addListener((_, changeInfo) => {
  if (changeInfo.status === 'complete') sendTabsUpdate();
});
chrome.tabs.onRemoved.addListener(() => sendTabsUpdate());
chrome.tabs.onCreated.addListener(() => sendTabsUpdate());
