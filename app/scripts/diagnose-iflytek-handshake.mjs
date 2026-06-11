import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import tls from "node:tls";
import { URL } from "node:url";

const DEFAULT_ENDPOINT = "wss://office-api-ast-dx.iflyaisol.com/ast/communicate/v1";
const AUDIO_ENCODE = "pcm_s16le";
const SAMPLE_RATE = "16000";

const env = {
  ...process.env,
  ...readEnvFile(path.resolve(process.cwd(), ".env")),
  ...readEnvFile(path.resolve(process.cwd(), "..", ".env"))
};

const appId = requiredEnv("IFLYTEK_LLM_APP_ID");
const apiKey = env.IFLYTEK_LLM_API_KEY || env.IFLYTEK_LLM_ACCESS_KEY_ID;
const apiSecret = env.IFLYTEK_LLM_API_SECRET || env.IFLYTEK_LLM_ACCESS_KEY_SECRET;

if (!apiKey) {
  fail("Missing IFLYTEK_LLM_API_KEY.");
}

if (!apiSecret) {
  fail("Missing IFLYTEK_LLM_API_SECRET.");
}

const endpoint = env.IFLYTEK_LLM_ENDPOINT || DEFAULT_ENDPOINT;
const lang = env.IFLYTEK_LLM_LANG || "autodialect";
const roleType = env.IFLYTEK_LLM_ROLE_TYPE || "2";
const featureIds = env.IFLYTEK_LLM_FEATURE_IDS;
const requestId = `diagnostic-${Date.now()}`;
const utc = formatIflytekUtc(new Date());

const signedUrl = signWebSocketUrl({
  endpoint,
  appId,
  apiKey,
  apiSecret,
  lang,
  roleType,
  featureIds,
  requestId,
  utc
});

const url = new URL(signedUrl);
const requestPath = `${url.pathname}${url.search}`;
const key = crypto.randomBytes(16).toString("base64");
const hostHeader = url.port ? `${url.hostname}:${url.port}` : url.hostname;
const request = [
  `GET ${requestPath} HTTP/1.1`,
  `Host: ${hostHeader}`,
  "Connection: Upgrade",
  "Upgrade: websocket",
  "Sec-WebSocket-Version: 13",
  `Sec-WebSocket-Key: ${key}`,
  "User-Agent: voicecoder-iflytek-diagnostic",
  "",
  ""
].join("\r\n");

console.log(`[iflytek diagnostic] url=${redactSignedUrl(signedUrl)}`);
console.log(`[iflytek diagnostic] utc=${utc}`);

const socket = tls.connect(
  {
    host: url.hostname,
    port: Number(url.port || 443),
    servername: url.hostname,
    ALPNProtocols: ["http/1.1"],
    timeout: 10_000
  },
  () => {
    socket.write(request);
  }
);

const chunks = [];

socket.on("data", (chunk) => {
  chunks.push(chunk);
  const response = Buffer.concat(chunks);
  if (response.length >= 512 || response.includes("\r\n\r\n")) {
    printResponse(response);
    socket.end();
  }
});

socket.on("timeout", () => {
  console.error("[iflytek diagnostic] timed out waiting for handshake response.");
  socket.destroy();
  process.exitCode = 1;
});

socket.on("error", (error) => {
  console.error(`[iflytek diagnostic] socket error: ${error.message}`);
  process.exitCode = 1;
});

socket.on("end", () => {
  if (!chunks.length) {
    console.error("[iflytek diagnostic] connection ended without a response.");
    process.exitCode = 1;
  }
});

function printResponse(response) {
  const preview = response.subarray(0, 512);
  const firstLine = preview.toString("latin1").split(/\r?\n/, 1)[0];
  console.log(`[iflytek diagnostic] first-line=${escapeControl(firstLine)}`);
  console.log(`[iflytek diagnostic] preview=${escapeControl(preview.toString("latin1"))}`);
  console.log(`[iflytek diagnostic] hex=${preview.toString("hex").match(/.{1,2}/g)?.join(" ") || ""}`);
}

function signWebSocketUrl({ endpoint, appId, apiKey, apiSecret, lang, roleType, featureIds, requestId, utc }) {
  const params = new Map([
    ["accessKeyId", apiKey],
    ["appId", appId],
    ["audio_encode", AUDIO_ENCODE],
    ["lang", lang],
    ["role_type", roleType],
    ["samplerate", SAMPLE_RATE],
    ["utc", utc],
    ["uuid", requestId]
  ]);

  if (featureIds?.trim()) {
    params.set("feature_ids", featureIds);
  }

  const sortedParams = [...params.entries()].sort(([left], [right]) => left.localeCompare(right));
  const baseString = sortedParams.map(([key, value]) => `${strictEncode(key)}=${strictEncode(value)}`).join("&");
  const signature = crypto.createHmac("sha1", apiSecret).update(baseString).digest("base64");
  const query = [...sortedParams, ["signature", signature]]
    .map(([key, value]) => `${strictEncode(key)}=${strictEncode(value)}`)
    .join("&");

  return `${endpoint}?${query}`;
}

function redactSignedUrl(value) {
  const parsed = new URL(value);
  for (const key of ["accessKeyId", "signature"]) {
    if (parsed.searchParams.has(key)) {
      parsed.searchParams.set(key, "<redacted>");
    }
  }
  return parsed.toString();
}

function readEnvFile(envPath) {
  if (!fs.existsSync(envPath)) {
    return {};
  }

  const values = {};
  for (const line of fs.readFileSync(envPath, "utf8").split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) {
      continue;
    }

    const separatorIndex = trimmed.indexOf("=");
    if (separatorIndex < 0) {
      continue;
    }

    const key = trimmed.slice(0, separatorIndex).trim();
    const value = trimmed
      .slice(separatorIndex + 1)
      .trim()
      .replace(/^['"]|['"]$/g, "");
    values[key] = value;
  }
  return values;
}

function requiredEnv(key) {
  const value = env[key];
  if (!value) {
    fail(`Missing ${key}.`);
  }
  return value;
}

function strictEncode(value) {
  return encodeURIComponent(value).replace(/[!'()*]/g, (character) =>
    `%${character.charCodeAt(0).toString(16).toUpperCase()}`
  );
}

function formatIflytekUtc(date) {
  const offsetMinutes = -date.getTimezoneOffset();
  const sign = offsetMinutes >= 0 ? "+" : "-";
  const absoluteOffsetMinutes = Math.abs(offsetMinutes);
  const offsetHours = String(Math.floor(absoluteOffsetMinutes / 60)).padStart(2, "0");
  const offsetRemainderMinutes = String(absoluteOffsetMinutes % 60).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}${sign}${offsetHours}${offsetRemainderMinutes}`;
}

function pad(value) {
  return String(value).padStart(2, "0");
}

function escapeControl(value) {
  return value.replace(/[^\x20-\x7e]/g, (character) => {
    const code = character.charCodeAt(0);
    if (character === "\r") {
      return "\\r";
    }
    if (character === "\n") {
      return "\\n";
    }
    return `\\x${code.toString(16).padStart(2, "0")}`;
  });
}

function fail(message) {
  console.error(`[iflytek diagnostic] ${message}`);
  process.exit(1);
}
