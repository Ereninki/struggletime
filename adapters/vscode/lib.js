// Pure helpers shared by extension.js and unit tests (no vscode dependency).
const fs = require("fs");
const os = require("os");
const path = require("path");

const FORBIDDEN = new Set([
  "content", "text", "code", "prompt", "prompt_text", "message", "command",
  "args", "argv", "keystrokes", "screenshot", "image", "path", "filepath",
  "file_path", "raw", "transcript", "cwd", "workspace", "body", "diff", "patch",
  "token", "api_key", "apikey", "authorization",
]);

function configHome(env = process.env) {
  if (env.STRUGGLETIME_CONFIG || env.AITIME_CONFIG) {
    return env.STRUGGLETIME_CONFIG || env.AITIME_CONFIG;
  }
  if (process.platform === "win32" && env.APPDATA) {
    return path.join(env.APPDATA, "struggletime");
  }
  return path.join(os.homedir(), ".config", "struggletime");
}

function runtimeHome(env = process.env) {
  if (env.STRUGGLETIME_HOME || env.AITIME_HOME) {
    return env.STRUGGLETIME_HOME || env.AITIME_HOME;
  }
  return path.join(os.homedir(), ".aitime");
}

function eventsPath(env = process.env, override = "") {
  if (override && String(override).trim()) return String(override).trim();
  if (env.AITIME_AGENT_EVENTS) return env.AITIME_AGENT_EVENTS;
  if (env.AITIME_EXEC_LOG) return env.AITIME_EXEC_LOG;
  return path.join(runtimeHome(env), "agent_events.jsonl");
}

function readSettingsSafe(env = process.env) {
  const p = path.join(configHome(env), "settings.json");
  try {
    const raw = fs.readFileSync(p, "utf8");
    const j = JSON.parse(raw);
    return {
      server: typeof j.server === "string" ? j.server : "",
      handle: typeof j.handle === "string" ? j.handle : "",
      project: typeof j.project === "string" ? j.project : "",
      hasToken: !!(j.token && String(j.token).trim()),
      projectCount: Array.isArray(j.projects) ? j.projects.length : 0,
    };
  } catch (_) {
    return { server: "", handle: "", project: "", hasToken: false, projectCount: 0 };
  }
}

function scrub(obj) {
  if (!obj || typeof obj !== "object") return {};
  const out = {};
  for (const [k, v] of Object.entries(obj)) {
    if (FORBIDDEN.has(k)) continue;
    if (v && typeof v === "object" && !Array.isArray(v)) {
      out[k] = scrub(v);
    } else if (typeof v === "string" || typeof v === "number" || typeof v === "boolean" || v === null) {
      out[k] = v;
    }
  }
  return out;
}

function editorLabelFromAppName(appName) {
  const name = String(appName || "").toLowerCase();
  if (name.includes("cursor")) return "cursor";
  if (name.includes("windsurf")) return "windsurf";
  if (name.includes("trae")) return "trae";
  if (name.includes("kiro")) return "kiro";
  if (name.includes("vscodium") || name.includes("codium") || name.includes("code - oss")) return "vscodium";
  if (name.includes("visual studio code") || name.includes("code")) return "vscode";
  return "vscode";
}

function buildRow(payload, editor, env = process.env) {
  const settings = readSettingsSafe(env);
  const row = scrub({
    ts: Math.floor(Date.now() / 1000),
    event_type: payload.event_type || payload.op || "edit",
    category: payload.category || "editor",
    exit_code: 0,
    editor: editor || "vscode",
    agent: payload.agent || "human",
    files_touched_count: payload.files_touched_count != null ? payload.files_touched_count : 1,
    source: "adapter",
    ...payload,
  });
  if (!row.project_id && settings.project && !/[\\/]/.test(settings.project)) {
    row.project_id = settings.project;
  }
  delete row.path; delete row.content; delete row.text; delete row.prompt_text;
  delete row.token; delete row.api_key; delete row.authorization;
  return row;
}

module.exports = {
  FORBIDDEN,
  configHome,
  runtimeHome,
  eventsPath,
  readSettingsSafe,
  scrub,
  editorLabelFromAppName,
  buildRow,
};
