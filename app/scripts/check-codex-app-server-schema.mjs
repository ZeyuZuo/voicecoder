import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const baselineVersion = "codex-cli 0.144.1";
const generatedRoot = resolve("src-tauri/generated/codex-app-server");
const jsonRoot = resolve(generatedRoot, "json");
const fixtureRoot = resolve("src-tauri/tests/fixtures/codex-app-server-v2");
const failures = [];

const manifest = readJson(resolve(generatedRoot, "manifest.json"));
if (manifest.version !== baselineVersion) {
  failures.push(`manifest version is ${JSON.stringify(manifest.version)}, expected ${baselineVersion}`);
}
if (manifest.transport !== "stdio") {
  failures.push(`manifest transport is ${JSON.stringify(manifest.transport)}, expected stdio`);
}

const schemaRequirements = [
  ["v2/ThreadStartParams.json", ["cwd", "approvalPolicy", "approvalsReviewer", "sandbox", "threadSource"]],
  ["v2/TurnStartParams.json", ["threadId", "cwd", "input", "sandboxPolicy", "approvalPolicy", "approvalsReviewer"]],
  ["v2/ThreadResumeParams.json", ["threadId", "cwd", "sandbox", "approvalPolicy", "approvalsReviewer"]],
  ["v2/ItemStartedNotification.json", ["threadId", "turnId", "item", "startedAtMs"]],
  ["v2/ItemCompletedNotification.json", ["threadId", "turnId", "item", "completedAtMs"]],
  ["v2/CommandExecutionOutputDeltaNotification.json", ["threadId", "turnId", "itemId", "delta"]],
  ["v2/FileChangeOutputDeltaNotification.json", ["threadId", "turnId", "itemId", "delta"]],
  ["v2/FileChangePatchUpdatedNotification.json", ["threadId", "turnId", "itemId", "changes"]],
  ["v2/TurnPlanUpdatedNotification.json", ["threadId", "turnId", "plan", "explanation"]],
  ["v2/ThreadTokenUsageUpdatedNotification.json", ["threadId", "turnId", "tokenUsage"]],
  ["v2/TurnCompletedNotification.json", ["threadId", "turn"]],
  ["v2/ServerRequestResolvedNotification.json", ["requestId", "threadId"]],
  ["CommandExecutionRequestApprovalParams.json", ["threadId", "turnId", "itemId", "command", "cwd"]],
  ["FileChangeRequestApprovalParams.json", ["threadId", "turnId", "itemId"]],
  ["PermissionsRequestApprovalParams.json", ["threadId", "turnId", "itemId", "permissions"]],
  ["ToolRequestUserInputParams.json", ["threadId", "turnId", "itemId", "questions"]],
  ["McpServerElicitationRequestParams.json", ["threadId", "turnId", "serverName"]]
];

for (const [relativePath, properties] of schemaRequirements) {
  const schema = readJson(resolve(jsonRoot, relativePath));
  requireProperties(relativePath, schema, properties);
}

const threadStartSchema = readJson(resolve(jsonRoot, "v2/ThreadStartParams.json"));
requireEnumValue("ThreadStartParams.ApprovalsReviewer", threadStartSchema.definitions?.ApprovalsReviewer, "auto_review");
requireEnumValue("ThreadStartParams.AskForApproval", threadStartSchema.definitions?.AskForApproval, "on-request");

const fixtureSchemas = [
  ["thread-start-request.json", "v2/ThreadStartParams.json"],
  ["turn-start-request.json", "v2/TurnStartParams.json"],
  ["thread-resume-request.json", "v2/ThreadResumeParams.json"],
  ["command-approval-request.json", "CommandExecutionRequestApprovalParams.json"],
  ["file-change-approval-request.json", "FileChangeRequestApprovalParams.json"],
  ["permissions-approval-request.json", "PermissionsRequestApprovalParams.json"],
  ["tool-user-input-request.json", "ToolRequestUserInputParams.json"],
  ["mcp-elicitation-request.json", "McpServerElicitationRequestParams.json"]
];

for (const [fixtureName, schemaPath] of fixtureSchemas) {
  const fixture = readJson(resolve(fixtureRoot, fixtureName));
  const schema = readJson(resolve(jsonRoot, schemaPath));
  const knownProperties = collectProperties(schema);
  for (const property of Object.keys(fixture.params ?? {})) {
    if (!knownProperties.has(property)) {
      failures.push(`${fixtureName} uses params.${property}, absent from ${schemaPath}`);
    }
  }
}

if (failures.length) {
  throw new Error(`Codex app-server schema compatibility check failed:\n- ${failures.join("\n- ")}`);
}

console.log(`Codex app-server schema compatibility verified for ${baselineVersion}.`);

function readJson(path) {
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch (error) {
    throw new Error(`Cannot read generated schema artifact ${path}: ${error.message}`);
  }
}

function requireProperties(name, schema, properties) {
  const actual = collectProperties(schema);
  for (const property of properties) {
    if (!actual.has(property)) {
      failures.push(`${name} is missing property ${property}`);
    }
  }
}

function collectProperties(schema) {
  const properties = new Set(Object.keys(schema?.properties ?? {}));
  for (const branchName of ["allOf", "anyOf", "oneOf"]) {
    for (const branch of schema?.[branchName] ?? []) {
      for (const property of collectProperties(branch)) {
        properties.add(property);
      }
    }
  }
  return properties;
}

function requireEnumValue(name, schema, value) {
  if (!JSON.stringify(schema ?? {}).includes(`\"${value}\"`)) {
    failures.push(`${name} no longer includes ${value}`);
  }
}
