// SPDX-License-Identifier: AGPL-3.0-or-later
// Layer-2 VS Code / Cursor / Windsurf / Trae / VSCodium adapter.
// Emits allowlisted metadata into ~/.aitime/agent_events.jsonl (or STRUGGLETIME_HOME).
// Never paths, prompts, file contents, argv, or API keys.
const vscode = require("vscode");
const fs = require("fs");
const path = require("path");
const {
  buildRow,
  editorLabelFromAppName,
  eventsPath: resolveEventsPath,
  readSettingsSafe,
  runtimeHome,
  scrub,
  FORBIDDEN,
} = require("./lib");

/** @type {vscode.OutputChannel | null} */
let channel = null;
let emitCount = 0;
let lastEmitAt = 0;
let focused = true;
let heartbeatTimer = null;
let editTimer = null;

function debugEnabled() {
  const cfg = (() => {
    try { return vscode.workspace.getConfiguration("struggletime").get("debug"); } catch (_) { return false; }
  })();
  if (cfg === true) return true;
  return /^(1|true|yes|on)$/i.test(String(process.env.STRUGGLETIME_DEBUG || process.env.AITIME_DEBUG || ""));
}

function log(line) {
  const msg = typeof line === "string" ? line : JSON.stringify(line);
  if (channel) channel.appendLine(`[${new Date().toISOString()}] ${msg}`);
  if (debugEnabled()) console.error("[struggletime]", msg);
}

function eventsPath() {
  let override = "";
  try {
    override = vscode.workspace.getConfiguration("struggletime").get("eventsPath") || "";
  } catch (_) { /* tests / headless */ }
  return resolveEventsPath(process.env, override);
}

function editorLabel() {
  return editorLabelFromAppName(vscode.env.appName || "");
}

function emit(payload) {
  const dest = eventsPath();
  fs.mkdirSync(path.dirname(dest), { recursive: true });
  const row = buildRow(payload, editorLabel());
  fs.appendFileSync(dest, JSON.stringify(row) + "\n");
  emitCount += 1;
  lastEmitAt = Date.now();
  if (debugEnabled()) {
    log({
      msg: "emit",
      editor: row.editor,
      event_type: row.event_type,
      files_touched_count: row.files_touched_count,
      has_project_id: !!row.project_id,
      emits: emitCount,
    });
  }
}

function showStatus() {
  const settings = readSettingsSafe();
  const dest = eventsPath();
  let bytes = 0;
  let lines = 0;
  try {
    const st = fs.statSync(dest);
    bytes = st.size;
    lines = fs.readFileSync(dest, "utf8").split("\n").filter(Boolean).length;
  } catch (_) { /* missing ok */ }
  const ed = editorLabel();
  const linesOut = [
    `StruggleTime adapter (${ed})`,
    `events file: ${dest}`,
    `queued lines: ${lines} (${bytes} bytes)`,
    `emits this session: ${emitCount}`,
    `last emit: ${lastEmitAt ? new Date(lastEmitAt).toISOString() : "(none yet)"}`,
    `settings server: ${settings.server || "(default https://struggletime.screenseekr.com)"}`,
    `api key in settings: ${settings.hasToken ? "set" : "MISSING — run: struggletime apikey <st_…>"}`,
    `linked projects: ${settings.projectCount}`,
    `daemon project id: ${settings.project || "(unset — events map to first watched project)"}`,
    "",
    "Heartbeats go: Cursor adapter → agent_events.jsonl → local daemon →",
    "  POST /v1/buckets/batch (default https://struggletime.screenseekr.com).",
    "Ensure: struggletime start + linked project + API key.",
    "Debug: struggletime.debug=true or STRUGGLETIME_DEBUG=1 → this Output panel.",
  ];
  if (!channel) channel = vscode.window.createOutputChannel("StruggleTime");
  channel.clear();
  for (const l of linesOut) channel.appendLine(l);
  channel.show(true);
  vscode.window.showInformationMessage(
    `StruggleTime: ${emitCount} emits this session; ${lines} queued — see Output → StruggleTime`
  );
}

function activate(ctx) {
  channel = vscode.window.createOutputChannel("StruggleTime");
  ctx.subscriptions.push(channel);
  const ed = editorLabel();
  log({ msg: "boot", editor: ed, events: eventsPath() });

  ctx.subscriptions.push(
    vscode.commands.registerCommand("struggletime.status", () => showStatus())
  );

  const scheduleEdit = () => {
    clearTimeout(editTimer);
    editTimer = setTimeout(() => {
      emit({ event_type: "edit", verb: "modified", op: "modified", editor: ed, agent: "human", files_touched_count: 1 });
    }, 1500);
  };

  ctx.subscriptions.push(vscode.workspace.onDidChangeTextDocument((e) => {
    if (!e.contentChanges || e.contentChanges.length === 0) return;
    scheduleEdit();
  }));

  ctx.subscriptions.push(vscode.workspace.onDidSaveTextDocument(() => {
    emit({ event_type: "save", verb: "save", op: "modified", editor: ed, agent: "human", files_touched_count: 1 });
  }));

  ctx.subscriptions.push(vscode.window.onDidChangeActiveTextEditor((editor) => {
    if (!editor) return;
    emit({ event_type: "focus", verb: "focus", editor: ed, agent: "human", files_touched_count: 0 });
  }));

  ctx.subscriptions.push(vscode.workspace.onDidCreateFiles((e) => {
    emit({
      event_type: "created", verb: "created", op: "created", editor: ed, agent: "human",
      files_touched_count: (e.files && e.files.length) || 1,
    });
  }));

  ctx.subscriptions.push(vscode.workspace.onDidDeleteFiles((e) => {
    emit({
      event_type: "deleted", verb: "deleted", op: "deleted", editor: ed, agent: "human",
      files_touched_count: (e.files && e.files.length) || 1,
    });
  }));

  ctx.subscriptions.push(vscode.window.onDidChangeWindowState((s) => {
    focused = !!s.focused;
    emit({
      event_type: focused ? "focus" : "blur",
      verb: focused ? "focus" : "blur",
      editor: ed,
      agent: "human",
      files_touched_count: 0,
    });
  }));

  heartbeatTimer = setInterval(() => {
    if (!focused) return;
    emit({
      event_type: "heartbeat",
      category: "editor",
      verb: "heartbeat",
      editor: ed,
      agent: "human",
      files_touched_count: 0,
    });
  }, 45_000);
  ctx.subscriptions.push({ dispose: () => clearInterval(heartbeatTimer) });

  emit({
    event_type: "heartbeat",
    category: "editor",
    verb: "boot",
    editor: ed,
    agent: "human",
    files_touched_count: 0,
  });
}

function deactivate() {
  if (heartbeatTimer) clearInterval(heartbeatTimer);
  if (editTimer) clearTimeout(editTimer);
}

module.exports = {
  activate,
  deactivate,
  editorLabel,
  emit,
  buildRow,
  eventsPath,
  runtimeHome,
  scrub,
  FORBIDDEN,
};
