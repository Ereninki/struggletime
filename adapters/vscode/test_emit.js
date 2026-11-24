#!/usr/bin/env node
const assert = require("assert");
const path = require("path");
const os = require("os");
const fs = require("fs");
const lib = require("./lib");

assert.strictEqual(lib.editorLabelFromAppName("Cursor"), "cursor");
assert.strictEqual(lib.editorLabelFromAppName("Visual Studio Code"), "vscode");

const row = lib.buildRow({
  event_type: "heartbeat",
  files_touched_count: 0,
  path: "/tmp/secret.rs",
  prompt_text: "leak",
  token: "st_should_never_appear",
}, "cursor");

assert.strictEqual(row.event_type, "heartbeat");
assert.strictEqual(row.editor, "cursor");
assert.strictEqual(row.source, "adapter");
assert.strictEqual(row.path, undefined);
assert.strictEqual(row.prompt_text, undefined);
assert.strictEqual(row.token, undefined);
assert.ok(!JSON.stringify(row).includes("secret"));
assert.ok(!JSON.stringify(row).includes("st_should"));

const scrubbed = lib.scrub({ path: "/x", event_type: "edit", nested: { argv: ["rm"] } });
assert.strictEqual(scrubbed.path, undefined);
assert.deepStrictEqual(scrubbed.nested, {});

const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "st-adapter-"));
const env = { STRUGGLETIME_HOME: tmp, AITIME_HOME: tmp };
assert.strictEqual(lib.runtimeHome(env), tmp);
assert.strictEqual(lib.eventsPath(env), path.join(tmp, "agent_events.jsonl"));

const cfg = path.join(tmp, "cfg");
fs.mkdirSync(cfg, { recursive: true });
fs.writeFileSync(path.join(cfg, "settings.json"), JSON.stringify({
  project: "my-proj",
  server: "https://struggletime.screenseekr.com",
  token: "st_test_only_local",
}));
const withProj = lib.buildRow({ event_type: "edit" }, "cursor", {
  STRUGGLETIME_CONFIG: cfg,
});
assert.strictEqual(withProj.project_id, "my-proj");
assert.ok(!JSON.stringify(withProj).includes("st_test"));

console.log("ok  vscode/cursor adapter emit + privacy scrub");
